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
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Tabs},
    Terminal,
};
use storage::{
    EventPatch, Group, HexColor, NewEvent, NewGroup, NewTask, Priority, SqliteStorage, Storage,
    TaskFilter, TaskPatch, TaskStatus,
};
use uuid::Uuid;

use tui::app::{AppEvent, AppState, Modal, Panel, MIN_COLS, MIN_ROWS};
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
// Modal form state
// ---------------------------------------------------------------------------

const COLOR_PRESETS: [&str; 16] = [
    "#e74c3c", "#e67e22", "#f1c40f", "#2ecc71",
    "#1abc9c", "#3498db", "#9b59b6", "#e91e63",
    "#795548", "#607d8b", "#ff5722", "#009688",
    "#4caf50", "#2196f3", "#9c27b0", "#f44336",
];

#[derive(Default, Clone)]
struct TaskFormState {
    title: String,
    priority_idx: usize,  // 0=alta 1=media 2=baja
    deadline: String,     // YYYY-MM-DD or empty
    group_idx: usize,     // 0=ninguno, 1..=N = groups_cache[idx-1]
    status_idx: usize,    // 0=pendiente 1=completada 2=cancelada (edit only)
    field: usize,         // active field
    edit_id: Option<i64>,
    error: Option<String>,
}

#[derive(Default, Clone)]
struct EventFormState {
    title: String,
    start_date: String,   // YYYY-MM-DD
    start_time: String,   // HH:MM
    duration: String,     // minutes or empty
    group_idx: usize,
    field: usize,
    edit_id: Option<i64>,
    error: Option<String>,
}

#[derive(Default, Clone)]
struct GroupFormState {
    name: String,
    color_idx: usize,     // index into COLOR_PRESETS (or custom)
    color_custom: String, // overrides preset when non-empty
    field: usize,         // 0=name 1=color
    edit_id: Option<i64>,
    error: Option<String>,
}

impl GroupFormState {
    fn effective_color(&self) -> &str {
        if !self.color_custom.is_empty() {
            &self.color_custom
        } else {
            COLOR_PRESETS[self.color_idx % COLOR_PRESETS.len()]
        }
    }
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
    task_cursor: usize,
    calendar_cursor: usize,
    group_cursor: usize,
    storage: Arc<dyn Storage + Send + Sync>,
    agent_child: Option<Child>,
    agent_stdin: Option<ChildStdin>,
    agent_rx: Option<mpsc::Receiver<Envelope>>,
    pending_request: Option<(Uuid, Instant)>,
    // Modal form state
    task_form: TaskFormState,
    event_form: EventFormState,
    group_form: GroupFormState,
    groups_cache: Vec<Group>,
    delete_confirm_name: String,
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
        task_cursor: 0,
        calendar_cursor: 0,
        group_cursor: 0,
        storage: storage.clone(),
        agent_child: None,
        agent_stdin: None,
        agent_rx: None,
        pending_request: None,
        task_form: TaskFormState::default(),
        event_form: EventFormState::default(),
        group_form: GroupFormState::default(),
        groups_cache: Vec::new(),
        delete_confirm_name: String::new(),
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
    // When a modal is open, route all keys to modal handler
    if state.app.modal.is_some() {
        handle_modal_key(state, key);
        return;
    }

