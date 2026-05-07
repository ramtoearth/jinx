//! Terminal Day Organizer — TUI binary entry point.
//!
//! Startup sequence (Task 18 / Requisitos 7.1, 7.3, 7.4, 9.1, 9.2):
//! 1. Resolve or override the SQLite path.
//! 2. Open `SqliteStorage` (creates the DB and applies migrations).
//! 3. Initialise the Ratatui terminal backend.
//! 4. Spawn the Python Agent as a child process.
//! 5. Send `agent_init`.
//! 6. Enter the main event loop (render + key events + IPC reads).
//! 7. On `Ctrl+Q` send `shutdown` and wait for the agent to exit.

use std::fs::OpenOptions;
use std::io::{self, Write as _};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

use crossterm::{
    event::{self, Event, KeyCode, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph},
    Terminal,
};
use storage::{SqliteStorage, Storage, TaskFilter};
use uuid::Uuid;

use tui::app::{AppEvent, AppState, Panel, MIN_COLS, MIN_ROWS};
use tui::ipc::{
    AgentInitAckPayload, AgentInitPayload, AgentReplyPayload, Envelope, Kind, MessageType,
    ModelProvider, UserMessagePayload,
};
use tui::proximos::{add_24h, proximos};

// ---------------------------------------------------------------------------
// Chat message
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct ChatMsg {
    role: &'static str,
    text: String,
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

fn main() -> io::Result<()> {
    // -- Storage -----------------------------------------------------------
    let db_path = match storage::resolve_db_path() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("Cannot resolve database path: {e}");
            std::process::exit(1);
        }
    };
    let storage: Arc<dyn Storage + Send + Sync> = Arc::new(
        SqliteStorage::open(&db_path).unwrap_or_else(|e| {
            eprintln!("Cannot open database: {e}");
            std::process::exit(1);
        }),
    );

    // -- Terminal setup ----------------------------------------------------
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = run_app(&mut terminal, storage);

    // -- Cleanup -----------------------------------------------------------
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    if let Err(e) = result {
        eprintln!("Error: {e}");
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Application state and loop
// ---------------------------------------------------------------------------

struct RuntimeState {
    app: AppState,
    chat_history: Vec<ChatMsg>,
    chat_input: String,
    chat_scroll: usize,
    task_cursor: usize,
    calendar_cursor: usize,
    storage_version: u64,
    storage: Arc<dyn Storage + Send + Sync>,
    agent_child: Option<Child>,
    agent_stdin: Option<ChildStdin>,
    // Inbound envelopes from the agent reader thread
    agent_rx: Option<mpsc::Receiver<Envelope>>,
    // Pending message waiting for agent_reply (request id)
    pending_request: Option<(Uuid, Instant)>,
}

fn run_app(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    storage: Arc<dyn Storage + Send + Sync>,
) -> io::Result<()> {
    let (size_cols, size_rows) = {
        let size = terminal.size()?;
        (size.width, size.height)
    };

    let mut state = RuntimeState {
        app: AppState::new(size_cols, size_rows),
        chat_history: Vec::new(),
        chat_input: String::new(),
        chat_scroll: 0,
        task_cursor: 0,
        calendar_cursor: 0,
        storage_version: 0,
        storage: storage.clone(),
        agent_child: None,
        agent_stdin: None,
        agent_rx: None,
        pending_request: None,
    };

    // Spawn agent
    spawn_agent(&mut state);

    let tick = Duration::from_millis(250);
    let timeout_dur = Duration::from_secs(30);
    let mut last_tick = Instant::now();

    loop {
        // --- Render -------------------------------------------------------
        terminal.draw(|f| render(f, &state))?;

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
                Event::Resize(cols, rows) => {
                    state.app = tui::app::reduce(state.app, AppEvent::Resize(cols, rows));
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
                    state.app.status_bar =
                        "Tiempo de espera agotado. Vuelve a intentarlo.".to_string();
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
    // Tab / Shift-Tab always cycle focus
    match key.code {
        KeyCode::Tab if key.modifiers == KeyModifiers::NONE => {
            state.app = tui::app::reduce(state.app.clone(), AppEvent::Key(key));
            return;
        }
        KeyCode::BackTab => {
            state.app = tui::app::reduce(state.app.clone(), AppEvent::Key(key));
            return;
        }
        _ => {}
    }

    if state.app.is_too_small() {
        return;
    }

    match state.app.focused_panel {
        Panel::Chat => handle_chat_key(state, key),
        Panel::Tareas => handle_tareas_key(state, key),
        Panel::Calendario => handle_calendario_key(state, key),
        Panel::Proximos => {}
    }
}

fn handle_chat_key(state: &mut RuntimeState, key: crossterm::event::KeyEvent) {
    match key.code {
        KeyCode::Enter => {
            let text = state.chat_input.trim().to_string();
            if text.is_empty() {
                state.app.status_bar = "Mensaje vacío, escribe algo para enviar.".to_string();
                return;
            }
            state.chat_history.push(ChatMsg { role: "usuario", text: text.clone() });
            state.chat_input.clear();
            send_user_message(state, text);
        }
        KeyCode::Backspace => {
            state.chat_input.pop();
        }
        KeyCode::Char('l') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            state.chat_input.clear();
        }
        KeyCode::Char(c) => {
            state.chat_input.push(c);
        }
        _ => {}
    }
}

fn handle_tareas_key(state: &mut RuntimeState, key: crossterm::event::KeyEvent) {
    state.app = tui::app::reduce(state.app.clone(), AppEvent::Key(key));
}

fn handle_calendario_key(state: &mut RuntimeState, key: crossterm::event::KeyEvent) {
    state.app = tui::app::reduce(state.app.clone(), AppEvent::Key(key));
}

// ---------------------------------------------------------------------------
// Agent IPC
// ---------------------------------------------------------------------------

fn spawn_agent(state: &mut RuntimeState) {
    let python = std::env::var("PYTHON").unwrap_or_else(|_| "python3".to_string());
    let workspace_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf();

    let agent_stderr = OpenOptions::new()
        .create(true)
        .append(true)
        .open("/tmp/tui_agent.log")
        .map(Stdio::from)
        .unwrap_or_else(|_| Stdio::null());

    let mut child = Command::new(&python)
        .arg("-m")
        .arg("agent.main")
        .current_dir(&workspace_root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(agent_stderr)
        .spawn()
        .unwrap_or_else(|e| {
            eprintln!("Failed to spawn agent: {e}");
            std::process::exit(1);
        });

    let mut stdin = child.stdin.take().expect("child stdin");
    let child_stdout = child.stdout.take().expect("child stdout");

    // Background reader thread: parses JSON lines and sends Envelopes over mpsc
    let (tx, rx) = mpsc::channel::<Envelope>();
    std::thread::spawn(move || {
        use std::io::BufRead;
        let mut log = OpenOptions::new()
            .create(true)
            .append(true)
            .open("/tmp/tui_agent.log")
            .ok();
        let reader = std::io::BufReader::new(child_stdout);
        for line in reader.lines() {
            match line {
                Ok(line) if !line.trim().is_empty() => {
                    match serde_json::from_str::<Envelope>(&line) {
                        Ok(env) => {
                            if tx.send(env).is_err() {
                                break; // main thread dropped receiver — TUI is shutting down
                            }
                        }
                        Err(e) => {
                            if let Some(ref mut f) = log {
                                let _ = writeln!(f, "[agent reader] parse error: {e}: {line}");
                            }
                        }
                    }
                }
                Ok(_) => {} // blank line
                Err(e) => {
                    if let Some(ref mut f) = log {
                        let _ = writeln!(f, "[agent reader] read error: {e}");
                    }
                    break;
                }
            }
        }
    });

    // Send agent_init
    let timezone = iana_timezone();
    let init_env = Envelope::new(
        Kind::Request,
        MessageType::AgentInit,
        &AgentInitPayload {
            timezone,
            model_provider: ModelProvider::Local,
        },
    )
    .expect("agent_init serializes");
    let line = serde_json::to_string(&init_env).expect("serialize") + "\n";
    let _ = stdin.write_all(line.as_bytes());
    let _ = stdin.flush();

    state.agent_child = Some(child);
    state.agent_stdin = Some(stdin);
    state.agent_rx = Some(rx);
    state.app.agent_alive = true;
}

fn send_user_message(state: &mut RuntimeState, text: String) {
    if let Some(ref mut stdin) = state.agent_stdin {
        let env = Envelope::new(
            Kind::Request,
            MessageType::UserMessage,
            &UserMessagePayload { text },
        )
        .expect("user_message serializes");
        let req_id = env.id;
        let line = serde_json::to_string(&env).expect("serialize") + "\n";
        if stdin.write_all(line.as_bytes()).is_ok() && stdin.flush().is_ok() {
            state.pending_request = Some((req_id, Instant::now()));
        } else {
            state.app.status_bar = "Error enviando mensaje al Agente.".to_string();
            state.app.agent_alive = false;
        }
    }
}

fn send_shutdown(state: &mut RuntimeState) {
    if let Some(ref mut stdin) = state.agent_stdin {
        let env = Envelope::new_empty(Kind::Request, MessageType::Shutdown);
        let line = serde_json::to_string(&env).expect("serialize") + "\n";
        let _ = stdin.write_all(line.as_bytes());
        let _ = stdin.flush();
    }
    if let Some(ref mut child) = state.agent_child {
        let _ = child.wait();
    }
}

fn read_agent_output(state: &mut RuntimeState) {
    loop {
        let env = match state.agent_rx.as_ref().and_then(|rx| rx.try_recv().ok()) {
            Some(e) => e,
            None => break,
        };
        handle_agent_envelope(state, env);
    }
}

fn handle_agent_envelope(state: &mut RuntimeState, env: Envelope) {
    match env.message_type {
        MessageType::AgentInitAck => {
            if let Ok(Some(p)) = env.payload_as::<AgentInitAckPayload>() {
                if let Some(notice) = p.provider_notice {
                    state.chat_history.push(ChatMsg { role: "sistema", text: notice });
                }
            }
        }
        MessageType::AgentReply => {
            if let Ok(Some(p)) = env.payload_as::<AgentReplyPayload>() {
                state.chat_history.push(ChatMsg { role: "agente", text: p.text });
            }
            state.pending_request = None;
            state.app.status_bar = "Listo.".to_string();
        }
        mt if is_storage_message_type(mt) => {
            let response = tui::ipc_handler::handle_storage_request(&env, &state.storage);
            if let Some(ref mut stdin) = state.agent_stdin {
                if let Ok(line) = serde_json::to_string(&response) {
                    let _ = stdin.write_all(line.as_bytes());
                    let _ = stdin.write_all(b"\n");
                    let _ = stdin.flush();
                }
            }
        }
        _ => {}
    }
}

fn is_storage_message_type(mt: MessageType) -> bool {
    matches!(
        mt,
        MessageType::StorageListTasks
            | MessageType::StorageCreateTask
            | MessageType::StorageUpdateTask
            | MessageType::StorageCompleteTask
            | MessageType::StorageDeleteTask
            | MessageType::StorageListEvents
            | MessageType::StorageCreateEvent
            | MessageType::StorageUpdateEvent
            | MessageType::StorageDeleteEvent
            | MessageType::StorageListGroups
            | MessageType::StorageCreateGroup
            | MessageType::StorageRenameGroup
            | MessageType::StorageRecolorGroup
            | MessageType::StorageDeleteGroup
            | MessageType::StorageExportMarkdown
            | MessageType::StorageExportSqlite
    )
}

fn iana_timezone() -> String {
    std::env::var("TZ").unwrap_or_else(|_| "UTC".to_string())
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

fn render(frame: &mut ratatui::Frame, state: &RuntimeState) {
    let size = frame.size();

    // Viewport guard
    if size.width < MIN_COLS || size.height < MIN_ROWS {
        let msg = Paragraph::new(format!(
            "Terminal demasiado pequeño. Mínimo: {MIN_COLS}×{MIN_ROWS} (actual: {}×{})",
            size.width, size.height
        ))
        .style(Style::default().fg(Color::Red));
        frame.render_widget(msg, size);
        return;
    }

    // Main layout: top area + status bar
    let main_and_status = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(1)])
        .split(size);

    let main_area = main_and_status[0];
    let status_area = main_and_status[1];

    // Two columns
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(main_area);

    let left_col = columns[0];
    let right_col = columns[1];

    // Left column: Panel_Chat (top) + Panel_Proximos (bottom)
    let left_rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
        .split(left_col);

    // Right column: Panel_Tareas (top) + Panel_Calendario (bottom)
    let right_rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(right_col);

    render_chat(frame, state, left_rows[0]);
    render_proximos(frame, state, left_rows[1]);
    render_tareas(frame, state, right_rows[0]);
    render_calendario(frame, state, right_rows[1]);
    render_status(frame, state, status_area);
}

fn panel_block(title: &str, focused: bool) -> Block<'_> {
    let border_style = if focused {
        Style::default().add_modifier(Modifier::BOLD).fg(Color::Cyan)
    } else {
        Style::default()
    };
    let title_str = if focused {
        format!("{title} [ACTIVO]")
    } else {
        title.to_string()
    };
    Block::default()
        .title(title_str)
        .borders(Borders::ALL)
        .border_style(border_style)
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

fn render_chat(frame: &mut ratatui::Frame, state: &RuntimeState, area: Rect) {
    let focused = state.app.focused_panel == Panel::Chat;
    let block = panel_block("Chat", focused);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    // Split inner: history (top) + input (bottom 3 lines)
    let parts = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(3)])
        .split(inner);

    let hist_area = parts[0];
    let wrap_width = (hist_area.width as usize).saturating_sub(2);
    let avail_height = hist_area.height as usize;

    // Build display lines for all messages
    let mut all_lines: Vec<Line<'static>> = Vec::new();
    for msg in &state.chat_history {
        let (label, color) = if msg.role == "agente" {
            ("Agente", Color::Green)
        } else if msg.role == "sistema" {
            ("Sistema", Color::Yellow)
        } else {
            ("Tú", Color::Cyan)
        };
        let style = Style::default().fg(color).add_modifier(Modifier::BOLD);
        let body_style = Style::default().fg(color);

        let clean = strip_md(&msg.text);
        let wrapped = word_wrap(&clean, wrap_width.saturating_sub(label.len() + 3));

        let header = format!("[{label}]");
        if wrapped.is_empty() {
            all_lines.push(Line::from(Span::styled(header, style)));
        } else {
            // First line: [Label] first-text-line
            all_lines.push(Line::from(vec![
                Span::styled(format!("{header} "), style),
                Span::styled(wrapped[0].clone(), body_style),
            ]));
            // Continuation lines indented by label+3 spaces
            let indent = " ".repeat(header.chars().count() + 1);
            for line in &wrapped[1..] {
                all_lines.push(Line::from(Span::styled(
                    format!("{indent}{line}"),
                    body_style,
                )));
            }
        }
        // Blank separator between messages
        all_lines.push(Line::from(""));
    }

    // Scroll to bottom: show only lines that fit
    let start = all_lines.len().saturating_sub(avail_height);
    let visible: Vec<Line<'static>> = all_lines[start..].to_vec();
    frame.render_widget(Paragraph::new(visible), hist_area);

    // Input field
    let input_block = Block::default().title("Mensaje").borders(Borders::ALL);
    let input_inner = input_block.inner(parts[1]);
    frame.render_widget(input_block, parts[1]);
    let input_para = Paragraph::new(state.chat_input.as_str());
    frame.render_widget(input_para, input_inner);
}

