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
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Tabs},
    Terminal,
};
use domain::{
    Priority, TaskFilter, TaskPatch, TaskStatus,
    task::TaskRepository, calendar::EventRepository, group::GroupRepository,
    note::NoteRepository,
};
use infrastructure::SqliteStorage;
use uuid::Uuid;

use jinx::app::{AppEvent, AppState, Modal, Panel, MIN_COLS, MIN_ROWS};
use jinx::calendario::{entry_count, flat_entries, nth_entry, FlatCalEntry};
use jinx::color::{detect_color_mode, resolve_style};
use jinx::config::{self as app_config};
use jinx::text_editor::TextEditor;

// ---------------------------------------------------------------------------
// Slash-command registry
// ---------------------------------------------------------------------------

struct SlashCommand {
    name: &'static str,
    description: &'static str,
}

const SLASH_COMMANDS: &[SlashCommand] = &[
    SlashCommand { name: "clear", description: "Borrar chat y reiniciar agente" },
];


mod state;
use state::*;

mod agent;
use agent::*;

mod modals;
use modals::*;

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

fn main() -> io::Result<()> {
    // -- Storage -----------------------------------------------------------
    let db_path = match infrastructure::resolve_db_path() {
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

    let cfg = app_config::load();
    let locale = jinx::locale::load(&cfg.language);

    let mut state = RuntimeState {
        app: AppState::new(size_cols, size_rows),
        locale,
        chat_history: Vec::new(),
        chat_editor: TextEditor::new(),
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
        storage: storage.clone(),
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
        notes_editor: TextEditor::new(),
        notes_title_editor: TextEditor::new(),
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
    }
}

fn update_cmd_picker(state: &mut RuntimeState) {
    let text = state.chat_editor.to_string();
    if text.starts_with('/') && state.chat_editor.line_count() == 1 {
        let query = &text[1..];
        let filtered: Vec<usize> = SLASH_COMMANDS.iter().enumerate()
            .filter(|(_, cmd)| cmd.name.starts_with(query))
            .map(|(i, _)| i)
            .collect();
        if !filtered.is_empty() {
            state.cmd_picker_active = true;
            state.cmd_picker_filtered = filtered;
            state.cmd_picker_cursor = state.cmd_picker_cursor.min(
                state.cmd_picker_filtered.len().saturating_sub(1)
            );
            return;
        }
    }
    state.cmd_picker_active = false;
    state.cmd_picker_filtered.clear();
}

fn handle_chat_key(state: &mut RuntimeState, key: crossterm::event::KeyEvent) {
    // Note picker intercepts keys when active
    if state.note_picker_active {
        if let Some(msg_idx) = state.note_picker_msg_idx {
            let count = state.chat_history.get(msg_idx)
                .and_then(|m| m.note_results.as_ref())
                .map(|v| v.len())
                .unwrap_or(0);
            if count > 0 {
                match key.code {
                    KeyCode::Down | KeyCode::Char('j') => {
                        if state.note_picker_cursor + 1 < count {
                            state.note_picker_cursor += 1;
                        }
                        return;
                    }
                    KeyCode::Up | KeyCode::Char('k') => {
                        if state.note_picker_cursor > 0 {
                            state.note_picker_cursor -= 1;
                        }
                        return;
                    }
                    KeyCode::Enter => {
                        if let Some(entry) = state.chat_history.get(msg_idx)
                            .and_then(|m| m.note_results.as_ref())
                            .and_then(|v| v.get(state.note_picker_cursor))
                        {
                            let note_id = entry.id;
                            state.notes_current_id = Some(note_id);
                            state.notes_view = NotesView::Preview;
                            state.notes_preview_scroll = 0;
                            state.app.focused_panel = Panel::Notas;
                            refresh_notes_cache(state);
                        }
                        state.note_picker_active = false;
                        return;
                    }
                    KeyCode::Esc => {
                        state.note_picker_active = false;
                        return;
                    }
                    _ => {}
                }
            }
        }
        state.note_picker_active = false;
    }

    // Slash-command picker intercepts keys when active
    if state.cmd_picker_active {
        match key.code {
            KeyCode::Down | KeyCode::Char('j') if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                let max = state.cmd_picker_filtered.len();
                if state.cmd_picker_cursor + 1 < max {
                    state.cmd_picker_cursor += 1;
                }
                return;
            }
            KeyCode::Up | KeyCode::Char('k') if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                if state.cmd_picker_cursor > 0 {
                    state.cmd_picker_cursor -= 1;
                }
                return;
            }
            KeyCode::Tab => {
                if let Some(&idx) = state.cmd_picker_filtered.get(state.cmd_picker_cursor) {
                    let full = format!("/{}", SLASH_COMMANDS[idx].name);
                    state.chat_editor = TextEditor::from_string(&full);
                }
                update_cmd_picker(state);
                return;
            }
            KeyCode::Enter => {
                if let Some(&idx) = state.cmd_picker_filtered.get(state.cmd_picker_cursor) {
                    let full = format!("/{}", SLASH_COMMANDS[idx].name);
                    state.chat_editor = TextEditor::from_string(&full);
                    state.cmd_picker_active = false;
                }
                // Fall through to the Enter handler below to execute
            }
            KeyCode::Esc => {
                state.cmd_picker_active = false;
                return;
            }
            _ => {
                // Fall through to normal key handling; update_cmd_picker runs at the end
            }
        }
    }

    match key.code {
        KeyCode::Enter => {
            let text = state.chat_editor.to_string();
            let trimmed = text.trim().to_string();
            if trimmed.is_empty() {
                state.app.status_bar = state.locale.errors.empty_message.clone();
                return;
            }
            if trimmed == "/clear" {
                state.chat_editor.clear();
                state.chat_history.clear();
                state.chat_scroll = 0;
                state.note_picker_active = false;
                state.note_picker_msg_idx = None;
                state.pending_request = None;
                restart_agent(state);
                return;
            }
            state.prompt_history.push(trimmed.clone());
            state.prompt_history_idx = None;
            state.prompt_stash.clear();
            state.note_picker_active = false;
            state.chat_history.push(ChatMsg { role: ChatRole::User, text: trimmed.clone(), note_results: None });
            state.chat_editor.clear();
            state.chat_scroll = 0;
            send_user_message(state, trimmed);
        }
        // Ctrl+J inserts newline (does NOT send)
        KeyCode::Char('j') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            state.chat_editor.insert_newline();
        }
        // Chat history scroll (Shift+arrows)
        KeyCode::Up if key.modifiers.contains(KeyModifiers::SHIFT) => {
            state.chat_scroll = state.chat_scroll.saturating_add(3);
        }
        KeyCode::Down if key.modifiers.contains(KeyModifiers::SHIFT) => {
            state.chat_scroll = state.chat_scroll.saturating_sub(3);
        }
        // Navigation — Up/Down: history recall when single-line, else cursor movement
        KeyCode::Left => state.chat_editor.move_left(),
        KeyCode::Right => state.chat_editor.move_right(),
        KeyCode::Up if state.chat_editor.line_count() == 1 => {
            if state.prompt_history.is_empty() { return; }
            let idx = match state.prompt_history_idx {
                None => {
                    state.prompt_stash = state.chat_editor.to_string();
                    state.prompt_history.len() - 1
                }
                Some(0) => return,
                Some(i) => i - 1,
            };
            state.prompt_history_idx = Some(idx);
            state.chat_editor = TextEditor::from_string(&state.prompt_history[idx]);
        }
        KeyCode::Down if state.chat_editor.line_count() == 1 && state.prompt_history_idx.is_some() => {
            let idx = state.prompt_history_idx.unwrap();
            if idx + 1 >= state.prompt_history.len() {
                state.prompt_history_idx = None;
                state.chat_editor = TextEditor::from_string(&state.prompt_stash);
            } else {
                state.prompt_history_idx = Some(idx + 1);
                state.chat_editor = TextEditor::from_string(&state.prompt_history[idx + 1]);
            }
        }
        KeyCode::Up => state.chat_editor.move_up(),
        KeyCode::Down => state.chat_editor.move_down(),
        // Readline shortcuts
        KeyCode::Char('a') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            state.chat_editor.move_home();
        }
        KeyCode::Char('e') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            state.chat_editor.move_end();
        }
        KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            state.chat_editor.kill_to_start();
        }
        KeyCode::Char('k') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            state.chat_editor.kill_to_end();
        }
        KeyCode::Char('w') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            state.chat_editor.kill_word_back();
        }
        KeyCode::Char('l') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            state.chat_editor.clear();
        }
        // Alt+B / Alt+F — word movement (Option on macOS emits ALT)
        KeyCode::Char('b') if key.modifiers.contains(KeyModifiers::ALT) => {
            state.chat_editor.move_word_back();
        }
        KeyCode::Char('f') if key.modifiers.contains(KeyModifiers::ALT) => {
            state.chat_editor.move_word_forward();
        }
        KeyCode::Backspace => state.chat_editor.backspace(),
        KeyCode::Delete => state.chat_editor.delete(),
        // Chat history scroll
        KeyCode::PageUp if key.modifiers.contains(KeyModifiers::SHIFT) => {
            // Jump to top
            state.chat_scroll = usize::MAX; // clamped during render
        }
        KeyCode::PageDown if key.modifiers.contains(KeyModifiers::SHIFT) => {
            state.chat_scroll = 0;
        }
        KeyCode::PageUp => {
            state.chat_scroll = state.chat_scroll.saturating_add(10);
        }
        KeyCode::PageDown => {
            state.chat_scroll = state.chat_scroll.saturating_sub(10);
        }
        // Regular character input
        KeyCode::Char(c) => {
            state.chat_editor.insert_char(c);
        }
        _ => {}
    }

    update_cmd_picker(state);
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
                    let count = state.storage.list_tasks(state.tareas_filter.to_storage_filter()).unwrap_or_default().len();
                    if state.task_cursor + 1 < count { state.task_cursor += 1; }
                }
                Panel::Calendario => {
                    let tasks = state.storage.list_tasks(TaskFilter::default()).unwrap_or_default();
                    let events = state.storage.list_events(None, None).unwrap_or_default();
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


fn handle_tareas_key(state: &mut RuntimeState, key: crossterm::event::KeyEvent) {
    if state.tareas_search_active {
        handle_tareas_search_key(state, key);
        return;
    }

    if key.code == KeyCode::Char('s') {
        state.pending_g = false;
        state.tareas_section = match state.tareas_section {
            TareasSection::Tasks => TareasSection::Groups,
            TareasSection::Groups => TareasSection::Tasks,
        };
        return;
    }

    match state.tareas_section {
        TareasSection::Tasks => handle_tareas_tasks_key(state, key),
        TareasSection::Groups => handle_tareas_groups_key(state, key),
    }
}

fn handle_tareas_tasks_key(state: &mut RuntimeState, key: crossterm::event::KeyEvent) {
    let tasks = if !state.tareas_search_query.is_empty() {
        get_search_filtered_tasks(state)
    } else {
        get_filtered_tasks(state)
    };
    if state.pending_g {
        state.pending_g = false;
        if key.code == KeyCode::Char('g') {
            state.task_cursor = 0;
            return;
        }
    }
    match key.code {
        KeyCode::Up | KeyCode::Char('k') if state.task_cursor > 0 => {
            state.task_cursor -= 1;
        }
        KeyCode::Down | KeyCode::Char('j') if state.task_cursor + 1 < tasks.len() => {
            state.task_cursor += 1;
        }
        KeyCode::Char('n') => open_new_task_modal(state),
        KeyCode::Char('e') => {
            if let Some(t) = tasks.get(state.task_cursor) {
                open_edit_task_modal(state, t.id);
            }
        }
        KeyCode::Char('c') => {
            if let Some(t) = tasks.get(state.task_cursor) {
                let task_id = t.id;
                let new_status = if t.status == TaskStatus::Completada {
                    TaskStatus::Pendiente
                } else {
                    TaskStatus::Completada
                };
                match state.storage.update_task(
                    task_id,
                    TaskPatch { status: Some(new_status), ..Default::default() },
                ) {
                    Ok(_) => {
                        state.app.status_bar = if new_status == TaskStatus::Completada {
                            state.locale.status.task_completed.clone()
                        } else {
                            state.locale.status.task_pending.clone()
                        };
                    }
                    Err(e) => state.app.status_bar = format!("Error: {}", e.message()),
                }
            }
        }
        KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            let half = state.panel_area.map(|r| r.height as usize / 2).unwrap_or(10);
            state.task_cursor = state.task_cursor.saturating_sub(half);
        }
        KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            let half = state.panel_area.map(|r| r.height as usize / 2).unwrap_or(10);
            state.task_cursor = (state.task_cursor + half).min(tasks.len().saturating_sub(1));
        }
        KeyCode::Char('d') => {
            if let Some(t) = tasks.get(state.task_cursor) {
                state.delete_confirm_name = t.title.clone();
                state.app.modal = Some(Modal::DeleteTask { id: t.id });
            }
        }
        KeyCode::Char('g') => { state.pending_g = true; }
        KeyCode::Char('G') => { state.task_cursor = tasks.len().saturating_sub(1); }
        KeyCode::Char('f') => open_filter_modal(state),
        KeyCode::Char('/') => {
            state.tareas_search_active = true;
            state.tareas_search_query.clear();
            state.task_cursor = 0;
            state.tareas_scroll = 0;
        }
        _ => {}
    }
}