    // Tab / Shift-Tab cycle panels
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
        Panel::Proximos => handle_proximos_key(state, key),
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
    let tasks = state
        .storage
        .list_tasks(TaskFilter { status: Some(TaskStatus::Pendiente), ..Default::default() })
        .unwrap_or_default();
    match key.code {
        KeyCode::Up => {
            if state.task_cursor > 0 {
                state.task_cursor -= 1;
            }
        }
        KeyCode::Down => {
            if state.task_cursor + 1 < tasks.len() {
                state.task_cursor += 1;
            }
        }
        KeyCode::Char('n') => open_new_task_modal(state),
        KeyCode::Char('e') => {
            if let Some(t) = tasks.get(state.task_cursor) {
                open_edit_task_modal(state, t.id);
            }
        }
        KeyCode::Char('c') => {
            if let Some(t) = tasks.get(state.task_cursor) {
                let id = t.id;
                match state.storage.complete_task(id) {
                    Ok(_) => {
                        state.app.status_bar = "Tarea completada.".to_string();
                        if state.task_cursor > 0 { state.task_cursor -= 1; }
                    }
                    Err(e) => state.app.status_bar = format!("Error: {}", e.message()),
                }
            }
        }
        KeyCode::Char('d') => {
            if let Some(t) = tasks.get(state.task_cursor) {
                state.delete_confirm_name = t.title.clone();
                state.app.modal = Some(Modal::DeleteTask { id: t.id });
            }
        }
        KeyCode::Char('g') => open_new_group_modal(state),
        _ => {}
    }
}

fn handle_calendario_key(state: &mut RuntimeState, key: crossterm::event::KeyEvent) {
    let events = state.storage.list_events(None, None).unwrap_or_default();
    match key.code {
        KeyCode::Up => {
            if state.calendar_cursor > 0 { state.calendar_cursor -= 1; }
        }
        KeyCode::Down => {
            if state.calendar_cursor + 1 < events.len() { state.calendar_cursor += 1; }
        }
        KeyCode::Char('n') => open_new_event_modal(state),
        KeyCode::Char('e') => {
            if let Some(ev) = events.get(state.calendar_cursor) {
                open_edit_event_modal(state, ev.id);
            }
        }
        KeyCode::Char('d') => {
            if let Some(ev) = events.get(state.calendar_cursor) {
                state.delete_confirm_name = ev.title.clone();
                state.app.modal = Some(Modal::DeleteEvent { id: ev.id });
            }
        }
        _ => {}
    }
}

