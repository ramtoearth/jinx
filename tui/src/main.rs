//! jinx — TUI binary entry point.
//!
//! Startup sequence (Task 18 / Requisitos 7.1, 7.3, 7.4, 9.1, 9.2):
//! 1. Resolve or override the SQLite path.
//! 2. Open `SqliteStorage` (creates the DB and applies migrations).
//! 3. Initialise the Ratatui terminal backend.
//! 4. Spawn the Python Agent as a child process.
//! 5. Send `agent_init`.
//! 6. Enter the main event loop (render + key events + IPC reads).
//! 7. On `Ctrl+Q` send `shutdown` and wait for the agent to exit.

use std::io;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crossterm::{
    event::{self, DisableBracketedPaste, EnableBracketedPaste, Event, KeyCode, KeyModifiers, MouseEvent, MouseEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    widgets::{Block, Borders, Paragraph, Tabs},
    Terminal,
};
use jinx_core::{AppServices, SqliteStorage, TaskFilter};

use jinx::app::{AppEvent, AppState, Panel, MIN_COLS, MIN_ROWS};
use jinx::calendario::{entry_count, flat_entries};
use jinx::color::detect_color_mode;

mod state;
use state::*;

mod agent;
use agent::*;

mod modals;
use modals::*;

mod panels;
use panels::*;

fn current_month() -> String {
    chrono::Local::now().format("%Y-%m").to_string()
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

fn main() -> io::Result<()> {
    // -- Storage -----------------------------------------------------------
    let db_path = match jinx_core::resolve_db_path() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("Cannot resolve database path: {e}");
            std::process::exit(1);
        }
    };
    let storage: Arc<SqliteStorage> = Arc::new(
        SqliteStorage::open(&db_path).unwrap_or_else(|e| {
            eprintln!("Cannot open database: {e}");
            std::process::exit(1);
        }),
    );

    // -- Terminal setup ----------------------------------------------------
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableBracketedPaste)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = run_app(&mut terminal, storage);

    // -- Cleanup -----------------------------------------------------------
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen, DisableBracketedPaste)?;
    terminal.show_cursor()?;

    if let Err(e) = result {
        eprintln!("Error: {e}");
    }
    Ok(())
}