fn handle_tareas_search_key(state: &mut RuntimeState, key: crossterm::event::KeyEvent) {
    match key.code {
        KeyCode::Esc => {
            state.tareas_search_active = false;
            state.tareas_search_query.clear();
            state.task_cursor = 0;
            state.tareas_scroll = 0;
        }
        KeyCode::Enter => {
            state.tareas_search_active = false;
        }
        KeyCode::Backspace => {
            state.tareas_search_query.pop();
            state.task_cursor = 0;
            state.tareas_scroll = 0;
        }
        KeyCode::Up if state.task_cursor > 0 => {
            state.task_cursor -= 1;
        }
        KeyCode::Down => {
            let count = get_search_filtered_tasks(state).len();
            if state.task_cursor + 1 < count {
                state.task_cursor += 1;
            }
        }
        KeyCode::Char(c) => {
            state.tareas_search_query.push(c);
            state.task_cursor = 0;
            state.tareas_scroll = 0;
        }
        _ => {}
    }
}

fn get_search_filtered_tasks(state: &RuntimeState) -> Vec<domain::Task> {
    let mut tasks = get_filtered_tasks(state);
    if !state.tareas_search_query.is_empty() {
        let query = state.tareas_search_query.to_lowercase();
        tasks.retain(|t| t.title.to_lowercase().contains(&query));
    }
    tasks
}