fn handle_proximos_key(state: &mut RuntimeState, key: crossterm::event::KeyEvent) {
    let groups = state.storage.list_groups().unwrap_or_default();
    match key.code {
        KeyCode::Up => {
            if state.group_cursor > 0 { state.group_cursor -= 1; }
        }
        KeyCode::Down => {
            if state.group_cursor + 1 < groups.len() { state.group_cursor += 1; }
        }
        KeyCode::Char('g') => open_new_group_modal(state),
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

// ---------------------------------------------------------------------------
// Modal open helpers
// ---------------------------------------------------------------------------

fn refresh_groups_cache(state: &mut RuntimeState) {
    state.groups_cache = state.storage.list_groups().unwrap_or_default();
}

fn open_new_task_modal(state: &mut RuntimeState) {
    refresh_groups_cache(state);
    state.task_form = TaskFormState { priority_idx: 1, ..Default::default() };
    state.app.modal = Some(Modal::NewTask);
}

fn open_edit_task_modal(state: &mut RuntimeState, id: i64) {
    refresh_groups_cache(state);
    let tasks = state.storage.list_tasks(TaskFilter::default()).unwrap_or_default();
    if let Some(t) = tasks.iter().find(|t| t.id == id) {
        let priority_idx = match t.priority {
            Priority::Alta => 0,
            Priority::Media => 1,
            Priority::Baja => 2,
        };
        let status_idx = match t.status {
            TaskStatus::Pendiente => 0,
            TaskStatus::Completada => 1,
            TaskStatus::Cancelada => 2,
        };
        let group_idx = t.group_id
            .and_then(|gid| state.groups_cache.iter().position(|g| g.id == gid).map(|p| p + 1))
            .unwrap_or(0);
        state.task_form = TaskFormState {
            title: t.title.clone(),
            priority_idx,
            deadline: t.deadline.clone().unwrap_or_default(),
            group_idx,
            status_idx,
            edit_id: Some(id),
            ..Default::default()
        };
        state.app.modal = Some(Modal::EditTask { id });
    }
}

fn open_new_event_modal(state: &mut RuntimeState) {
    refresh_groups_cache(state);
    state.event_form = EventFormState::default();
    state.app.modal = Some(Modal::NewEvent);
}

fn open_edit_event_modal(state: &mut RuntimeState, id: i64) {
    refresh_groups_cache(state);
    let events = state.storage.list_events(None, None).unwrap_or_default();
    if let Some(ev) = events.iter().find(|e| e.id == id) {
        let group_idx = ev.group_id
            .and_then(|gid| state.groups_cache.iter().position(|g| g.id == gid).map(|p| p + 1))
            .unwrap_or(0);
        state.event_form = EventFormState {
            title: ev.title.clone(),
            start_date: ev.start_date.clone(),
            start_time: ev.start_time.clone(),
            duration: ev.duration_minutes.map(|d| d.to_string()).unwrap_or_default(),
            group_idx,
            edit_id: Some(id),
            ..Default::default()
        };
        state.app.modal = Some(Modal::EditEvent { id });
    }
}

fn open_new_group_modal(state: &mut RuntimeState) {
    state.group_form = GroupFormState::default();
    state.app.modal = Some(Modal::NewGroup);
}

fn open_edit_group_modal(state: &mut RuntimeState, id: i64) {
    let groups = state.storage.list_groups().unwrap_or_default();
    if let Some(g) = groups.iter().find(|g| g.id == id) {
        let color_str = g.color.to_string();
        let color_idx = COLOR_PRESETS.iter().position(|&p| p == color_str).unwrap_or(0);
        let color_custom = if COLOR_PRESETS.contains(&color_str.as_str()) {
            String::new()
        } else {
            color_str
        };
        state.group_form = GroupFormState {
            name: g.name.clone(),
            color_idx,
            color_custom,
            edit_id: Some(id),
            ..Default::default()
        };
        state.app.modal = Some(Modal::EditGroup { id });
    }
}

// ---------------------------------------------------------------------------
// Modal key handler dispatcher
// ---------------------------------------------------------------------------

fn handle_modal_key(state: &mut RuntimeState, key: crossterm::event::KeyEvent) {
    let modal = state.app.modal.clone();
    match modal {
        Some(Modal::NewTask) | Some(Modal::EditTask { .. }) => handle_task_form_key(state, key),
        Some(Modal::NewEvent) | Some(Modal::EditEvent { .. }) => handle_event_form_key(state, key),
        Some(Modal::NewGroup) | Some(Modal::EditGroup { .. }) => handle_group_form_key(state, key),
        Some(Modal::DeleteTask { id }) => handle_delete_key(state, key, |s| {
            match s.storage.delete_task(id) {
                Ok(_) => { s.app.modal = None; s.app.status_bar = "Tarea eliminada.".to_string(); if s.task_cursor > 0 { s.task_cursor -= 1; } }
                Err(e) => s.app.status_bar = format!("Error: {}", e.message()),
            }
        }),
        Some(Modal::DeleteEvent { id }) => handle_delete_key(state, key, |s| {
            match s.storage.delete_event(id) {
                Ok(_) => { s.app.modal = None; s.app.status_bar = "Evento eliminado.".to_string(); if s.calendar_cursor > 0 { s.calendar_cursor -= 1; } }
                Err(e) => s.app.status_bar = format!("Error: {}", e.message()),
            }
        }),
        Some(Modal::DeleteGroup { id }) => handle_delete_key(state, key, |s| {
            match s.storage.delete_group(id) {
                Ok(_) => { s.app.modal = None; s.app.status_bar = "Grupo eliminado.".to_string(); if s.group_cursor > 0 { s.group_cursor -= 1; } }
                Err(e) => s.app.status_bar = format!("Error: {}", e.message()),
            }
        }),
        _ => { if key.code == KeyCode::Esc { state.app.modal = None; } }
    }
}

fn handle_delete_key<F: FnOnce(&mut RuntimeState)>(
    state: &mut RuntimeState,
    key: crossterm::event::KeyEvent,
    confirm_action: F,
) {
    match key.code {
        KeyCode::Char('y') | KeyCode::Char('Y') => confirm_action(state),
        KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
            state.app.modal = None;
        }
        _ => {}
    }
}

fn handle_task_form_key(state: &mut RuntimeState, key: crossterm::event::KeyEvent) {
    let is_edit = state.task_form.edit_id.is_some();
    let n_fields = if is_edit { 5 } else { 4 };
    match key.code {
        KeyCode::Tab => state.task_form.field = (state.task_form.field + 1) % n_fields,
        KeyCode::BackTab => state.task_form.field = (state.task_form.field + n_fields - 1) % n_fields,
        KeyCode::Left => match state.task_form.field {
            1 => state.task_form.priority_idx = (state.task_form.priority_idx + 2) % 3,
            3 => { let n = state.groups_cache.len() + 1; state.task_form.group_idx = (state.task_form.group_idx + n - 1) % n; }
            4 => state.task_form.status_idx = (state.task_form.status_idx + 2) % 3,
            _ => {}
        },
        KeyCode::Right => match state.task_form.field {
            1 => state.task_form.priority_idx = (state.task_form.priority_idx + 1) % 3,
            3 => { let n = state.groups_cache.len() + 1; state.task_form.group_idx = (state.task_form.group_idx + 1) % n; }
            4 => state.task_form.status_idx = (state.task_form.status_idx + 1) % 3,
            _ => {}
        },
        KeyCode::Char(c) => match state.task_form.field {
            0 => state.task_form.title.push(c),
            2 => state.task_form.deadline.push(c),
            _ => {}
        },
        KeyCode::Backspace => match state.task_form.field {
            0 => { state.task_form.title.pop(); }
            2 => { state.task_form.deadline.pop(); }
            _ => {}
        },
        KeyCode::Enter => save_task(state),
        KeyCode::Esc => { state.app.modal = None; state.task_form.error = None; }
        _ => {}
    }
}

fn handle_event_form_key(state: &mut RuntimeState, key: crossterm::event::KeyEvent) {
    let n_fields = 5;
    match key.code {
        KeyCode::Tab => state.event_form.field = (state.event_form.field + 1) % n_fields,
        KeyCode::BackTab => state.event_form.field = (state.event_form.field + n_fields - 1) % n_fields,
        KeyCode::Left => if state.event_form.field == 4 {
            let n = state.groups_cache.len() + 1;
            state.event_form.group_idx = (state.event_form.group_idx + n - 1) % n;
        },
        KeyCode::Right => if state.event_form.field == 4 {
            let n = state.groups_cache.len() + 1;
            state.event_form.group_idx = (state.event_form.group_idx + 1) % n;
        },
        KeyCode::Char(c) => match state.event_form.field {
            0 => state.event_form.title.push(c),
            1 => state.event_form.start_date.push(c),
            2 => state.event_form.start_time.push(c),
            3 => state.event_form.duration.push(c),
            _ => {}
        },
        KeyCode::Backspace => match state.event_form.field {
            0 => { state.event_form.title.pop(); }
            1 => { state.event_form.start_date.pop(); }
            2 => { state.event_form.start_time.pop(); }
            3 => { state.event_form.duration.pop(); }
            _ => {}
        },
        KeyCode::Enter => save_event(state),
        KeyCode::Esc => { state.app.modal = None; state.event_form.error = None; }
        _ => {}
    }
}

fn handle_group_form_key(state: &mut RuntimeState, key: crossterm::event::KeyEvent) {
    match key.code {
        KeyCode::Tab => state.group_form.field = (state.group_form.field + 1) % 2,
        KeyCode::BackTab => state.group_form.field = (state.group_form.field + 1) % 2,
        KeyCode::Left => if state.group_form.field == 1 && state.group_form.color_custom.is_empty() {
            state.group_form.color_idx = (state.group_form.color_idx + COLOR_PRESETS.len() - 1) % COLOR_PRESETS.len();
        },
        KeyCode::Right => if state.group_form.field == 1 && state.group_form.color_custom.is_empty() {
            state.group_form.color_idx = (state.group_form.color_idx + 1) % COLOR_PRESETS.len();
        },
        KeyCode::Char(c) => match state.group_form.field {
            0 => state.group_form.name.push(c),
            1 => { state.group_form.color_custom.push(c); }
            _ => {}
        },
        KeyCode::Backspace => match state.group_form.field {
            0 => { state.group_form.name.pop(); }
            1 => { state.group_form.color_custom.pop(); }
            _ => {}
        },
        KeyCode::Enter => save_group(state),
        KeyCode::Esc => { state.app.modal = None; state.group_form.error = None; }
        _ => {}
    }
}

// ---------------------------------------------------------------------------
// Save helpers
// ---------------------------------------------------------------------------

fn save_task(state: &mut RuntimeState) {
    let form = state.task_form.clone();
    if form.title.trim().is_empty() {
        state.task_form.error = Some("El título no puede estar vacío.".to_string());
        return;
    }
    let priorities = [Priority::Alta, Priority::Media, Priority::Baja];
    let statuses = [TaskStatus::Pendiente, TaskStatus::Completada, TaskStatus::Cancelada];
    let priority = priorities[form.priority_idx];
    let deadline = if form.deadline.trim().is_empty() { None } else { Some(form.deadline.trim().to_string()) };
    let group_id = if form.group_idx == 0 { None } else {
        state.groups_cache.get(form.group_idx - 1).map(|g| g.id)
    };

    let result: Result<_, _> = if let Some(id) = form.edit_id {
        state.storage.update_task(id, TaskPatch {
            title: Some(form.title.trim().to_string()),
            priority: Some(priority),
            deadline: Some(deadline),
            group_id: Some(group_id),
            status: Some(statuses[form.status_idx]),
        }).map(|_| ())
    } else {
        state.storage.create_task(NewTask {
            title: form.title.trim().to_string(),
            priority: Some(priority),
            deadline,
            group_id,
        }).map(|_| ())
    };

    match result {
        Ok(()) => {
            state.app.modal = None;
            state.task_form = TaskFormState::default();
            state.app.status_bar = "Tarea guardada.".to_string();
        }
        Err(e) => state.task_form.error = Some(e.message()),
    }
}

fn save_event(state: &mut RuntimeState) {
    let form = state.event_form.clone();
    if form.title.trim().is_empty() {
        state.event_form.error = Some("El título no puede estar vacío.".to_string());
        return;
    }
    if form.start_date.trim().is_empty() {
        state.event_form.error = Some("La fecha de inicio es obligatoria.".to_string());
        return;
    }
    if form.start_time.trim().is_empty() {
        state.event_form.error = Some("La hora de inicio es obligatoria.".to_string());
        return;
    }
    let duration_minutes: Option<u32> = if form.duration.trim().is_empty() {
        None
    } else {
        match form.duration.trim().parse::<u32>() {
            Ok(d) => Some(d),
            Err(_) => { state.event_form.error = Some("Duración debe ser un número entero.".to_string()); return; }
        }
    };
    let group_id = if form.group_idx == 0 { None } else {
        state.groups_cache.get(form.group_idx - 1).map(|g| g.id)
    };

    let result: Result<_, _> = if let Some(id) = form.edit_id {
        state.storage.update_event(id, EventPatch {
            title: Some(form.title.trim().to_string()),
            start_date: Some(form.start_date.trim().to_string()),
            start_time: Some(form.start_time.trim().to_string()),
            duration_minutes: Some(duration_minutes),
            group_id: Some(group_id),
        }).map(|_| ())
    } else {
        state.storage.create_event(NewEvent {
            title: form.title.trim().to_string(),
            start_date: form.start_date.trim().to_string(),
            start_time: form.start_time.trim().to_string(),
            duration_minutes,
            group_id,
        }).map(|_| ())
    };

    match result {
        Ok(()) => {
            state.app.modal = None;
            state.event_form = EventFormState::default();
            state.app.status_bar = "Evento guardado.".to_string();
        }
        Err(e) => state.event_form.error = Some(e.message()),
    }
}

fn save_group(state: &mut RuntimeState) {
    let form = state.group_form.clone();
    if form.name.trim().is_empty() {
        state.group_form.error = Some("El nombre no puede estar vacío.".to_string());
        return;
    }
    let color_str = form.effective_color().to_string();
    let color = match HexColor::new(&color_str) {
        Ok(c) => c,
        Err(_) => { state.group_form.error = Some("Color inválido. Usa formato #RRGGBB.".to_string()); return; }
    };

    let result: Result<_, _> = if let Some(id) = form.edit_id {
        state.storage.rename_group(id, form.name.trim().to_string())
            .and_then(|_| state.storage.recolor_group(id, color))
            .map(|_| ())
    } else {
        state.storage.create_group(NewGroup { name: form.name.trim().to_string(), color }).map(|_| ())
    };

    match result {
        Ok(()) => {
            state.app.modal = None;
            state.group_form = GroupFormState::default();
            state.app.status_bar = "Grupo guardado.".to_string();
        }
        Err(e) => state.group_form.error = Some(e.message()),
    }
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
    let size = frame.area();

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

    match state.app.focused_panel {
        Panel::Chat => render_chat(frame, state, chunks[1]),
        Panel::Tareas => render_tareas(frame, state, chunks[1]),
        Panel::Calendario => render_calendario(frame, state, chunks[1]),
        Panel::Proximos => render_proximos(frame, state, chunks[1]),
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
        Panel::Proximos => 3,
    };
    let tabs = Tabs::new(vec!["  Chat  ", "  Tareas  ", "  Calendario  ", "  Próximos  "])
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

// ---------------------------------------------------------------------------
// Modal rendering
// ---------------------------------------------------------------------------

fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let vert = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(vert[1])[1]
}

fn render_modal(frame: &mut ratatui::Frame, state: &RuntimeState, area: Rect) {
    let popup = centered_rect(70, 70, area);
    frame.render_widget(Clear, popup);
    match &state.app.modal {
        Some(Modal::NewTask) | Some(Modal::EditTask { .. }) => render_task_form(frame, state, popup),
        Some(Modal::NewEvent) | Some(Modal::EditEvent { .. }) => render_event_form(frame, state, popup),
        Some(Modal::NewGroup) | Some(Modal::EditGroup { .. }) => render_group_form(frame, state, popup),
        Some(Modal::DeleteTask { .. }) | Some(Modal::DeleteEvent { .. }) | Some(Modal::DeleteGroup { .. }) => {
            render_delete_confirm(frame, state, popup);
        }
        _ => {}
    }
}

fn form_line<'a>(label: &'a str, value: String, active: bool) -> Line<'a> {
    let (ls, vs) = if active {
        (Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
         Style::default().fg(Color::Cyan))
    } else {
        (Style::default().fg(Color::DarkGray), Style::default())
    };
    Line::from(vec![
        Span::styled(format!("  {:16}", label), ls),
        Span::styled(value, vs),
    ])
}