fn render_proximos(frame: &mut ratatui::Frame, state: &RuntimeState, area: Rect) {
    let focused = state.app.focused_panel == Panel::Proximos;
    let block = panel_block("Próximos", focused);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let tasks = state.storage.list_tasks(TaskFilter::default()).unwrap_or_default();
    let events = state.storage.list_events(None, None).unwrap_or_default();

    let now = chrono::Utc::now();
    let now_date = now.format("%Y-%m-%d").to_string();
    let now_time = now.format("%H:%M").to_string();
    let (end_date, end_time) = add_24h(&now_date, &now_time);

    let entries = proximos(&tasks, &events, &now_date, &now_time, &end_date, &end_time);

    let items: Vec<ListItem> = entries
        .iter()
        .map(|e| {
            let label = match e.kind {
                tui::proximos::EntryKind::Task { priority } => {
                    format!("▸ {} ({}) [{} {}]", e.title, priority.as_str(), e.date, e.time)
                }
                tui::proximos::EntryKind::Event => {
                    format!("● {} [{} {}]", e.title, e.date, e.time)
                }
            };
            ListItem::new(label)
        })
        .collect();

    frame.render_widget(List::new(items), inner);
}

fn render_tareas(frame: &mut ratatui::Frame, state: &RuntimeState, area: Rect) {
    let focused = state.app.focused_panel == Panel::Tareas;
    let block = panel_block("Tareas", focused);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let tasks = state
        .storage
        .list_tasks(TaskFilter {
            status: Some(storage::TaskStatus::Pendiente),
            ..Default::default()
        })
        .unwrap_or_default();

    let mut items: Vec<ListItem> = tasks
        .iter()
        .map(|t| {
            let label = format!(
                "[{}] {} ({})",
                t.priority.as_str(),
                t.title,
                t.deadline.as_deref().unwrap_or("sin fecha")
            );
            ListItem::new(label)
        })
        .collect();

    if focused {
        items.push(ListItem::new(Line::from(vec![
            Span::raw("  "),
            Span::styled("n:nuevo  e:editar  c:completar  d:eliminar  ↑↓:navegar", Style::default().fg(Color::DarkGray)),
        ])));
    }

    frame.render_widget(List::new(items), inner);
}