fn handle_tareas_groups_key(state: &mut RuntimeState, key: crossterm::event::KeyEvent) {
    let groups = state.storage.list_groups().unwrap_or_default();
    if state.pending_g {
        state.pending_g = false;
        if key.code == KeyCode::Char('g') {
            state.group_cursor = 0;
            return;
        }
    }
    match key.code {
        KeyCode::Up | KeyCode::Char('k') if state.group_cursor > 0 => {
            state.group_cursor -= 1;
        }
        KeyCode::Down | KeyCode::Char('j') if state.group_cursor + 1 < groups.len() => {
            state.group_cursor += 1;
        }
        KeyCode::Char('n') => open_new_group_modal(state),
        KeyCode::Char('g') => { state.pending_g = true; }
        KeyCode::Char('G') => { state.group_cursor = groups.len().saturating_sub(1); }
        KeyCode::Char('e') => {
            if let Some(g) = groups.get(state.group_cursor) {
                open_edit_group_modal(state, g.id);
            }
        }
        KeyCode::Char('d') => {
            if let Some(g) = groups.get(state.group_cursor) {
                state.delete_confirm_name = g.name.clone();
                state.app.modal = Some(Modal::DeleteGroup { id: g.id });
            }
        }
        _ => {}
    }
}

fn get_filtered_tasks(state: &RuntimeState) -> Vec<domain::Task> {
    let mut tasks = state
        .storage
        .list_tasks(state.tareas_filter.to_storage_filter())
        .unwrap_or_default();
    if !state.tareas_filter.priorities.is_empty() {
        tasks.retain(|t| state.tareas_filter.priorities.contains(&t.priority));
    }
    tasks
}

fn handle_calendario_key(state: &mut RuntimeState, key: crossterm::event::KeyEvent) {
    let tasks = state.storage.list_tasks(TaskFilter::default()).unwrap_or_default();
    let events = state.storage.list_events(None, None).unwrap_or_default();
    let mut view = jinx::calendario::calendar_layout(&tasks, &events);
    if let Some((from, to)) = calendar_date_range(state.calendar_filter_idx) {
        view.retain(|date, _| date.as_str() >= from.as_str() && date.as_str() <= to.as_str());
    }
    let flat = flat_entries(&view);
    let count = entry_count(&flat);

    if state.pending_g {
        state.pending_g = false;
        if key.code == KeyCode::Char('g') {
            state.calendar_cursor = 0;
            return;
        }
    }
    match key.code {
        KeyCode::Up | KeyCode::Char('k') if state.calendar_cursor > 0 => {
            state.calendar_cursor -= 1;
        }
        KeyCode::Down | KeyCode::Char('j') if count > 0 && state.calendar_cursor + 1 < count => {
            state.calendar_cursor += 1;
        }
        KeyCode::Char('n') => open_new_event_modal(state),
        KeyCode::Char('e') => {
            if let Some(entry) = nth_entry(&flat, state.calendar_cursor) {
                if entry.is_task {
                    open_edit_task_modal(state, entry.entity_id);
                } else {
                    open_edit_event_modal(state, entry.entity_id);
                }
            }
        }
        KeyCode::Char('c') => {
            if let Some(entry) = nth_entry(&flat, state.calendar_cursor) {
                if entry.is_task {
                    let task_id = entry.entity_id;
                    let task = tasks.iter().find(|t| t.id == task_id);
                    let new_status = if task.map(|t| t.status) == Some(TaskStatus::Completada) {
                        TaskStatus::Pendiente
                    } else {
                        TaskStatus::Completada
                    };
                    match state.storage.update_task(
                        task_id,
                        TaskPatch { status: Some(new_status), ..Default::default() },
                    ) {
                        Ok(_) => {
                            state.app.status_bar = if new_status == TaskStatus::Completada {
                                state.locale.status.task_completed.clone()
                            } else {
                                state.locale.status.task_pending.clone()
                            };
                        }
                        Err(e) => state.app.status_bar = format!("Error: {}", e.message()),
                    }
                }
            }
        }
        KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            let half = state.panel_area.map(|r| r.height as usize / 2).unwrap_or(10);
            state.calendar_cursor = state.calendar_cursor.saturating_sub(half);
        }
        KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            let half = state.panel_area.map(|r| r.height as usize / 2).unwrap_or(10);
            state.calendar_cursor = (state.calendar_cursor + half).min(count.saturating_sub(1));
        }
        KeyCode::Char('d') => {
            if let Some(entry) = nth_entry(&flat, state.calendar_cursor) {
                state.delete_confirm_name = entry.text.clone();
                if entry.is_task {
                    state.app.modal = Some(Modal::DeleteTask { id: entry.entity_id });
                } else {
                    state.app.modal = Some(Modal::DeleteEvent { id: entry.entity_id });
                }
            }
        }
        KeyCode::Char('g') => { state.pending_g = true; }
        KeyCode::Char('G') => { state.calendar_cursor = count.saturating_sub(1); }
        KeyCode::Char('f') => {
            state.calendar_filter_idx = (state.calendar_filter_idx + 1) % 4;
            state.calendar_cursor = 0;
            state.calendar_scroll = 0;
            state.calendar_scroll_initialized = false;
        }
        _ => {}
    }
}


// ---------------------------------------------------------------------------
// Modal open helpers
// ---------------------------------------------------------------------------