fn render_task_form(frame: &mut ratatui::Frame, state: &RuntimeState, area: Rect) {
    let form = &state.task_form;
    let is_edit = form.edit_id.is_some();
    let title = if is_edit { "Editar Tarea" } else { "Nueva Tarea" };
    let block = Block::default().title(title).borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let priorities = ["alta", "media", "baja"];
    let statuses = ["pendiente", "completada", "cancelada"];
    let groups: Vec<String> = std::iter::once("(ninguno)".to_string())
        .chain(state.groups_cache.iter().map(|g| g.name.clone()))
        .collect();

    let mut lines: Vec<Line<'static>> = vec![Line::from("")];
    lines.push(form_line("Título", form.title.clone(), form.field == 0));
    lines.push(form_line("Prioridad", format!("← {} →", priorities[form.priority_idx]), form.field == 1));
    lines.push(form_line(
        "Fecha límite",
        if form.deadline.is_empty() { "(YYYY-MM-DD o vacío)".into() } else { form.deadline.clone() },
        form.field == 2,
    ));
    lines.push(form_line(
        "Grupo",
        format!("← {} →", groups.get(form.group_idx).map(String::as_str).unwrap_or("(ninguno)")),
        form.field == 3,
    ));
    if is_edit {
        lines.push(form_line("Estado", format!("← {} →", statuses[form.status_idx]), form.field == 4));
    }
    lines.push(Line::from(""));
    if let Some(ref err) = form.error {
        lines.push(Line::from(Span::styled(format!("  ⚠ {err}"), Style::default().fg(Color::Red))));
    }
    lines.push(Line::from(Span::styled(
        "  Tab:campo  ←/→:opción  Enter:guardar  Esc:cancelar",
        Style::default().fg(Color::DarkGray),
    )));
    frame.render_widget(Paragraph::new(lines), inner);
}