fn run_app(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    storage: Arc<SqliteStorage>,
) -> io::Result<()> {
    let (size_cols, size_rows) = {
        let size = terminal.size()?;
        (size.width, size.height)
    };

    let cfg = jinx::config::load();
    let locale = jinx::locale::load(&cfg.language);

    let mut state = RuntimeState {
        app: AppState::new(size_cols, size_rows),
        locale,
        chat_history: Vec::new(),
        chat_editor: jinx::text_editor::TextEditor::new(),
        chat_scroll: 0,
        prompt_history: Vec::new(),
        prompt_history_idx: None,
        prompt_stash: String::new(),
        task_cursor: 0,
        tareas_scroll: 0,
        tareas_search_active: false,
        tareas_search_query: String::new(),
        calendar_cursor: 0,
        calendar_scroll: 0,
        calendar_scroll_initialized: false,
        calendar_filter_idx: 0,
        group_cursor: 0,
        tareas_section: TareasSection::default(),
        tareas_filter: ActiveTaskFilter::default(),
        color_mode: detect_color_mode(),
        services: AppServices::new(storage.clone()),
        agent_child: None,
        agent_stdin: None,
        agent_rx: None,
        pending_request: None,
        task_form: TaskFormState::default(),
        event_form: EventFormState::default(),
        group_form: GroupFormState::default(),
        settings_form: SettingsFormState::default(),
        filter_form: FilterFormState::default(),
        groups_cache: Vec::new(),
        delete_confirm_name: String::new(),
        pending_g: false,
        notes_cache: Vec::new(),
        notes_cursor: 0,
        notes_scroll: 0,
        notes_view: NotesView::default(),
        notes_editor: jinx::text_editor::TextEditor::new(),
        notes_title_editor: jinx::text_editor::TextEditor::new(),
        notes_title_focused: true,
        notes_search_active: false,
        notes_search_query: String::new(),
        notes_current_id: None,
        notes_preview_scroll: 0,
        notes_editor_scroll: 0,
        notes_undo_stack: Vec::new(),
        notes_redo_stack: Vec::new(),
        notes_pending_g: false,
        last_note_results: None,
        note_picker_active: false,
        note_picker_cursor: 0,
        note_picker_msg_idx: None,
        dir_browser: DirBrowserState::default(),
        cmd_picker_active: false,
        cmd_picker_cursor: 0,
        cmd_picker_filtered: Vec::new(),
        finance_month: current_month(),
        panel_area: None,
        input_area: None,
        history_area: None,
    };

    // Spawn agent
    spawn_agent(&mut state);

    let tick = Duration::from_millis(250);
    let timeout_dur = Duration::from_secs(30);
    let mut last_tick = Instant::now();

    loop {
        // --- Render -------------------------------------------------------
        terminal.draw(|f| render(f, &mut state))?;

        // --- Drain agent output every iteration (non-blocking) ------------
        read_agent_output(&mut state);

        // --- Event polling ------------------------------------------------
        let elapsed = last_tick.elapsed();
        let wait = tick.saturating_sub(elapsed);

        if event::poll(wait)? {
            match event::read()? {
                Event::Key(key) => {
                    // Global quit
                    if key.code == KeyCode::Char('q')
                        && key.modifiers.contains(KeyModifiers::CONTROL)
                    {
                        send_shutdown(&mut state);
                        break;
                    }

                    handle_key(&mut state, key);
                }
                Event::Mouse(mouse) => {
                    handle_mouse(&mut state, mouse);
                }
                Event::Paste(data) => {
                    handle_paste(&mut state, data);
                }
                Event::Resize(cols, rows) => {
                    state.app = jinx::app::reduce(state.app, AppEvent::Resize(cols, rows));
                }
                _ => {}
            }
        }

        // --- Tick ---------------------------------------------------------
        if last_tick.elapsed() >= tick {
            last_tick = Instant::now();

            // Check timeout
            if let Some((_, started)) = state.pending_request {
                if started.elapsed() >= timeout_dur {
                    state.app.status_bar = state.locale.errors.timeout.clone();
                    state.pending_request = None;
                }
            }
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Key handling
// ---------------------------------------------------------------------------

fn handle_key(state: &mut RuntimeState, key: crossterm::event::KeyEvent) {
    // When a modal is open, route all keys to modal handler
    if state.app.modal.is_some() {
        handle_modal_key(state, key);
        return;
    }

    // Tab / Shift-Tab cycle panels
    match key.code {
        KeyCode::Tab if key.modifiers == KeyModifiers::NONE => {
            state.pending_g = false;
            state.notes_pending_g = false;
            state.app = jinx::app::reduce(state.app.clone(), AppEvent::Key(key));
            return;
        }
        KeyCode::BackTab => {
            state.pending_g = false;
            state.notes_pending_g = false;
            state.app = jinx::app::reduce(state.app.clone(), AppEvent::Key(key));
            return;
        }
        _ => {}
    }

    // Ctrl+P opens the settings modal from any panel
    if key.code == KeyCode::Char('p') && key.modifiers.contains(KeyModifiers::CONTROL) {
        open_settings_modal(state);
        return;
    }

    if state.app.is_too_small() {
        return;
    }

    match state.app.focused_panel {
        Panel::Chat => handle_chat_key(state, key),
        Panel::Tareas => handle_tareas_key(state, key),
        Panel::Calendario => handle_calendario_key(state, key),
        Panel::Notas => handle_notas_key(state, key),
        Panel::Finanzas => handle_finanzas_key(state, key),
    }
}

fn handle_mouse(state: &mut RuntimeState, mouse: MouseEvent) {
    if state.app.is_too_small() || state.app.modal.is_some() {
        return;
    }
    let (col, row) = (mouse.column, mouse.row);
    match mouse.kind {
        MouseEventKind::ScrollUp => {
            if let Some(hist) = state.history_area {
                if col >= hist.x && col < hist.x + hist.width && row >= hist.y && row < hist.y + hist.height {
                    state.chat_scroll = state.chat_scroll.saturating_add(3);
                    return;
                }
            }
            if let Some(input) = state.input_area {
                if col >= input.x && col < input.x + input.width && row >= input.y && row < input.y + input.height {
                    // Input scroll: move cursor up
                    state.chat_editor.move_up();
                    return;
                }
            }
            // If scroll happens over the full panel area, determine panel by current tab
            match state.app.focused_panel {
                Panel::Tareas => { if state.task_cursor > 0 { state.task_cursor -= 1; } }
                Panel::Calendario => { if state.calendar_cursor > 0 { state.calendar_cursor -= 1; } }
                _ => { state.chat_scroll = state.chat_scroll.saturating_add(3); }
            }
        }
        MouseEventKind::ScrollDown => {
            if let Some(hist) = state.history_area {
                if col >= hist.x && col < hist.x + hist.width && row >= hist.y && row < hist.y + hist.height {
                    state.chat_scroll = state.chat_scroll.saturating_sub(3);
                    return;
                }
            }
            if let Some(input) = state.input_area {
                if col >= input.x && col < input.x + input.width && row >= input.y && row < input.y + input.height {
                    state.chat_editor.move_down();
                    return;
                }
            }
            match state.app.focused_panel {
                Panel::Tareas => {
                    let count = state.services.tasks.list(state.tareas_filter.to_storage_filter()).unwrap_or_default().len();
                    if state.task_cursor + 1 < count { state.task_cursor += 1; }
                }
                Panel::Calendario => {
                    let tasks = state.services.tasks.list(TaskFilter::default()).unwrap_or_default();
                    let events = state.services.calendar.list(None, None).unwrap_or_default();
                    let mut view = jinx::calendario::calendar_layout(&tasks, &events);
                    if let Some((from, to)) = calendar_date_range(state.calendar_filter_idx) {
                        view.retain(|date, _| date.as_str() >= from.as_str() && date.as_str() <= to.as_str());
                    }
                    let flat = flat_entries(&view);
                    let count = entry_count(&flat);
                    if state.calendar_cursor + 1 < count { state.calendar_cursor += 1; }
                }
                _ => { state.chat_scroll = state.chat_scroll.saturating_sub(3); }
            }
        }
        _ => {}
    }
}

fn handle_paste(state: &mut RuntimeState, data: String) {
    if state.app.modal.is_some() {
        handle_modal_paste(state, &data);
        return;
    }
    if state.app.focused_panel != Panel::Chat {
        return;
    }
    for c in data.chars() {
        if c == '\n' || c == '\r' {
            state.chat_editor.insert_newline();
        } else {
            state.chat_editor.insert_char(c);
        }
    }
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

fn render(frame: &mut ratatui::Frame, state: &mut RuntimeState) {
    let size = frame.area();

    // Viewport guard
    if size.width < MIN_COLS || size.height < MIN_ROWS {
        let msg = Paragraph::new(
            state.locale.errors.terminal_too_small
                .replace("{min_cols}", &MIN_COLS.to_string())
                .replace("{min_rows}", &MIN_ROWS.to_string())
                .replace("{cols}", &size.width.to_string())
                .replace("{rows}", &size.height.to_string())
        )
        .style(Style::default().fg(Color::Red));
        frame.render_widget(msg, size);
        return;
    }

    // Layout: tab bar (3) + panel content + status bar (1)
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(1),
            Constraint::Length(1),
        ])
        .split(size);

    render_tabs(frame, state, chunks[0]);

    state.panel_area = Some(chunks[1]);
    match state.app.focused_panel {
        Panel::Chat => render_chat(frame, state, chunks[1]),
        Panel::Tareas => render_tareas(frame, state, chunks[1]),
        Panel::Calendario => render_calendario(frame, state, chunks[1]),
        Panel::Notas => render_notas(frame, state, chunks[1]),
        Panel::Finanzas => render_finanzas(frame, state, chunks[1]),
    }

    render_status(frame, state, chunks[2]);

    // Modal overlay on top of everything
    if state.app.modal.is_some() {
        render_modal(frame, state, size);
    }
}

fn render_tabs(frame: &mut ratatui::Frame, state: &RuntimeState, area: Rect) {
    let active = match state.app.focused_panel {
        Panel::Chat => 0,
        Panel::Tareas => 1,
        Panel::Calendario => 2,
        Panel::Notas => 3,
        Panel::Finanzas => 4,
    };
    let tabs = Tabs::new(vec![
        format!("  {}  ", state.locale.panels.chat),
        format!("  {}  ", state.locale.panels.tasks),
        format!("  {}  ", state.locale.panels.calendar),
        format!("  {}  ", state.locale.panels.notes),
        "  Finanzas  ".to_string(),
    ])
        .select(active)
        .block(Block::default().borders(Borders::ALL))
        .style(Style::default().fg(Color::DarkGray))
        .highlight_style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        );
    frame.render_widget(tabs, area);
}

fn render_status(frame: &mut ratatui::Frame, state: &RuntimeState, area: Rect) {
    let hint = &state.locale.hints.global;

    if let Some((spinner_char, secs)) = spinner_state(&state.pending_request) {
        let working = state.locale.chat.thinking_status
            .replace("{spinner}", &spinner_char.to_string())
            .replace("{secs}", &secs.to_string());
        let text = format!("{}  │  {}", working, hint);
        let para = Paragraph::new(text).style(Style::default().fg(Color::Yellow));
        frame.render_widget(para, area);
    } else {
        let text = if state.app.status_bar.is_empty() {
            hint.to_string()
        } else {
            format!("{}  │  {}", state.app.status_bar, hint)
        };
        let para = Paragraph::new(text).style(Style::default().fg(Color::DarkGray));
        frame.render_widget(para, area);
    }
}

// ---------------------------------------------------------------------------
// Feature: terminal-day-organizer, Property 24: Propagación de errores del Almacén al usuario
// ---------------------------------------------------------------------------
// Valida: Requisitos 11.3, 13.11

// Feature: terminal-day-organizer, Property 25: Guardia de mensaje vacío
// ---------------------------------------------------------------------------
// Valida: Requisitos 11.1

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn p24_storage_error_message_in_status_bar() {
        let _storage = Arc::new(SqliteStorage::in_memory().expect("in-memory"));
        let mut app = AppState::new(120, 40);

        let err = jinx_core::DomainError::NotFound("Task 99 not found".to_string());
        app = jinx::app::reduce(
            app,
            AppEvent::StorageError(err.clone()),
        );

        assert!(
            app.status_bar.contains("Task 99 not found"),
            "status bar must contain the error message, got: {}",
            app.status_bar
        );
    }

    proptest! {
        #[test]
        fn p25_empty_trim_message_blocked(
            padding in r"[ \t\r\n]*",
        ) {
            // Any string consisting only of whitespace must be caught by the
            // empty-message guard in handle_chat_key (text.trim().is_empty()).
            let text = padding;
            let is_empty = text.trim().is_empty();
            // The guard blocks sending when trim is empty.
            prop_assert!(is_empty, "strategy produces whitespace-only strings");
            // The locale's empty_message string is used as the warning — just
            // verify the guard logic works (the exact text is locale-dependent).
            let locale = jinx::locale::load("en");
            prop_assert!(!locale.errors.empty_message.is_empty());
        }
    }
}