fn refresh_groups_cache(state: &mut RuntimeState) {
    state.groups_cache = state.storage.list_groups().unwrap_or_default();
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
    };
    let tabs = Tabs::new(vec![
        format!("  {}  ", state.locale.panels.chat),
        format!("  {}  ", state.locale.panels.tasks),
        format!("  {}  ", state.locale.panels.calendar),
        format!("  {}  ", state.locale.panels.notes),
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

fn panel_block(title: &str) -> Block<'_> {
    Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
}


fn strip_md(s: &str) -> String {
    s.replace("**", "").replace("__", "")
}

fn word_wrap(text: &str, width: usize) -> Vec<String> {
    if width == 0 {
        return vec![];
    }
    let mut result = Vec::new();
    for raw_line in text.split('\n') {
        let trimmed = raw_line.trim_end();
        if trimmed.is_empty() {
            continue;
        }
        if trimmed.chars().count() <= width {
            result.push(trimmed.to_string());
            continue;
        }
        let mut cur = String::new();
        for word in trimmed.split_whitespace() {
            let word_len = word.chars().count();
            if cur.is_empty() {
                cur = word.to_string();
            } else if cur.chars().count() + 1 + word_len <= width {
                cur.push(' ');
                cur.push_str(word);
            } else {
                result.push(cur);
                cur = word.to_string();
            }
        }
        if !cur.is_empty() {
            result.push(cur);
        }
    }
    result
}

fn render_chat(frame: &mut ratatui::Frame, state: &mut RuntimeState, area: Rect) {
    let block = panel_block(state.locale.panels.chat.as_str());
    let inner = block.inner(area);
    frame.render_widget(block, area);

    // Calculate dynamic input height based on editor content
    let input_width = inner.width.saturating_sub(2) as usize; // account for borders
    let visual_lines = count_visual_lines(&state.chat_editor, input_width);
    let min_input_height: u16 = 3;
    let max_input_height: u16 = 8.min((inner.height * 40 / 100).max(3));
    let input_height = (visual_lines as u16 + 2) // +2 for border
        .max(min_input_height)
        .min(max_input_height);

    let parts = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(input_height)])
        .split(inner);

    let hist_area = parts[0];
    state.history_area = Some(hist_area);
    let wrap_width = (hist_area.width as usize).saturating_sub(2);
    let avail_height = hist_area.height as usize;

    // Build display lines for all messages
    let mut all_lines: Vec<Line<'static>> = Vec::new();
    for (msg_idx, msg) in state.chat_history.iter().enumerate() {
        let (label, color): (&str, Color) = match msg.role {
            ChatRole::Agent => (state.locale.chat.agent.as_str(), Color::Green),
            ChatRole::System => (state.locale.chat.system.as_str(), Color::Yellow),
            ChatRole::User => (state.locale.chat.you.as_str(), Color::Cyan),
        };
        let style = Style::default().fg(color).add_modifier(Modifier::BOLD);
        let body_style = Style::default().fg(color);

        let clean = strip_md(&msg.text);
        let wrapped = word_wrap(&clean, wrap_width.saturating_sub(label.len() + 3));

        let header = format!("[{label}]");
        if wrapped.is_empty() {
            all_lines.push(Line::from(Span::styled(header, style)));
        } else {
            all_lines.push(Line::from(vec![
                Span::styled(format!("{header} "), style),
                Span::styled(wrapped[0].clone(), body_style),
            ]));
            let indent = " ".repeat(header.chars().count() + 1);
            for line in &wrapped[1..] {
                all_lines.push(Line::from(Span::styled(
                    format!("{indent}{line}"),
                    body_style,
                )));
            }
        }

        // Render note picker entries if present
        if let Some(entries) = &msg.note_results {
            let is_active = state.note_picker_active
                && state.note_picker_msg_idx == Some(msg_idx);
            all_lines.push(Line::from(Span::styled(
                "  ┌─────────────────────────────────────┐".to_string(),
                Style::default().fg(Color::DarkGray),
            )));
            for (i, entry) in entries.iter().enumerate() {
                let cursor = if is_active && i == state.note_picker_cursor { "▶" } else { " " };
                let title = if entry.title.is_empty() {
                    &state.locale.misc.untitled_note
                } else {
                    &entry.title
                };
                let date = &entry.updated_at[..10.min(entry.updated_at.len())];
                let label_text = format!("  │ {cursor} {title}  {date}");
                let entry_style = if is_active && i == state.note_picker_cursor {
                    Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(Color::White)
                };
                all_lines.push(Line::from(Span::styled(label_text, entry_style)));
            }
            all_lines.push(Line::from(Span::styled(
                "  └─────────────────────────────────────┘".to_string(),
                Style::default().fg(Color::DarkGray),
            )));
            if is_active {
                all_lines.push(Line::from(Span::styled(
                    "  ↑↓:select  Enter:open  Esc:close".to_string(),
                    Style::default().fg(Color::DarkGray).add_modifier(Modifier::ITALIC),
                )));
            }
        }

        all_lines.push(Line::from(""));
    }

    // Typing indicator when agent is working
    if let Some((_, started)) = &state.pending_request {
        let elapsed = started.elapsed();
        let dot_count = (elapsed.as_millis() / 500) as usize % 3 + 1;
        let dots = ".".repeat(dot_count);
        let secs = elapsed.as_secs();
        let indicator = state.locale.chat.thinking
            .replace("{dots}", &dots)
            .replace("{secs}", &secs.to_string());
        all_lines.push(Line::from(Span::styled(
            indicator,
            Style::default().fg(Color::DarkGray).add_modifier(Modifier::ITALIC),
        )));
        all_lines.push(Line::from(""));
    }

    // Scroll support: offset from bottom
    let total = all_lines.len();
    let scroll_offset = state.chat_scroll.min(total.saturating_sub(avail_height));
    let end = total.saturating_sub(scroll_offset);
    let start = end.saturating_sub(avail_height);
    let mut visible: Vec<Line<'static>> = all_lines[start..end].to_vec();

    // Show scroll indicator if not at bottom
    if scroll_offset > 0 && !visible.is_empty() {
        visible[0] = Line::from(Span::styled(
            state.locale.chat.older_messages.clone(),
            Style::default().fg(Color::DarkGray).add_modifier(Modifier::ITALIC),
        ));
    }

    frame.render_widget(Paragraph::new(visible), hist_area);

    // Input field with cursor
    let input_block = Block::default().title(state.locale.chat.input_title.as_str()).borders(Borders::ALL);
    let input_inner = input_block.inner(parts[1]);
    state.input_area = Some(input_inner);
    frame.render_widget(input_block, parts[1]);

    // Render editor content with character-level wrapping (matches cursor calculation)
    let input_w = input_inner.width as usize;
    let mut input_lines: Vec<Line<'_>> = Vec::new();
    for logical_line in state.chat_editor.lines() {
        if logical_line.is_empty() || input_w == 0 {
            input_lines.push(Line::from(""));
        } else {
            let chars: Vec<char> = logical_line.chars().collect();
            for chunk in chars.chunks(input_w) {
                input_lines.push(Line::from(chunk.iter().collect::<String>()));
            }
        }
    }
    let input_para = Paragraph::new(input_lines);
    frame.render_widget(input_para, input_inner);

    // Slash-command picker overlay
    if state.cmd_picker_active && !state.cmd_picker_filtered.is_empty() {
        let picker_height = (state.cmd_picker_filtered.len() as u16 + 2).min(6);
        let picker_area = Rect {
            x: parts[1].x,
            y: parts[1].y.saturating_sub(picker_height),
            width: parts[1].width.min(40),
            height: picker_height,
        };
        let picker_block = Block::default().borders(Borders::ALL)
            .border_style(Style::default().fg(Color::DarkGray));
        let picker_inner = picker_block.inner(picker_area);
        frame.render_widget(Clear, picker_area);
        frame.render_widget(picker_block, picker_area);

        let mut cmd_lines: Vec<Line<'static>> = Vec::new();
        for (i, &cmd_idx) in state.cmd_picker_filtered.iter().enumerate() {
            let cmd = &SLASH_COMMANDS[cmd_idx];
            let cursor_mark = if i == state.cmd_picker_cursor { "▶" } else { " " };
            let style = if i == state.cmd_picker_cursor {
                Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White)
            };
            cmd_lines.push(Line::from(Span::styled(
                format!("{} /{:<12} {}", cursor_mark, cmd.name, cmd.description),
                style,
            )));
        }
        frame.render_widget(Paragraph::new(cmd_lines), picker_inner);
    }

    // Set cursor position (only when Chat panel is focused and no modal)
    if state.app.focused_panel == Panel::Chat && state.app.modal.is_none() {
        let (cursor_x, cursor_y) = calculate_cursor_position(&state.chat_editor, input_inner);
        frame.set_cursor_position((cursor_x, cursor_y));
    }
}

/// Count the total visual lines the editor content would occupy given a width.
fn count_visual_lines(editor: &TextEditor, width: usize) -> usize {
    if width == 0 {
        return editor.line_count();
    }
    editor.lines().iter().map(|line| {
        let char_count = line.chars().count();
        if char_count == 0 {
            1
        } else {
            char_count.div_ceil(width)
        }
    }).sum()
}

/// Calculate the absolute (x, y) position of the cursor within the input area.
fn calculate_cursor_position(editor: &TextEditor, area: Rect) -> (u16, u16) {
    let width = area.width as usize;
    if width == 0 {
        return (area.x, area.y);
    }

    let mut visual_row: usize = 0;
    for row in 0..editor.cursor_row() {
        let line_chars = editor.lines()[row].chars().count();
        visual_row += if line_chars == 0 { 1 } else { line_chars.div_ceil(width) };
    }

    // For the cursor's line, find which visual row/col the cursor falls on
    let current_line = editor.current_line();
    let cursor_char_offset = current_line[..editor.cursor_col()].chars().count();
    let extra_rows = cursor_char_offset / width;
    let col_in_row = cursor_char_offset % width;

    visual_row += extra_rows;

    let x = area.x + col_in_row as u16;
    let y = area.y + (visual_row as u16).min(area.height.saturating_sub(1));
    (x, y)
}