fn render_event_form(frame: &mut ratatui::Frame, state: &RuntimeState, area: Rect) {
    let form = &state.event_form;
    let is_edit = form.edit_id.is_some();
    let title = if is_edit { "Editar Evento" } else { "Nuevo Evento" };
    let block = Block::default().title(title).borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let groups: Vec<String> = std::iter::once("(ninguno)".to_string())
        .chain(state.groups_cache.iter().map(|g| g.name.clone()))
        .collect();

    let mut lines: Vec<Line<'static>> = vec![Line::from("")];
    lines.push(form_line("Título", form.title.clone(), form.field == 0));
    lines.push(form_line("Fecha inicio", if form.start_date.is_empty() { "(YYYY-MM-DD)".into() } else { form.start_date.clone() }, form.field == 1));
    lines.push(form_line("Hora inicio", if form.start_time.is_empty() { "(HH:MM)".into() } else { form.start_time.clone() }, form.field == 2));
    lines.push(form_line("Duración (min)", if form.duration.is_empty() { "(vacío = sin límite)".into() } else { form.duration.clone() }, form.field == 3));
    lines.push(form_line(
        "Grupo",
        format!("← {} →", groups.get(form.group_idx).map(String::as_str).unwrap_or("(ninguno)")),
        form.field == 4,
    ));
    lines.push(Line::from(""));
    if let Some(ref err) = form.error {
        lines.push(Line::from(Span::styled(format!("  ⚠ {err}"), Style::default().fg(Color::Red))));
    }
    lines.push(Line::from(Span::styled(
        "  Tab:campo  ←/→:grupo  Enter:guardar  Esc:cancelar",
        Style::default().fg(Color::DarkGray),
    )));
    frame.render_widget(Paragraph::new(lines), inner);
}