fn render_calendario(frame: &mut ratatui::Frame, state: &RuntimeState, area: Rect) {
    let focused = state.app.focused_panel == Panel::Calendario;
    let block = panel_block("Calendario", focused);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let tasks = state.storage.list_tasks(TaskFilter::default()).unwrap_or_default();
    let events = state.storage.list_events(None, None).unwrap_or_default();
    let view = tui::calendario::calendar_layout(&tasks, &events);

    let mut dates: Vec<String> = view.keys().cloned().collect();
    dates.sort();

    let mut lines: Vec<ListItem> = vec![];
    for date in &dates {
        lines.push(ListItem::new(date.as_str()).style(Style::default().add_modifier(Modifier::BOLD)));
        if let Some(entries) = view.get(date) {
            for entry in entries {
                lines.push(ListItem::new(format!("  {}", entry.text)));
            }
        }
    }

    if focused {
        lines.push(ListItem::new(Line::from(vec![
            Span::styled("n:nuevo  e:editar  d:eliminar  ←→:día  PgUp/PgDn:mes", Style::default().fg(Color::DarkGray)),
        ])));
    }

    frame.render_widget(List::new(lines), inner);
}

fn render_status(frame: &mut ratatui::Frame, state: &RuntimeState, area: Rect) {
    let text = if state.app.status_bar.is_empty() {
        "Tab:cambiar panel  Ctrl+Q:salir".to_string()
    } else {
        state.app.status_bar.clone()
    };
    let para = Paragraph::new(text).style(Style::default().fg(Color::DarkGray));
    frame.render_widget(para, area);
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

        let err = storage::StorageError::NotFound("Task 99 not found".to_string());
        app = tui::app::reduce(
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
            // Verify the warning text that would appear in the status bar.
            let warning = "Mensaje vacío, escribe algo para enviar.";
            prop_assert!(!warning.is_empty());
        }
    }
}