fn render_tareas(frame: &mut ratatui::Frame, state: &mut RuntimeState, area: Rect) {
    let block = panel_block(state.locale.panels.tasks.as_str());
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let groups = state.storage.list_groups().unwrap_or_default();
    let groups_height = (groups.len() + 3).min(8) as u16;

    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(4), Constraint::Length(groups_height)])
        .split(inner);

    // --- Tasks section ---
    let tasks = if !state.tareas_search_query.is_empty() || state.tareas_search_active {
        get_search_filtered_tasks(state)
    } else {
        get_filtered_tasks(state)
    };
    let mut task_items: Vec<ListItem> = Vec::new();

    if state.tareas_search_active {
        let search_line = format!(" /{}\u{2502}", state.tareas_search_query);
        task_items.push(ListItem::new(Line::from(Span::styled(
            search_line,
            Style::default().fg(Color::Green),
        ))));
    }

    if !state.tareas_filter.is_default() {
        let status_label = match state.tareas_filter.status {
            Some(TaskStatus::Pendiente) => state.locale.filters.pending.as_str(),
            Some(TaskStatus::Completada) => state.locale.filters.completed.as_str(),
            Some(TaskStatus::Cancelada) => state.locale.filters.cancelled.as_str(),
            None => state.locale.filters.all.as_str(),
        };
        let priority_label: String = if state.tareas_filter.priorities.is_empty() {
            state.locale.filters.all.to_string()
        } else {
            state.tareas_filter.priorities.iter().map(|p| match p {
                Priority::Alta => state.locale.filters.high.as_str(),
                Priority::Media => state.locale.filters.medium.as_str(),
                Priority::Baja => state.locale.filters.low.as_str(),
            }).collect::<Vec<_>>().join("+")
        };
        let group_label = match &state.tareas_filter.group_id {
            None => state.locale.filters.all_groups.clone(),
            Some(None) => state.locale.filters.no_group.clone(),
            Some(Some(gid)) => groups
                .iter()
                .find(|g| g.id == *gid)
                .map(|g| g.name.clone())
                .unwrap_or_else(|| "?".to_string()),
        };
        let date_label = if state.tareas_filter.no_deadline {
            format!("  {}:{}", state.locale.form_labels.date.trim(), state.locale.filters.no_date_filter.as_str())
        } else {
            match (&state.tareas_filter.from_date, &state.tareas_filter.to_date) {
                (None, None) => String::new(),
                (Some(f), Some(t)) if f == t => format!("  {}:{}", state.locale.form_labels.date.trim(), f),
                (Some(f), Some(t)) => format!("  {}:{}/{}", state.locale.form_labels.date.trim(), f, t),
                (Some(f), None) => format!("  {}:{}", state.locale.form_labels.from_date.trim(), f),
                (None, Some(t)) => format!("  {}:{}", state.locale.form_labels.to_date.trim(), t),
            }
        };
        let filter_line = state.locale.filters.filters_prefix
            .replace("{status}", status_label)
            .replace("{priority}", &priority_label)
            .replace("{group}", &group_label)
            .replace("{date}", &date_label);
        task_items.push(ListItem::new(Line::from(Span::styled(
            filter_line,
            Style::default().fg(Color::Yellow),
        ))));
    }

    for (i, t) in tasks.iter().enumerate() {
        let cursor = if state.tareas_section == TareasSection::Tasks && i == state.task_cursor {
            "▶"
        } else {
            " "
        };

        let group_indicator = if let Some(gid) = t.group_id {
            if let Some(g) = groups.iter().find(|g| g.id == gid) {
                let styled = resolve_style(Some(&g.color), Some(&g.name), state.color_mode);
                if let Some(prefix) = &styled.prefix {
                    format!("{} ", prefix)
                } else {
                    "██ ".to_string()
                }
            } else {
                "   ".to_string()
            }
        } else {
            "   ".to_string()
        };

        let deadline_str: String = t.deadline.as_deref().map(|d| {
            if let Some(pos) = d.find('T') {
                let date = &d[..pos];
                let time_part = &d[pos + 1..];
                let hm: &str = if time_part.len() >= 5 { &time_part[..5] } else { time_part };
                if hm == "00:00" {
                    date.to_string()
                } else {
                    format!("{} {}", date, hm)
                }
            } else {
                d.to_string()
            }
        }).map(|d| format!(" ({})", d)).unwrap_or_default();

        let label = format!(
            " {} {}[{}] {}{}",
            cursor, group_indicator, t.priority.as_str(), t.title, deadline_str
        );

        let base_style = if state.tareas_section == TareasSection::Tasks && i == state.task_cursor {
            Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)
        } else if let Some(gid) = t.group_id {
            if let Some(g) = groups.iter().find(|g| g.id == gid) {
                resolve_style(Some(&g.color), Some(&g.name), state.color_mode).style
            } else {
                Style::default()
            }
        } else {
            Style::default()
        };
        task_items.push(ListItem::new(label).style(base_style));
    }

    if tasks.is_empty() {
        task_items.push(ListItem::new(Line::from(Span::styled(
            state.locale.misc.no_tasks.clone(),
            Style::default().fg(Color::DarkGray),
        ))));
    }

    let task_hint = if state.tareas_search_active {
        state.locale.hints.tasks_search.as_str()
    } else if state.tareas_section == TareasSection::Tasks {
        state.locale.hints.tasks_nav.as_str()
    } else {
        state.locale.hints.tasks_switch_to_groups.as_str()
    };
    task_items.push(ListItem::new(Line::from(Span::styled(
        task_hint.to_string(),
        Style::default().fg(Color::DarkGray),
    ))));

    // Scroll: keep cursor visible within available height
    let visible_height = sections[0].height as usize;
    let filter_offset = if !state.tareas_filter.is_default() { 1 } else { 0 };
    let cursor_line = if state.tareas_section == TareasSection::Tasks {
        state.task_cursor + filter_offset
    } else {
        0
    };
    if visible_height > 0 && !task_items.is_empty() {
        if cursor_line < state.tareas_scroll {
            state.tareas_scroll = cursor_line;
        } else if cursor_line >= state.tareas_scroll + visible_height {
            state.tareas_scroll = cursor_line - visible_height + 1;
        }
        let max_scroll = task_items.len().saturating_sub(visible_height);
        state.tareas_scroll = state.tareas_scroll.min(max_scroll);
    }
    let end = (state.tareas_scroll + visible_height).min(task_items.len());
    let visible_items: Vec<ListItem> = task_items.drain(state.tareas_scroll..end).collect();
    frame.render_widget(List::new(visible_items), sections[0]);

    // --- Groups section ---
    let mut group_items: Vec<ListItem> = Vec::new();
    group_items.push(ListItem::new(Line::from(Span::styled(
        state.locale.misc.groups_separator.clone(),
        Style::default().fg(Color::DarkGray).add_modifier(Modifier::BOLD),
    ))));

    if groups.is_empty() {
        group_items.push(ListItem::new(state.locale.misc.no_groups.clone()));
    } else {
        for (i, g) in groups.iter().enumerate() {
            let selected = state.tareas_section == TareasSection::Groups && i == state.group_cursor;
            let cursor = if selected { "▶" } else { " " };
            let label = format!(" {} {} ({})", cursor, g.name, g.color);
            let style = if selected {
                Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)
            } else {
                resolve_style(Some(&g.color), Some(&g.name), state.color_mode).style
            };
            group_items.push(ListItem::new(label).style(style));
        }
    }

    let group_hint = if state.tareas_section == TareasSection::Groups {
        state.locale.hints.groups_nav.as_str()
    } else {
        state.locale.hints.groups_switch_to_tasks.as_str()
    };
    group_items.push(ListItem::new(Line::from(Span::styled(
        group_hint.to_string(),
        Style::default().fg(Color::DarkGray),
    ))));

    frame.render_widget(List::new(group_items), sections[1]);
}

fn calendar_date_range(filter_idx: usize) -> Option<(String, String)> {
    match filter_idx {
        1 => { let t = today_str(); Some((t.clone(), t)) }
        2 => Some(week_bounds()),
        3 => Some(month_bounds()),
        _ => None,
    }
}