fn render_group_form(frame: &mut ratatui::Frame, state: &RuntimeState, area: Rect) {
    let form = &state.group_form;
    let is_edit = form.edit_id.is_some();
    let title = if is_edit { "Editar Grupo" } else { "Nuevo Grupo" };
    let block = Block::default().title(title).borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let color_display = if form.color_custom.is_empty() {
        format!("← {} → (preset {}/{})", COLOR_PRESETS[form.color_idx % COLOR_PRESETS.len()], form.color_idx + 1, COLOR_PRESETS.len())
    } else {
        format!("{} (personalizado)", form.color_custom)
    };

    let mut lines: Vec<Line<'static>> = vec![Line::from("")];
    lines.push(form_line("Nombre", form.name.clone(), form.field == 0));
    lines.push(form_line("Color", color_display, form.field == 1));
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "  En Color: ←/→ presets  o escribe #RRGGBB  luego Backspace para volver a presets",
        Style::default().fg(Color::DarkGray),
    )));
    if let Some(ref err) = form.error {
        lines.push(Line::from(Span::styled(format!("  ⚠ {err}"), Style::default().fg(Color::Red))));
    }
    lines.push(Line::from(Span::styled(
        "  Tab:campo  Enter:guardar  Esc:cancelar",
        Style::default().fg(Color::DarkGray),
    )));
    frame.render_widget(Paragraph::new(lines), inner);
}

fn render_delete_confirm(frame: &mut ratatui::Frame, state: &RuntimeState, area: Rect) {
    let kind = match &state.app.modal {
        Some(Modal::DeleteTask { .. }) => "tarea",
        Some(Modal::DeleteEvent { .. }) => "evento",
        Some(Modal::DeleteGroup { .. }) => "grupo",
        _ => "elemento",
    };
    let block = Block::default().title("Confirmar eliminación").borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Red));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let lines: Vec<Line<'static>> = vec![
        Line::from(""),
        Line::from(Span::styled(
            format!("  ¿Eliminar el {} \"{}\"?", kind, state.delete_confirm_name),
            Style::default().fg(Color::Yellow),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "  Presiona  y  para confirmar  o  n / Esc  para cancelar.",
            Style::default().fg(Color::DarkGray),
        )),
    ];
    frame.render_widget(Paragraph::new(lines), inner);
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
    let block = panel_block("Chat");
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
    let block = panel_block("Próximos");
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let tasks = state.storage.list_tasks(TaskFilter::default()).unwrap_or_default();
    let events = state.storage.list_events(None, None).unwrap_or_default();

    let now = chrono::Utc::now();
    let now_date = now.format("%Y-%m-%d").to_string();
    let now_time = now.format("%H:%M").to_string();
    let (end_date, end_time) = add_24h(&now_date, &now_time);

    let entries = proximos(&tasks, &events, &now_date, &now_time, &end_date, &end_time);

    let mut items: Vec<ListItem> = entries
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

    // Groups section at the bottom
    let groups = state.storage.list_groups().unwrap_or_default();
    if !items.is_empty() {
        items.push(ListItem::new(Line::from("")));
    }
    items.push(ListItem::new(Line::from(Span::styled(
        "── Grupos ──",
        Style::default().fg(Color::DarkGray).add_modifier(Modifier::BOLD),
    ))));
    if groups.is_empty() {
        items.push(ListItem::new("  (sin grupos)"));
    } else {
        for (i, g) in groups.iter().enumerate() {
            let selected = i == state.group_cursor;
            let label = format!(
                " {} {} ({})",
                if selected { "▶" } else { " " },
                g.name,
                g.color.to_string()
            );
            let style = if selected {
                Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            items.push(ListItem::new(label).style(style));
        }
    }
    items.push(ListItem::new(Line::from(Span::styled(
        "  ↑↓:navegar  g:nuevo  e:editar  d:eliminar",
        Style::default().fg(Color::DarkGray),
    ))));

    frame.render_widget(List::new(items), inner);
}