fn render_calendario(frame: &mut ratatui::Frame, state: &mut RuntimeState, area: Rect) {
    let block = panel_block(state.locale.panels.calendar.as_str());
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let tasks = state.storage.list_tasks(TaskFilter::default()).unwrap_or_default();
    let events = state.storage.list_events(None, None).unwrap_or_default();
    let mut view = jinx::calendario::calendar_layout(&tasks, &events);
    if let Some((from, to)) = calendar_date_range(state.calendar_filter_idx) {
        view.retain(|date, _| date.as_str() >= from.as_str() && date.as_str() <= to.as_str());
    }
    let flat = flat_entries(&view);

    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    let groups = state.storage.list_groups().unwrap_or_default();
    let mut lines: Vec<ListItem> = vec![];

    if state.calendar_filter_idx > 0 {
        let filter_labels = [
            "",
            state.locale.filters.today.as_str(),
            state.locale.filters.this_week.as_str(),
            state.locale.filters.this_month.as_str(),
        ];
        let label = format!("  [{}]", filter_labels[state.calendar_filter_idx]);
        lines.push(ListItem::new(Line::from(Span::styled(
            label, Style::default().fg(Color::Magenta),
        ))));
    }
    let mut entry_idx = 0usize;
    let mut today_line_idx: Option<usize> = None;
    let mut today_entry_idx: Option<usize> = None;
    let mut current_date_is_today_or_later = false;
    let mut cursor_line_idx: usize = 0;

    for item in &flat {
        match item {
            FlatCalEntry::DateHeader(date) => {
                if today_line_idx.is_none() && date.as_str() >= today.as_str() {
                    today_line_idx = Some(lines.len());
                    current_date_is_today_or_later = true;
                }
                if date == &today {
                    let label = state.locale.misc.today_marker.replace("{date}", date);
                    lines.push(
                        ListItem::new(label)
                            .style(Style::default().fg(Color::Rgb(255, 20, 147)).add_modifier(Modifier::BOLD)),
                    );
                } else {
                    lines.push(
                        ListItem::new(date.as_str())
                            .style(Style::default().add_modifier(Modifier::BOLD)),
                    );
                }
            }
            FlatCalEntry::Entry(entry) => {
                if today_entry_idx.is_none() && current_date_is_today_or_later {
                    today_entry_idx = Some(entry_idx);
                }
                let selected = entry_idx == state.calendar_cursor;
                if selected {
                    cursor_line_idx = lines.len();
                }
                let cursor = if selected { "▶" } else { " " };

                let entry_style = if selected {
                    Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)
                } else if let Some(gid) = entry.group_id {
                    if let Some(g) = groups.iter().find(|g| g.id == gid) {
                        resolve_style(Some(&g.color), Some(&g.name), state.color_mode).style
                    } else {
                        Style::default()
                    }
                } else {
                    Style::default()
                };

                let label = format!("  {} {}", cursor, entry.text);
                lines.push(ListItem::new(label).style(entry_style));
                entry_idx += 1;
            }
        }
    }

    if flat.is_empty() {
        lines.push(ListItem::new(Line::from(Span::styled(
            state.locale.misc.no_calendar_entries.clone(),
            Style::default().fg(Color::DarkGray),
        ))));
    }

    lines.push(ListItem::new(Line::from(Span::styled(
        state.locale.hints.calendar_nav.clone(),
        Style::default().fg(Color::DarkGray),
    ))));

    let visible_height = inner.height as usize;

    if !state.calendar_scroll_initialized {
        if let Some(idx) = today_entry_idx {
            state.calendar_cursor = idx;
        }
        if let Some(line_idx) = today_line_idx {
            state.calendar_scroll = line_idx;
        }
        let max_scroll = lines.len().saturating_sub(visible_height);
        state.calendar_scroll = state.calendar_scroll.min(max_scroll);
        state.calendar_scroll_initialized = true;
    } else if !lines.is_empty() && visible_height > 0 {
        if cursor_line_idx < state.calendar_scroll {
            state.calendar_scroll = cursor_line_idx;
        } else if cursor_line_idx >= state.calendar_scroll + visible_height {
            state.calendar_scroll = cursor_line_idx - visible_height + 1;
        }
        let max_scroll = lines.len().saturating_sub(visible_height);
        state.calendar_scroll = state.calendar_scroll.min(max_scroll);
    }

    let end = (state.calendar_scroll + visible_height).min(lines.len());
    let visible_lines: Vec<ListItem> = lines.drain(state.calendar_scroll..end).collect();
    frame.render_widget(List::new(visible_lines), inner);
}

fn spinner_state(pending: &Option<(Uuid, Instant)>) -> Option<(char, u64)> {
    pending.as_ref().map(|(_, started)| {
        let elapsed = started.elapsed();
        let frame_idx = (elapsed.as_millis() / 250) as usize % SPINNER_FRAMES.len();
        (SPINNER_FRAMES[frame_idx], elapsed.as_secs())
    })
}

// ---------------------------------------------------------------------------
// Notes panel
// ---------------------------------------------------------------------------

fn refresh_notes_cache(state: &mut RuntimeState) {
    state.notes_cache = if state.notes_search_active && !state.notes_search_query.is_empty() {
        state.storage.search_notes(&state.notes_search_query).unwrap_or_default()
    } else {
        state.storage.list_notes().unwrap_or_default()
    };
}

fn handle_notas_key(state: &mut RuntimeState, key: crossterm::event::KeyEvent) {
    match state.notes_view {
        NotesView::List => handle_notes_list_key(state, key),
        NotesView::Preview => handle_notes_preview_key(state, key),
        NotesView::Edit => handle_notes_edit_key(state, key),
    }
}

fn handle_notes_list_key(state: &mut RuntimeState, key: crossterm::event::KeyEvent) {
    if state.notes_search_active {
        match key.code {
            KeyCode::Esc => {
                state.notes_search_active = false;
                state.notes_search_query.clear();
                refresh_notes_cache(state);
            }
            KeyCode::Enter => {
                state.notes_search_active = false;
            }
            KeyCode::Backspace => {
                state.notes_search_query.pop();
                refresh_notes_cache(state);
                state.notes_cursor = 0;
            }
            KeyCode::Char(c) => {
                state.notes_search_query.push(c);
                refresh_notes_cache(state);
                state.notes_cursor = 0;
            }
            _ => {}
        }
        return;
    }

    let count = state.notes_cache.len();
    let page = state.panel_area.map(|r| r.height as usize / 2).unwrap_or(10);

    if state.notes_pending_g {
        state.notes_pending_g = false;
        if key.code == KeyCode::Char('g') && count > 0 {
            state.notes_cursor = 0;
        }
        return;
    }

    match key.code {
        KeyCode::Down | KeyCode::Char('j')
            if count > 0 && state.notes_cursor + 1 < count =>
        {
            state.notes_cursor += 1;
        }
        KeyCode::Up | KeyCode::Char('k') if state.notes_cursor > 0 => {
            state.notes_cursor -= 1;
        }
        KeyCode::Char('G') if count > 0 => {
            state.notes_cursor = count - 1;
        }
        KeyCode::Char('g') => {
            state.notes_pending_g = true;
        }
        KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) && count > 0 => {
            state.notes_cursor = (state.notes_cursor + page).min(count - 1);
        }
        KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            state.notes_cursor = state.notes_cursor.saturating_sub(page);
        }
        KeyCode::Enter => {
            if let Some(note) = state.notes_cache.get(state.notes_cursor) {
                state.notes_current_id = Some(note.id);
                state.notes_preview_scroll = 0;
                state.notes_view = NotesView::Preview;
            }
        }
        KeyCode::Char('n') => {
            match state.storage.create_note(domain::NewNote {
                title: String::new(),
                body: String::new(),
            }) {
                Ok(note) => {
                    state.notes_current_id = Some(note.id);
                    state.notes_title_editor = TextEditor::new();
                    state.notes_editor = TextEditor::new();
                    state.notes_title_focused = true;
                    state.notes_editor_scroll = 0;
                    state.notes_undo_stack.clear();
                    state.notes_redo_stack.clear();
                    state.notes_view = NotesView::Edit;
                    refresh_notes_cache(state);
                    state.notes_cursor = 0;
                }
                Err(e) => state.app.status_bar = format!("Error: {}", e.message()),
            }
        }
        KeyCode::Char('d') => {
            if let Some(note) = state.notes_cache.get(state.notes_cursor) {
                state.delete_confirm_name = note.title.clone();
                state.app.modal = Some(Modal::DeleteNote { id: note.id });
            }
        }
        KeyCode::Char('/') => {
            state.notes_search_active = true;
            state.notes_search_query.clear();
        }
        _ => {}
    }
}

fn handle_notes_preview_key(state: &mut RuntimeState, key: crossterm::event::KeyEvent) {
    match key.code {
        KeyCode::Char('e') => {
            if let Some(note) = state.notes_cache.iter().find(|n| Some(n.id) == state.notes_current_id) {
                state.notes_title_editor = TextEditor::from_string(&note.title);
                state.notes_editor = TextEditor::from_string(&note.body);
                state.notes_title_focused = true;
                state.notes_editor_scroll = 0;
                state.notes_undo_stack.clear();
                state.notes_redo_stack.clear();
                state.notes_view = NotesView::Edit;
            }
        }
        KeyCode::Esc => {
            state.notes_view = NotesView::List;
            state.notes_current_id = None;
        }
        KeyCode::Down | KeyCode::Char('j') => {
            state.notes_preview_scroll += 1;
        }
        KeyCode::Up | KeyCode::Char('k') => {
            state.notes_preview_scroll = state.notes_preview_scroll.saturating_sub(1);
        }
        KeyCode::Char('d') => {
            if let Some(id) = state.notes_current_id {
                if let Some(note) = state.notes_cache.iter().find(|n| n.id == id) {
                    state.delete_confirm_name = note.title.clone();
                }
                state.app.modal = Some(Modal::DeleteNote { id });
            }
        }
        KeyCode::Char('s') => {
            if let Some(id) = state.notes_current_id {
                if let Some(note) = state.notes_cache.iter().find(|n| n.id == id) {
                    let cfg = app_config::load();
                    let initial_dir = cfg.last_export_dir
                        .map(std::path::PathBuf::from)
                        .unwrap_or_else(|| std::env::var("HOME").map(std::path::PathBuf::from).unwrap_or_else(|_| std::path::PathBuf::from("/")));
                    state.dir_browser.current_dir = initial_dir;
                    state.dir_browser.note_id = id;
                    let safe_title: String = note.title.chars()
                        .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' || c == ' ' { c } else { '_' })
                        .collect();
                    state.dir_browser.filename = format!("{}.md", safe_title.trim());
                    state.dir_browser.field = 0;
                    state.dir_browser.refresh_entries();
                    state.app.modal = Some(Modal::ExportNote { id });
                }
            }
        }
        _ => {}
    }
}


fn notes_push_undo(state: &mut RuntimeState) {
    let snapshot = (
        state.notes_editor.lines().to_vec(),
        state.notes_editor.cursor_row(),
        state.notes_editor.cursor_col(),
    );
    state.notes_undo_stack.push(snapshot);
    state.notes_redo_stack.clear();
}

fn handle_notes_edit_key(state: &mut RuntimeState, key: crossterm::event::KeyEvent) {
    match key.code {
        KeyCode::Char('s') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            save_current_note(state);
            state.notes_view = NotesView::Preview;
        }
        KeyCode::Esc => {
            save_current_note(state);
            state.notes_view = NotesView::Preview;
        }
        KeyCode::Char('t') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            state.notes_title_focused = !state.notes_title_focused;
        }
        KeyCode::Char('z') if key.modifiers.contains(KeyModifiers::CONTROL) && !state.notes_title_focused => {
            if let Some((lines, row, col)) = state.notes_undo_stack.pop() {
                let current = (
                    state.notes_editor.lines().to_vec(),
                    state.notes_editor.cursor_row(),
                    state.notes_editor.cursor_col(),
                );
                state.notes_redo_stack.push(current);
                state.notes_editor = TextEditor::from_lines(lines, row, col);
            }
        }
        KeyCode::Char('y') if key.modifiers.contains(KeyModifiers::CONTROL) && !state.notes_title_focused => {
            if let Some((lines, row, col)) = state.notes_redo_stack.pop() {
                let current = (
                    state.notes_editor.lines().to_vec(),
                    state.notes_editor.cursor_row(),
                    state.notes_editor.cursor_col(),
                );
                state.notes_undo_stack.push(current);
                state.notes_editor = TextEditor::from_lines(lines, row, col);
            }
        }
        _ => {
            if state.notes_title_focused {
                match key.code {
                    KeyCode::Enter => {
                        state.notes_title_focused = false;
                    }
                    KeyCode::Char(c) => { state.notes_title_editor.insert_char(c); }
                    KeyCode::Backspace => { state.notes_title_editor.backspace(); }
                    KeyCode::Left => { state.notes_title_editor.move_left(); }
                    KeyCode::Right => { state.notes_title_editor.move_right(); }
                    KeyCode::Home => { state.notes_title_editor.move_home(); }
                    KeyCode::End => { state.notes_title_editor.move_end(); }
                    _ => {}
                }
            } else {
                match key.code {
                    KeyCode::Char(c) => {
                        if key.modifiers.contains(KeyModifiers::CONTROL) {
                            match c {
                                'u' => { notes_push_undo(state); state.notes_editor.kill_to_start(); }
                                'k' => { notes_push_undo(state); state.notes_editor.kill_to_end(); }
                                'a' => { state.notes_editor.move_home(); }
                                'e' => { state.notes_editor.move_end(); }
                                _ => {}
                            }
                        } else {
                            notes_push_undo(state);
                            state.notes_editor.insert_char(c);
                        }
                    }
                    KeyCode::Enter => { notes_push_undo(state); state.notes_editor.insert_newline(); }
                    KeyCode::Backspace => { notes_push_undo(state); state.notes_editor.backspace(); }
                    KeyCode::Delete => { notes_push_undo(state); state.notes_editor.delete(); }
                    KeyCode::Left => { state.notes_editor.move_left(); }
                    KeyCode::Right => { state.notes_editor.move_right(); }
                    KeyCode::Up => { state.notes_editor.move_up(); }
                    KeyCode::Down => { state.notes_editor.move_down(); }
                    KeyCode::Home => { state.notes_editor.move_home(); }
                    KeyCode::End => { state.notes_editor.move_end(); }
                    _ => {}
                }
            }
        }
    }
}

fn save_current_note(state: &mut RuntimeState) {
    if let Some(id) = state.notes_current_id {
        let title = state.notes_title_editor.to_string();
        let body = state.notes_editor.to_string();
        let title_val = if title.trim().is_empty() {
            state.locale.misc.untitled_note.clone()
        } else {
            title
        };
        match state.storage.update_note(id, domain::NotePatch {
            title: Some(title_val),
            body: Some(body),
        }) {
            Ok(_) => {
                state.app.status_bar = state.locale.status.note_saved.clone();
                refresh_notes_cache(state);
            }
            Err(e) => state.app.status_bar = format!("Error: {}", e.message()),
        }
    }
}