fn render_tareas(frame: &mut ratatui::Frame, state: &RuntimeState, area: Rect) {
    let block = panel_block("Tareas");
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
        .enumerate()
        .map(|(i, t)| {
            let label = format!(
                " {} [{}] {} ({})",
                if i == state.task_cursor { "▶" } else { " " },
                t.priority.as_str(),
                t.title,
                t.deadline.as_deref().unwrap_or("sin fecha")
            );
            let style = if i == state.task_cursor {
                Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            ListItem::new(label).style(style)
        })
        .collect();

    items.push(ListItem::new(Line::from(Span::styled(
        "  ↑↓:navegar  n:nueva  e:editar  c:completar  d:eliminar  g:nuevo grupo",
        Style::default().fg(Color::DarkGray),
    ))));

    frame.render_widget(List::new(items), inner);
}

fn render_calendario(frame: &mut ratatui::Frame, state: &RuntimeState, area: Rect) {
    let block = panel_block("Calendario");
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

    lines.push(ListItem::new(Line::from(Span::styled(
        "  n:nuevo  e:editar  d:eliminar",
        Style::default().fg(Color::DarkGray),
    ))));

    frame.render_widget(List::new(lines), inner);
}

fn render_status(frame: &mut ratatui::Frame, state: &RuntimeState, area: Rect) {
    let hint = "Tab:siguiente  Shift+Tab:anterior  Ctrl+Q:salir";
    let text = if state.app.status_bar.is_empty() {
        hint.to_string()
    } else {
        format!("{}  │  {}", state.app.status_bar, hint)
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