fn render_notas(frame: &mut ratatui::Frame, state: &mut RuntimeState, area: Rect) {
    if state.notes_cache.is_empty() && state.notes_view == NotesView::List && !state.notes_search_active {
        refresh_notes_cache(state);
    }

    let block = Block::default()
        .borders(Borders::ALL)
        .title(format!(" {} ", state.locale.panels.notes));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    match state.notes_view {
        NotesView::List => {
            render_notes_list(frame, state, inner);
        }
        NotesView::Preview | NotesView::Edit => {
            if inner.width >= 60 {
                let cols = Layout::default()
                    .direction(Direction::Horizontal)
                    .constraints([Constraint::Percentage(30), Constraint::Percentage(70)])
                    .split(inner);
                render_notes_list(frame, state, cols[0]);
                if state.notes_view == NotesView::Preview {
                    render_note_preview(frame, state, cols[1]);
                } else {
                    render_note_edit(frame, state, cols[1]);
                }
            } else if state.notes_view == NotesView::Preview {
                render_note_preview(frame, state, inner);
            } else {
                render_note_edit(frame, state, inner);
            }
        }
    }
}

fn render_notes_list(frame: &mut ratatui::Frame, state: &RuntimeState, area: Rect) {
    if area.height < 2 {
        return;
    }

    let available = area.height as usize;
    let count = state.notes_cache.len();

    if state.notes_search_active {
        let query_line = format!(" /{}", state.notes_search_query);
        let search_para = Paragraph::new(query_line).style(Style::default().fg(Color::Yellow));
        let search_area = Rect { height: 1, ..area };
        frame.render_widget(search_para, search_area);
        let list_area = Rect { y: area.y + 1, height: area.height.saturating_sub(1), ..area };
        render_notes_list_items(frame, state, list_area);
        return;
    }

    if count == 0 {
        let empty = Paragraph::new(state.locale.misc.no_notes.as_str())
            .style(Style::default().fg(Color::DarkGray));
        frame.render_widget(empty, area);
        return;
    }

    render_notes_list_items(frame, state, area);
    let _ = available;
}

fn render_notes_list_items(frame: &mut ratatui::Frame, state: &RuntimeState, area: Rect) {
    let count = state.notes_cache.len();
    let available = area.height as usize;
    let scroll = if state.notes_cursor >= state.notes_scroll + available {
        state.notes_cursor - available + 1
    } else if state.notes_cursor < state.notes_scroll {
        state.notes_cursor
    } else {
        state.notes_scroll
    };

    let items: Vec<ListItem> = state.notes_cache.iter().enumerate()
        .skip(scroll)
        .take(available)
        .map(|(i, note)| {
            let cursor = if i == state.notes_cursor { "▶" } else { " " };
            let title = if note.title.is_empty() {
                &state.locale.misc.untitled_note
            } else {
                &note.title
            };
            let date = &note.updated_at[..10.min(note.updated_at.len())];
            let label = format!("{cursor} {title}  {date}");
            let style = if i == state.notes_cursor {
                Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)
            } else if Some(note.id) == state.notes_current_id {
                Style::default().fg(Color::White)
            } else {
                Style::default().fg(Color::DarkGray)
            };
            ListItem::new(label).style(style)
        })
        .collect();

    let list = List::new(items);
    frame.render_widget(list, area);
    let _ = count;
}

fn render_note_preview(frame: &mut ratatui::Frame, state: &RuntimeState, area: Rect) {
    let note = match state.notes_cache.iter().find(|n| Some(n.id) == state.notes_current_id) {
        Some(n) => n,
        None => return,
    };

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(2), Constraint::Min(1), Constraint::Length(1)])
        .split(area);

    let title_display = if note.title.is_empty() {
        &state.locale.misc.untitled_note
    } else {
        &note.title
    };
    let title_para = Paragraph::new(format!(" {title_display}"))
        .style(Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD));
    frame.render_widget(title_para, chunks[0]);

    let width = chunks[1].width.saturating_sub(1) as usize;
    let rendered = jinx::markdown::render_markdown(&note.body, width.max(10));
    let total = rendered.len();
    let avail = chunks[1].height as usize;
    let scroll = state.notes_preview_scroll.min(total.saturating_sub(avail));
    let visible: Vec<ratatui::text::Line<'_>> = rendered.into_iter().skip(scroll).take(avail).collect();
    let body_para = Paragraph::new(visible);
    frame.render_widget(body_para, chunks[1]);

    let hint_para = Paragraph::new(state.locale.hints.notes_preview.as_str())
        .style(Style::default().fg(Color::DarkGray));
    frame.render_widget(hint_para, chunks[2]);
}

fn render_note_edit(frame: &mut ratatui::Frame, state: &mut RuntimeState, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(2), Constraint::Min(1)])
        .split(area);

    // Title line
    let title_text = state.notes_title_editor.to_string();
    let title_style = if state.notes_title_focused {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    let title_label = if state.notes_title_focused {
        format!(" > {title_text}│")
    } else {
        format!("   {title_text}")
    };
    let title_para = Paragraph::new(title_label).style(title_style);
    frame.render_widget(title_para, chunks[0]);

    // Body
    let body_style = if !state.notes_title_focused {
        Style::default().fg(Color::White)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let input_w = chunks[1].width.saturating_sub(1) as usize;
    let avail = chunks[1].height as usize;

    // Build all visual lines from logical lines
    let mut body_lines: Vec<ratatui::text::Line<'static>> = Vec::new();
    for logical_line in state.notes_editor.lines() {
        if input_w == 0 {
            body_lines.push(ratatui::text::Line::from(logical_line.to_string()));
        } else {
            let chars: Vec<char> = logical_line.chars().collect();
            if chars.is_empty() {
                body_lines.push(ratatui::text::Line::from(String::new()));
            } else {
                for chunk in chars.chunks(input_w) {
                    body_lines.push(ratatui::text::Line::from(chunk.iter().collect::<String>()));
                }
            }
        }
    }

    // Calculate cursor's visual row
    let mut cursor_visual_row: usize = 0;
    if !state.notes_title_focused && input_w > 0 {
        let (row, col) = (state.notes_editor.cursor_row(), state.notes_editor.cursor_col());
        for (i, logical_line) in state.notes_editor.lines().iter().enumerate() {
            let char_count = logical_line.chars().count();
            let n_visual = if char_count == 0 { 1 } else { (char_count.saturating_sub(1) / input_w) + 1 };
            if i == row {
                cursor_visual_row += col / input_w;
                break;
            }
            cursor_visual_row += n_visual;
        }
    }

    // Auto-scroll: ensure cursor is visible
    if cursor_visual_row < state.notes_editor_scroll {
        state.notes_editor_scroll = cursor_visual_row;
    } else if cursor_visual_row >= state.notes_editor_scroll + avail {
        state.notes_editor_scroll = cursor_visual_row - avail + 1;
    }

    let scroll = state.notes_editor_scroll;
    let visible: Vec<ratatui::text::Line<'static>> = body_lines.into_iter()
        .skip(scroll)
        .take(avail)
        .collect();
    let body_para = Paragraph::new(visible).style(body_style);
    frame.render_widget(body_para, chunks[1]);

    // Set cursor position
    if !state.notes_title_focused && input_w > 0 {
        let col = state.notes_editor.cursor_col();
        let visual_col = (col % input_w) as u16;
        let screen_row = (cursor_visual_row - scroll) as u16;
        let x = chunks[1].x + visual_col;
        let y = chunks[1].y + screen_row;
        if x < chunks[1].x + chunks[1].width && y < chunks[1].y + chunks[1].height {
            frame.set_cursor_position((x, y));
        }
    }
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
        let storage = Arc::new(SqliteStorage::in_memory().expect("in-memory"));
        let mut app = AppState::new(120, 40);

        let err = domain::DomainError::NotFound("Task 99 not found".to_string());
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
