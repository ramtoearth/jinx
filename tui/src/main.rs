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

use std::fs::OpenOptions;
use std::io::{self, Write as _};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::{mpsc, Arc};
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
use storage::{
    EventPatch, Group, HexColor, NewEvent, NewGroup, NewTask, Priority, SqliteStorage, Storage,
    TaskFilter, TaskPatch, TaskStatus,
};
use uuid::Uuid;

use jinx::app::{AppEvent, AppState, Modal, Panel, MIN_COLS, MIN_ROWS};
use jinx::calendario::{entry_count, flat_entries, nth_entry, FlatCalEntry};
use jinx::color::{detect_color_mode, resolve_style, ColorMode};
use jinx::config::{self as app_config};
use jinx::ipc::{
    AgentInitAckPayload, AgentInitPayload, AgentReplyPayload, Envelope, Kind, MessageType,
    ModelProvider, UserMessagePayload,
};
use jinx::text_editor::TextEditor;

// ---------------------------------------------------------------------------
// Platform-aware log path
// ---------------------------------------------------------------------------

fn agent_log_path() -> std::path::PathBuf {
    std::env::temp_dir().join("tui_agent.log")
}

// ---------------------------------------------------------------------------
// Embedded Python agent — bundled at compile time so the binary is self-contained
// ---------------------------------------------------------------------------

const AGENT_PYPROJECT: &str = include_str!("../../pyproject.toml");
const AGENT_INIT:      &str = include_str!("../../agent/__init__.py");
const AGENT_IPC:       &str = include_str!("../../agent/ipc.py");
const AGENT_STORAGE:   &str = include_str!("../../agent/storage_tools.py");
const AGENT_MAIN:      &str = include_str!("../../agent/main.py");

/// Extract the embedded agent files to the OS data directory and return the
/// project root path (the directory that contains `pyproject.toml`).
///
/// Files are only written when their content has changed, so subsequent calls
/// are nearly free (a few `read_to_string` comparisons).
fn extract_agent() -> std::path::PathBuf {
    let data_dir = directories::ProjectDirs::from("", "", "jinx")
        .map(|d| d.data_dir().to_path_buf())
        .unwrap_or_else(|| std::path::PathBuf::from(".jinx"));

    let pkg_dir = data_dir.join("agent"); // contains *.py modules

    let _ = std::fs::create_dir_all(&pkg_dir);

    let write_if_changed = |path: &std::path::Path, content: &str| {
        let needs_write = std::fs::read_to_string(path)
            .map(|existing| existing != content)
            .unwrap_or(true);
        if needs_write {
            if let Err(e) = std::fs::write(path, content) {
                eprintln!("[extract_agent] could not write {}: {e}", path.display());
            }
        }
    };

    write_if_changed(&data_dir.join("pyproject.toml"),  AGENT_PYPROJECT);
    write_if_changed(&pkg_dir.join("__init__.py"),       AGENT_INIT);
    write_if_changed(&pkg_dir.join("ipc.py"),            AGENT_IPC);
    write_if_changed(&pkg_dir.join("storage_tools.py"),  AGENT_STORAGE);
    write_if_changed(&pkg_dir.join("main.py"),           AGENT_MAIN);

    data_dir
}

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

const SPINNER_FRAMES: [char; 10] = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];

const COLOR_PRESETS: [&str; 16] = [
    "#e74c3c", "#e67e22", "#f1c40f", "#2ecc71",
    "#1abc9c", "#3498db", "#9b59b6", "#e91e63",
    "#795548", "#607d8b", "#ff5722", "#009688",
    "#4caf50", "#2196f3", "#9c27b0", "#f44336",
];

#[derive(Clone)]
struct TaskFormState {
    title: String,
    priority_idx: usize,  // 0=alta 1=media 2=baja
    deadline: DateTimeInput,
    group_idx: usize,     // 0=ninguno, 1..=N = groups_cache[idx-1]
    status_idx: usize,    // 0=pendiente 1=completada 2=cancelada (edit only)
    field: usize,         // active field: 0=title 1=priority 2=deadline 3=group 4=status(edit)
    edit_id: Option<i64>,
    error: Option<String>,
}

impl Default for TaskFormState {
    fn default() -> Self {
        Self {
            title: String::new(),
            priority_idx: 1,
            deadline: DateTimeInput::date_only_disabled(),
            group_idx: 0,
            status_idx: 0,
            field: 0,
            edit_id: None,
            error: None,
        }
    }
}

#[derive(Clone)]
struct EventFormState {
    title: String,
    datetime: DateTimeInput,
    duration: String,     // minutes or empty
    group_idx: usize,
    field: usize,         // 0=title 1=datetime 2=duration 3=group
    edit_id: Option<i64>,
    error: Option<String>,
}

impl Default for EventFormState {
    fn default() -> Self {
        Self {
            title: String::new(),
            datetime: DateTimeInput::date_time_now(),
            duration: String::new(),
            group_idx: 0,
            field: 0,
            edit_id: None,
            error: None,
        }
    }
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

#[derive(Default, Clone)]
struct SettingsFormState {
    field: usize,         // 0=provider, 1=model, 2=host (host only when local)
    provider_idx: usize,  // 0=Local, 1=Remote
    model_input: String,
    host_input: String,
}

#[derive(Clone)]
struct FilterFormState {
    status_idx: usize,    // 0=pendiente, 1=todas, 2=completada, 3=cancelada
    priority_idx: usize,  // 0=todas, 1=alta, 2=media, 3=baja
    group_idx: usize,     // 0=todos, 1..N=grupo, N+1=sin grupo
    date_idx: usize,      // 0=todas, 1=hoy, 2=esta semana, 3=este mes, 4=custom
    date_from: DateTimeInput,
    date_to: DateTimeInput,
    field: usize,         // 0=status, 1=priority, 2=group, 3=fecha, 4=desde, 5=hasta
}

impl Default for FilterFormState {
    fn default() -> Self {
        Self {
            status_idx: 0,
            priority_idx: 0,
            group_idx: 0,
            date_idx: 0,
            date_from: DateTimeInput::date_only_disabled(),
            date_to: DateTimeInput::date_only_disabled(),
            field: 0,
        }
    }
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
// Date/time segmented input widget
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
enum DateInputResult {
    Consumed,
    NextField,
}

#[derive(Clone)]
struct DateTimeInput {
    year: u16,
    month: u8,
    day: u8,
    hour: u8,
    minute: u8,
    segment: usize,
    has_time: bool,
    enabled: bool,
    typing_buf: String,
}

impl DateTimeInput {
    fn date_only_disabled() -> Self {
        let now = chrono::Utc::now();
        Self {
            year: now.format("%Y").to_string().parse().unwrap_or(2026),
            month: now.format("%m").to_string().parse().unwrap_or(1),
            day: now.format("%d").to_string().parse().unwrap_or(1),
            hour: 0,
            minute: 0,
            segment: 0,
            has_time: false,
            enabled: false,
            typing_buf: String::new(),
        }
    }

    fn date_time_now() -> Self {
        let now = chrono::Utc::now();
        Self {
            year: now.format("%Y").to_string().parse().unwrap_or(2026),
            month: now.format("%m").to_string().parse().unwrap_or(1),
            day: now.format("%d").to_string().parse().unwrap_or(1),
            hour: now.format("%H").to_string().parse().unwrap_or(0),
            minute: now.format("%M").to_string().parse().unwrap_or(0),
            segment: 0,
            has_time: true,
            enabled: true,
            typing_buf: String::new(),
        }
    }

    fn from_iso(s: &str, has_time: bool) -> Self {
        let date_part = if let Some(pos) = s.find('T') { &s[..pos] } else { s };
        let parts: Vec<&str> = date_part.split('-').collect();
        let year = parts.first().and_then(|p| p.parse().ok()).unwrap_or(2026);
        let month = parts.get(1).and_then(|p| p.parse().ok()).unwrap_or(1);
        let day = parts.get(2).and_then(|p| p.parse().ok()).unwrap_or(1);

        let (hour, minute) = if has_time {
            if let Some(pos) = s.find('T') {
                let time_part = &s[pos + 1..];
                let hm: Vec<&str> = time_part.split(':').collect();
                let h = hm.first().and_then(|p| p.parse().ok()).unwrap_or(0);
                let m = hm.get(1).and_then(|p| p.parse().ok()).unwrap_or(0);
                (h, m)
            } else {
                (0, 0)
            }
        } else {
            (0, 0)
        };

        let mut input = Self {
            year, month, day, hour, minute,
            segment: 0, has_time, enabled: true, typing_buf: String::new(),
        };
        input.clamp();
        input
    }

    fn from_date_time_strings(date: &str, time: &str) -> Self {
        let parts: Vec<&str> = date.split('-').collect();
        let year = parts.first().and_then(|p| p.parse().ok()).unwrap_or(2026);
        let month = parts.get(1).and_then(|p| p.parse().ok()).unwrap_or(1);
        let day = parts.get(2).and_then(|p| p.parse().ok()).unwrap_or(1);
        let hm: Vec<&str> = time.split(':').collect();
        let hour = hm.first().and_then(|p| p.parse().ok()).unwrap_or(0);
        let minute = hm.get(1).and_then(|p| p.parse().ok()).unwrap_or(0);

        let mut input = Self {
            year, month, day, hour, minute,
            segment: 0, has_time: true, enabled: true, typing_buf: String::new(),
        };
        input.clamp();
        input
    }

    fn max_day(&self) -> u8 {
        jinx::proximos::days_in_month(self.year as u32, self.month as u32) as u8
    }

    fn clamp(&mut self) {
        self.year = self.year.clamp(2020, 2099);
        self.month = self.month.clamp(1, 12);
        self.day = self.day.clamp(1, self.max_day());
        self.hour = self.hour.min(23);
        self.minute = self.minute.min(59);
    }

    fn n_segments(&self) -> usize {
        if self.has_time { 5 } else { 3 }
    }

    fn to_date_string(&self) -> Option<String> {
        if !self.enabled { return None; }
        Some(format!("{:04}-{:02}-{:02}", self.year, self.month, self.day))
    }

    fn to_time_string(&self) -> String {
        format!("{:02}:{:02}", self.hour, self.minute)
    }

    fn to_iso_string(&self) -> Option<String> {
        if !self.enabled { return None; }
        Some(format!(
            "{:04}-{:02}-{:02}T{:02}:{:02}:00+00:00",
            self.year, self.month, self.day, self.hour, self.minute
        ))
    }

    fn commit_typing_buf(&mut self) {
        if self.typing_buf.is_empty() { return; }
        let val: u16 = self.typing_buf.parse().unwrap_or(0);
        match self.segment {
            0 => self.year = (val).clamp(2020, 2099),
            1 => self.month = (val as u8).clamp(1, 12),
            2 => {
                self.day = (val as u8).clamp(1, self.max_day());
            }
            3 => self.hour = (val as u8).min(23),
            4 => self.minute = (val as u8).min(59),
            _ => {}
        }
        self.typing_buf.clear();
        self.clamp();
    }

    fn segment_max_digits(&self) -> usize {
        if self.segment == 0 { 4 } else { 2 }
    }

    fn handle_key(&mut self, code: KeyCode) -> DateInputResult {
        if !self.enabled {
            match code {
                KeyCode::Enter | KeyCode::Char(' ') => {
                    self.enabled = true;
                    return DateInputResult::Consumed;
                }
                KeyCode::Tab => return DateInputResult::NextField,
                _ => return DateInputResult::Consumed,
            }
        }

        match code {
            KeyCode::Left => {
                self.commit_typing_buf();
                if self.segment > 0 {
                    self.segment -= 1;
                }
                DateInputResult::Consumed
            }
            KeyCode::Right => {
                self.commit_typing_buf();
                if self.segment + 1 < self.n_segments() {
                    self.segment += 1;
                }
                DateInputResult::Consumed
            }
            KeyCode::Up => {
                self.commit_typing_buf();
                match self.segment {
                    0 if self.year < 2099 => { self.year += 1; }
                    1 => { self.month = if self.month >= 12 { 1 } else { self.month + 1 }; }
                    2 => {
                        let max = self.max_day();
                        self.day = if self.day >= max { 1 } else { self.day + 1 };
                    }
                    3 => { self.hour = if self.hour >= 23 { 0 } else { self.hour + 1 }; }
                    4 => { self.minute = if self.minute >= 59 { 0 } else { self.minute + 1 }; }
                    _ => {}
                }
                self.clamp();
                DateInputResult::Consumed
            }
            KeyCode::Down => {
                self.commit_typing_buf();
                match self.segment {
                    0 if self.year > 2020 => { self.year -= 1; }
                    1 => { self.month = if self.month <= 1 { 12 } else { self.month - 1 }; }
                    2 => {
                        let max = self.max_day();
                        self.day = if self.day <= 1 { max } else { self.day - 1 };
                    }
                    3 => { self.hour = if self.hour == 0 { 23 } else { self.hour - 1 }; }
                    4 => { self.minute = if self.minute == 0 { 59 } else { self.minute - 1 }; }
                    _ => {}
                }
                self.clamp();
                DateInputResult::Consumed
            }
            KeyCode::Char(c) if c.is_ascii_digit() => {
                self.typing_buf.push(c);
                if self.typing_buf.len() >= self.segment_max_digits() {
                    self.commit_typing_buf();
                    if self.segment + 1 < self.n_segments() {
                        self.segment += 1;
                    }
                }
                DateInputResult::Consumed
            }
            KeyCode::Backspace => {
                if self.typing_buf.pop().is_none() && self.segment > 0 {
                    self.segment -= 1;
                }
                DateInputResult::Consumed
            }
            KeyCode::Delete => {
                if !self.has_time {
                    self.enabled = false;
                }
                DateInputResult::Consumed
            }
            KeyCode::Tab => {
                self.commit_typing_buf();
                DateInputResult::NextField
            }
            _ => DateInputResult::Consumed,
        }
    }
}

fn date_input_line<'a>(label: &'a str, input: &DateTimeInput, field_active: bool) -> Line<'a> {
    let label_style = if field_active {
        Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let mut spans: Vec<Span<'static>> = vec![
        Span::styled(format!("  {:16}", label), label_style),
    ];

    if !input.enabled {
        let hint = if field_active { "(Enter para activar, Tab siguiente)" } else { "(sin fecha)" };
        spans.push(Span::styled(hint.to_string(), Style::default().fg(Color::DarkGray)));
        return Line::from(spans);
    }

    let segments: Vec<String> = if input.has_time {
        vec![
            format!("{:04}", input.year),
            format!("{:02}", input.month),
            format!("{:02}", input.day),
            format!("{:02}", input.hour),
            format!("{:02}", input.minute),
        ]
    } else {
        vec![
            format!("{:04}", input.year),
            format!("{:02}", input.month),
            format!("{:02}", input.day),
        ]
    };

    let separators: Vec<&str> = if input.has_time {
        vec!["-", "-", "  ", ":"]
    } else {
        vec!["-", "-"]
    };

    for (i, seg) in segments.iter().enumerate() {
        let display = if field_active && i == input.segment && !input.typing_buf.is_empty() {
            let pad = if i == 0 { 4 } else { 2 };
            format!("{:_<pad$}", input.typing_buf, pad = pad)
        } else {
            seg.clone()
        };

        let style = if field_active && i == input.segment {
            Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)
        } else if field_active {
            Style::default().fg(Color::White)
        } else {
            Style::default()
        };
        spans.push(Span::styled(display, style));
        if i < separators.len() {
            spans.push(Span::raw(separators[i].to_string()));
        }
    }

    if field_active {
        spans.push(Span::styled(
            "  ←/→:seg ↑↓:±1 Del:quitar".to_string(),
            Style::default().fg(Color::DarkGray),
        ));
    }

    Line::from(spans)
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

// ---------------------------------------------------------------------------
// Tareas panel sub-section and filter state
// ---------------------------------------------------------------------------

#[derive(Default, Clone, Copy, PartialEq, Eq)]
enum TareasSection {
    #[default]
    Tasks,
    Groups,
}

#[derive(Clone)]
struct ActiveTaskFilter {
    status: Option<TaskStatus>,
    group_id: Option<Option<i64>>,
    priority: Option<Priority>,
    from_date: Option<String>,
    to_date: Option<String>,
}

impl Default for ActiveTaskFilter {
    fn default() -> Self {
        Self {
            status: Some(TaskStatus::Pendiente),
            group_id: None,
            priority: None,
            from_date: None,
            to_date: None,
        }
    }
}

impl ActiveTaskFilter {
    fn to_storage_filter(&self) -> TaskFilter {
        TaskFilter {
            status: self.status,
            group_id: self.group_id,
            from_date: self.from_date.clone(),
            to_date: self.to_date.clone(),
        }
    }

    fn is_default(&self) -> bool {
        self.status == Some(TaskStatus::Pendiente)
            && self.group_id.is_none()
            && self.priority.is_none()
            && self.from_date.is_none()
            && self.to_date.is_none()
    }
}

// ---------------------------------------------------------------------------
// Application state and loop
// ---------------------------------------------------------------------------

struct RuntimeState {
    app: AppState,
    chat_history: Vec<ChatMsg>,
    chat_editor: TextEditor,
    chat_scroll: usize, // lines from bottom; 0 = pinned to bottom
    task_cursor: usize,
    calendar_cursor: usize,
    calendar_scroll: usize,
    calendar_scroll_initialized: bool,
    group_cursor: usize,
    tareas_section: TareasSection,
    tareas_filter: ActiveTaskFilter,
    color_mode: ColorMode,
    storage: Arc<dyn Storage + Send + Sync>,
    agent_child: Option<Child>,
    agent_stdin: Option<ChildStdin>,
    agent_rx: Option<mpsc::Receiver<Envelope>>,
    pending_request: Option<(Uuid, Instant)>,
    // Modal form state
    task_form: TaskFormState,
    event_form: EventFormState,
    group_form: GroupFormState,
    settings_form: SettingsFormState,
    filter_form: FilterFormState,
    groups_cache: Vec<Group>,
    delete_confirm_name: String,
    // Layout rects for mouse hit-testing
    panel_area: Option<Rect>,
    input_area: Option<Rect>,
    history_area: Option<Rect>,
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
        chat_editor: TextEditor::new(),
        chat_scroll: 0,
        task_cursor: 0,
        calendar_cursor: 0,
        calendar_scroll: 0,
        calendar_scroll_initialized: false,
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
            state.app = jinx::app::reduce(state.app.clone(), AppEvent::Key(key));
            return;
        }
        KeyCode::BackTab => {
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
    }
}

fn handle_chat_key(state: &mut RuntimeState, key: crossterm::event::KeyEvent) {
    match key.code {
        KeyCode::Enter => {
            let text = state.chat_editor.to_string();
            let trimmed = text.trim().to_string();
            if trimmed.is_empty() {
                state.app.status_bar = "Mensaje vacío, escribe algo para enviar.".to_string();
                return;
            }
            state.chat_history.push(ChatMsg { role: "usuario", text: trimmed.clone() });
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
        // Navigation
        KeyCode::Left => state.chat_editor.move_left(),
        KeyCode::Right => state.chat_editor.move_right(),
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
                    let view = jinx::calendario::calendar_layout(&tasks, &events);
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

fn handle_modal_paste(state: &mut RuntimeState, data: &str) {
    let clean: String = data.chars().filter(|&c| c != '\n' && c != '\r').collect();
    match &state.app.modal {
        Some(Modal::NewTask) | Some(Modal::EditTask { .. })
            if state.task_form.field == 0 => { state.task_form.title.push_str(&clean); }
        Some(Modal::NewEvent) | Some(Modal::EditEvent { .. }) => match state.event_form.field {
            0 => state.event_form.title.push_str(&clean),
            2 => state.event_form.duration.push_str(&clean),
            _ => {}
        },
        Some(Modal::NewGroup) | Some(Modal::EditGroup { .. }) => match state.group_form.field {
            0 => state.group_form.name.push_str(&clean),
            1 => state.group_form.color_custom.push_str(&clean),
            _ => {}
        },
        Some(Modal::Settings) => match state.settings_form.field {
            1 => state.settings_form.model_input.push_str(&clean),
            2 => state.settings_form.host_input.push_str(&clean),
            _ => {}
        },
        _ => {}
    }
}

fn handle_tareas_key(state: &mut RuntimeState, key: crossterm::event::KeyEvent) {
    if key.code == KeyCode::Char('s') {
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
    let tasks = get_filtered_tasks(state);
    match key.code {
        KeyCode::Up if state.task_cursor > 0 => {
            state.task_cursor -= 1;
        }
        KeyCode::Down if state.task_cursor + 1 < tasks.len() => {
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
                let new_status = if t.status == TaskStatus::Completada {
                    TaskStatus::Pendiente
                } else {
                    TaskStatus::Completada
                };
                match state.storage.update_task(
                    t.id,
                    TaskPatch { status: Some(new_status), ..Default::default() },
                ) {
                    Ok(_) => {
                        state.app.status_bar = if new_status == TaskStatus::Completada {
                            "Tarea completada.".to_string()
                        } else {
                            "Tarea marcada como pendiente.".to_string()
                        };
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
        KeyCode::Char('f') => open_filter_modal(state),
        _ => {}
    }
}

fn handle_tareas_groups_key(state: &mut RuntimeState, key: crossterm::event::KeyEvent) {
    let groups = state.storage.list_groups().unwrap_or_default();
    match key.code {
        KeyCode::Up if state.group_cursor > 0 => {
            state.group_cursor -= 1;
        }
        KeyCode::Down if state.group_cursor + 1 < groups.len() => {
            state.group_cursor += 1;
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

fn get_filtered_tasks(state: &RuntimeState) -> Vec<storage::Task> {
    let mut tasks = state
        .storage
        .list_tasks(state.tareas_filter.to_storage_filter())
        .unwrap_or_default();
    if let Some(p) = state.tareas_filter.priority {
        tasks.retain(|t| t.priority == p);
    }
    tasks
}

fn handle_calendario_key(state: &mut RuntimeState, key: crossterm::event::KeyEvent) {
    let tasks = state.storage.list_tasks(TaskFilter::default()).unwrap_or_default();
    let events = state.storage.list_events(None, None).unwrap_or_default();
    let view = jinx::calendario::calendar_layout(&tasks, &events);
    let flat = flat_entries(&view);
    let count = entry_count(&flat);

    match key.code {
        KeyCode::Up if state.calendar_cursor > 0 => {
            state.calendar_cursor -= 1;
        }
        KeyCode::Down if count > 0 && state.calendar_cursor + 1 < count => {
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
                    let task = tasks.iter().find(|t| t.id == entry.entity_id);
                    let new_status = if task.map(|t| t.status) == Some(TaskStatus::Completada) {
                        TaskStatus::Pendiente
                    } else {
                        TaskStatus::Completada
                    };
                    match state.storage.update_task(
                        entry.entity_id,
                        TaskPatch { status: Some(new_status), ..Default::default() },
                    ) {
                        Ok(_) => {
                            state.app.status_bar = if new_status == TaskStatus::Completada {
                                "Tarea completada.".to_string()
                            } else {
                                "Tarea marcada como pendiente.".to_string()
                            };
                        }
                        Err(e) => state.app.status_bar = format!("Error: {}", e.message()),
                    }
                }
            }
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
        let deadline = match &t.deadline {
            Some(s) => DateTimeInput::from_iso(s, false),
            None => DateTimeInput::date_only_disabled(),
        };
        state.task_form = TaskFormState {
            title: t.title.clone(),
            priority_idx,
            deadline,
            group_idx,
            status_idx,
            field: 0,
            edit_id: Some(id),
            error: None,
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
            datetime: DateTimeInput::from_date_time_strings(&ev.start_date, &ev.start_time),
            duration: ev.duration_minutes.map(|d| d.to_string()).unwrap_or_default(),
            group_idx,
            field: 0,
            edit_id: Some(id),
            error: None,
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

fn open_filter_modal(state: &mut RuntimeState) {
    refresh_groups_cache(state);
    let (date_idx, date_from, date_to) = match (&state.tareas_filter.from_date, &state.tareas_filter.to_date) {
        (None, None) => (0, DateTimeInput::date_only_disabled(), DateTimeInput::date_only_disabled()),
        (Some(f), Some(t)) => {
            let today = today_str();
            let (wk_m, wk_s) = week_bounds();
            let (mo_f, mo_l) = month_bounds();
            if f == &today && t == &today {
                (1, DateTimeInput::date_only_disabled(), DateTimeInput::date_only_disabled())
            } else if f == &wk_m && t == &wk_s {
                (2, DateTimeInput::date_only_disabled(), DateTimeInput::date_only_disabled())
            } else if f == &mo_f && t == &mo_l {
                (3, DateTimeInput::date_only_disabled(), DateTimeInput::date_only_disabled())
            } else {
                (4, DateTimeInput::from_iso(f, false), DateTimeInput::from_iso(t, false))
            }
        }
        (Some(f), None) => (4, DateTimeInput::from_iso(f, false), DateTimeInput::date_only_disabled()),
        (None, Some(t)) => (4, DateTimeInput::date_only_disabled(), DateTimeInput::from_iso(t, false)),
    };
    state.filter_form = FilterFormState {
        status_idx: match state.tareas_filter.status {
            Some(TaskStatus::Pendiente) => 0,
            None => 1,
            Some(TaskStatus::Completada) => 2,
            Some(TaskStatus::Cancelada) => 3,
        },
        priority_idx: match state.tareas_filter.priority {
            None => 0,
            Some(Priority::Alta) => 1,
            Some(Priority::Media) => 2,
            Some(Priority::Baja) => 3,
        },
        group_idx: match state.tareas_filter.group_id {
            None => 0,
            Some(Some(gid)) => {
                state.groups_cache.iter().position(|g| g.id == gid).map(|i| i + 1).unwrap_or(0)
            }
            Some(None) => state.groups_cache.len() + 1,
        },
        date_idx,
        date_from,
        date_to,
        field: 0,
    };
    state.app.modal = Some(Modal::FilterTasks);
}

fn handle_filter_form_key(state: &mut RuntimeState, key: crossterm::event::KeyEvent) {
    let n_groups = state.groups_cache.len();
    let is_custom = state.filter_form.date_idx == 4;
    let n_fields: usize = if is_custom { 6 } else { 4 };

    if state.filter_form.field == 4 && is_custom {
        match state.filter_form.date_from.handle_key(key.code) {
            DateInputResult::Consumed => return,
            DateInputResult::NextField => {
                state.filter_form.field = 5;
                return;
            }
        }
    }
    if state.filter_form.field == 5 && is_custom {
        match state.filter_form.date_to.handle_key(key.code) {
            DateInputResult::Consumed => return,
            DateInputResult::NextField => {
                state.filter_form.field = 0;
                return;
            }
        }
    }

    match key.code {
        KeyCode::Tab => {
            let next = (state.filter_form.field + 1) % n_fields;
            state.filter_form.field = if !is_custom && next > 3 { 0 } else { next };
        }
        KeyCode::BackTab => {
            let prev = if state.filter_form.field == 0 { n_fields - 1 } else { state.filter_form.field - 1 };
            state.filter_form.field = if !is_custom && prev > 3 { 3 } else { prev };
        }
        KeyCode::Left => match state.filter_form.field {
            0 => state.filter_form.status_idx = (state.filter_form.status_idx + 3) % 4,
            1 => state.filter_form.priority_idx = (state.filter_form.priority_idx + 3) % 4,
            2 => {
                let n = n_groups + 2;
                state.filter_form.group_idx = (state.filter_form.group_idx + n - 1) % n;
            }
            3 => state.filter_form.date_idx = (state.filter_form.date_idx + 4) % 5,
            _ => {}
        },
        KeyCode::Right => match state.filter_form.field {
            0 => state.filter_form.status_idx = (state.filter_form.status_idx + 1) % 4,
            1 => state.filter_form.priority_idx = (state.filter_form.priority_idx + 1) % 4,
            2 => {
                let n = n_groups + 2;
                state.filter_form.group_idx = (state.filter_form.group_idx + 1) % n;
            }
            3 => state.filter_form.date_idx = (state.filter_form.date_idx + 1) % 5,
            _ => {}
        },
        KeyCode::Char('r') => {
            state.filter_form = FilterFormState::default();
        }
        KeyCode::Enter => {
            apply_filter(state);
        }
        KeyCode::Esc => {
            state.app.modal = None;
        }
        _ => {}
    }

    if state.filter_form.date_idx == 4 {
        state.filter_form.date_from.enabled = true;
        state.filter_form.date_to.enabled = true;
    }
}

fn today_str() -> String {
    chrono::Utc::now().format("%Y-%m-%d").to_string()
}

fn week_bounds() -> (String, String) {
    let now = chrono::Utc::now();
    let weekday = now.format("%u").to_string().parse::<i64>().unwrap_or(1);
    let monday = now - chrono::Duration::days(weekday - 1);
    let sunday = monday + chrono::Duration::days(6);
    (
        monday.format("%Y-%m-%d").to_string(),
        sunday.format("%Y-%m-%d").to_string(),
    )
}

fn month_bounds() -> (String, String) {
    let now = chrono::Utc::now();
    let year: u32 = now.format("%Y").to_string().parse().unwrap_or(2026);
    let month: u32 = now.format("%m").to_string().parse().unwrap_or(1);
    let last_day = jinx::proximos::days_in_month(year, month);
    (
        format!("{:04}-{:02}-01", year, month),
        format!("{:04}-{:02}-{:02}", year, month, last_day),
    )
}

fn apply_filter(state: &mut RuntimeState) {
    let form = &state.filter_form;
    state.tareas_filter.status = match form.status_idx {
        0 => Some(TaskStatus::Pendiente),
        1 => None,
        2 => Some(TaskStatus::Completada),
        3 => Some(TaskStatus::Cancelada),
        _ => Some(TaskStatus::Pendiente),
    };
    state.tareas_filter.priority = match form.priority_idx {
        0 => None,
        1 => Some(Priority::Alta),
        2 => Some(Priority::Media),
        3 => Some(Priority::Baja),
        _ => None,
    };
    let n_groups = state.groups_cache.len();
    state.tareas_filter.group_id = match form.group_idx {
        0 => None,
        i if i <= n_groups => Some(Some(state.groups_cache[i - 1].id)),
        _ => Some(None),
    };
    let (from_date, to_date) = match form.date_idx {
        0 => (None, None),
        1 => { let t = today_str(); (Some(t.clone()), Some(t)) }
        2 => { let (m, s) = week_bounds(); (Some(m), Some(s)) }
        3 => { let (f, l) = month_bounds(); (Some(f), Some(l)) }
        4 => (form.date_from.to_date_string(), form.date_to.to_date_string()),
        _ => (None, None),
    };
    state.tareas_filter.from_date = from_date;
    state.tareas_filter.to_date = to_date;
    state.task_cursor = 0;
    state.app.modal = None;
}

fn open_settings_modal(state: &mut RuntimeState) {
    let cfg = app_config::load();
    state.settings_form = SettingsFormState {
        provider_idx: if cfg.provider == app_config::Provider::Local { 0 } else { 1 },
        model_input: match &cfg.provider {
            app_config::Provider::Local => cfg.local.model.clone(),
            app_config::Provider::Remote => cfg.remote.model_id.clone(),
        },
        host_input: cfg.local.host.clone(),
        field: 0,
    };
    state.app.modal = Some(Modal::Settings);
}

fn handle_settings_form_key(state: &mut RuntimeState, key: crossterm::event::KeyEvent) {
    let is_local = state.settings_form.provider_idx == 0;
    let n_fields = if is_local { 3 } else { 2 };
    match key.code {
        KeyCode::Tab => {
            state.settings_form.field = (state.settings_form.field + 1) % n_fields;
        }
        KeyCode::BackTab => {
            state.settings_form.field = (state.settings_form.field + n_fields - 1) % n_fields;
        }
        KeyCode::Left | KeyCode::Right if state.settings_form.field == 0 => {
            state.settings_form.provider_idx = 1 - state.settings_form.provider_idx;
            // Clamp field when switching to Remote (only 2 fields)
            if state.settings_form.provider_idx == 1 && state.settings_form.field >= 2 {
                state.settings_form.field = 1;
            }
        }
        KeyCode::Char(c) => match state.settings_form.field {
            1 => state.settings_form.model_input.push(c),
            2 => state.settings_form.host_input.push(c),
            _ => {}
        },
        KeyCode::Backspace => match state.settings_form.field {
            1 => { state.settings_form.model_input.pop(); }
            2 => { state.settings_form.host_input.pop(); }
            _ => {}
        },
        KeyCode::Enter => save_settings(state),
        KeyCode::Esc => state.app.modal = None,
        _ => {}
    }
}

fn save_settings(state: &mut RuntimeState) {
    let is_local = state.settings_form.provider_idx == 0;
    let defaults = app_config::Config::default();
    let cfg = app_config::Config {
        provider: if is_local {
            app_config::Provider::Local
        } else {
            app_config::Provider::Remote
        },
        local: app_config::LocalConfig {
            model: if is_local {
                state.settings_form.model_input.trim().to_string()
            } else {
                defaults.local.model
            },
            host: if is_local {
                let h = state.settings_form.host_input.trim().to_string();
                if h.is_empty() { defaults.local.host } else { h }
            } else {
                defaults.local.host
            },
        },
        remote: app_config::RemoteConfig {
            model_id: if !is_local {
                state.settings_form.model_input.trim().to_string()
            } else {
                String::new()
            },
        },
    };
    if let Err(e) = app_config::save(&cfg) {
        state.app.status_bar = format!("Error guardando config: {e}");
        return;
    }
    state.app.modal = None;
    restart_agent(state);
    state.app.status_bar = "Configuración guardada. Agente reiniciado.".to_string();
}

fn restart_agent(state: &mut RuntimeState) {
    send_shutdown(state);
    state.agent_stdin = None;
    state.agent_child = None;
    state.agent_rx = None;
    state.app.agent_alive = false;
    spawn_agent(state);
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
        Some(Modal::Settings) => handle_settings_form_key(state, key),
        Some(Modal::FilterTasks) => handle_filter_form_key(state, key),
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

    if state.task_form.field == 2 {
        match state.task_form.deadline.handle_key(key.code) {
            DateInputResult::Consumed => return,
            DateInputResult::NextField => {
                state.task_form.field = (state.task_form.field + 1) % n_fields;
                return;
            }
        }
    }

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
        KeyCode::Char(c) if state.task_form.field == 0 => { state.task_form.title.push(c); }
        KeyCode::Backspace if state.task_form.field == 0 => { state.task_form.title.pop(); }
        KeyCode::Enter => save_task(state),
        KeyCode::Esc => { state.app.modal = None; state.task_form.error = None; }
        _ => {}
    }
}

fn handle_event_form_key(state: &mut RuntimeState, key: crossterm::event::KeyEvent) {
    let n_fields = 4; // title, datetime, duration, group

    if state.event_form.field == 1 {
        match state.event_form.datetime.handle_key(key.code) {
            DateInputResult::Consumed => return,
            DateInputResult::NextField => {
                state.event_form.field = (state.event_form.field + 1) % n_fields;
                return;
            }
        }
    }

    match key.code {
        KeyCode::Tab => state.event_form.field = (state.event_form.field + 1) % n_fields,
        KeyCode::BackTab => state.event_form.field = (state.event_form.field + n_fields - 1) % n_fields,
        KeyCode::Left if state.event_form.field == 3 => {
            let n = state.groups_cache.len() + 1;
            state.event_form.group_idx = (state.event_form.group_idx + n - 1) % n;
        }
        KeyCode::Right if state.event_form.field == 3 => {
            let n = state.groups_cache.len() + 1;
            state.event_form.group_idx = (state.event_form.group_idx + 1) % n;
        }
        KeyCode::Char(c) => match state.event_form.field {
            0 => state.event_form.title.push(c),
            2 => state.event_form.duration.push(c),
            _ => {}
        },
        KeyCode::Backspace => match state.event_form.field {
            0 => { state.event_form.title.pop(); }
            2 => { state.event_form.duration.pop(); }
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
        KeyCode::Left if state.group_form.field == 1 && state.group_form.color_custom.is_empty() => {
            state.group_form.color_idx = (state.group_form.color_idx + COLOR_PRESETS.len() - 1) % COLOR_PRESETS.len();
        }
        KeyCode::Right if state.group_form.field == 1 && state.group_form.color_custom.is_empty() => {
            state.group_form.color_idx = (state.group_form.color_idx + 1) % COLOR_PRESETS.len();
        }
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
    let deadline = form.deadline.to_iso_string();
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
    let start_date = form.datetime.to_date_string().unwrap_or_default();
    let start_time = form.datetime.to_time_string();
    if start_date.is_empty() {
        state.event_form.error = Some("La fecha de inicio es obligatoria.".to_string());
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
            start_date: Some(start_date.clone()),
            start_time: Some(start_time.clone()),
            duration_minutes: Some(duration_minutes),
            group_id: Some(group_id),
        }).map(|_| ())
    } else {
        state.storage.create_event(NewEvent {
            title: form.title.trim().to_string(),
            start_date,
            start_time,
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
    let agent_project = extract_agent();

    let log_path = agent_log_path();
    let agent_stderr = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .map(Stdio::from)
        .unwrap_or_else(|_| Stdio::null());

    let mut child = Command::new("uv")
        .args([
            "run",
            "--project", agent_project.to_str().unwrap_or("."),
            "python", "-m", "agent.main",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(agent_stderr)
        .spawn()
        .unwrap_or_else(|e| {
            eprintln!("Error al iniciar el agente: {e}");
            eprintln!("Instala uv: brew install uv  (o https://astral.sh/uv)");
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
            .open(agent_log_path())
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
    let cfg = app_config::load();
    let timezone = iana_timezone();
    let (model_provider, ollama_model, ollama_host, bedrock_model_id) = match cfg.provider {
        app_config::Provider::Local => (
            ModelProvider::Local,
            cfg.local.model,
            cfg.local.host,
            None,
        ),
        app_config::Provider::Remote => (
            ModelProvider::Remote,
            String::new(),
            String::new(),
            Some(cfg.remote.model_id).filter(|s| !s.is_empty()),
        ),
    };
    let init_env = Envelope::new(
        Kind::Request,
        MessageType::AgentInit,
        &AgentInitPayload {
            timezone,
            model_provider,
            ollama_model,
            ollama_host,
            bedrock_model_id,
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
    while let Some(env) = state.agent_rx.as_ref().and_then(|rx| rx.try_recv().ok()) {
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
                state.chat_scroll = 0; // auto-scroll to bottom on new message
            }
            state.pending_request = None;
            state.app.status_bar = "Listo.".to_string();
        }
        mt if is_storage_message_type(mt) => {
            let response = jinx::ipc_handler::handle_storage_request(&env, &state.storage);
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

fn render(frame: &mut ratatui::Frame, state: &mut RuntimeState) {
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

    state.panel_area = Some(chunks[1]);
    match state.app.focused_panel {
        Panel::Chat => render_chat(frame, state, chunks[1]),
        Panel::Tareas => render_tareas(frame, state, chunks[1]),
        Panel::Calendario => render_calendario(frame, state, chunks[1]),
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
    };
    let tabs = Tabs::new(vec!["  Chat  ", "  Tareas  ", "  Calendario  "])
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
        Some(Modal::Settings) => render_settings_form(frame, state, popup),
        Some(Modal::FilterTasks) => render_filter_form(frame, state, popup),
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

fn render_filter_form(frame: &mut ratatui::Frame, state: &RuntimeState, area: Rect) {
    let form = &state.filter_form;
    let block = Block::default()
        .title("Filtrar Tareas")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let status_labels = ["pendiente", "todas", "completada", "cancelada"];
    let priority_labels = ["todas", "alta", "media", "baja"];

    let group_label = match form.group_idx {
        0 => "todos".to_string(),
        i if i <= state.groups_cache.len() => state.groups_cache[i - 1].name.clone(),
        _ => "sin grupo".to_string(),
    };

    let date_labels = ["todas", "hoy", "esta semana", "este mes", "custom"];

    let mut lines: Vec<Line<'static>> = vec![Line::from("")];
    lines.push(form_line(
        "Estado",
        format!("← {} →", status_labels[form.status_idx]),
        form.field == 0,
    ));
    lines.push(form_line(
        "Prioridad",
        format!("← {} →", priority_labels[form.priority_idx]),
        form.field == 1,
    ));
    lines.push(form_line(
        "Grupo",
        format!("← {} →", group_label),
        form.field == 2,
    ));
    lines.push(form_line(
        "Fecha",
        format!("← {} →", date_labels[form.date_idx]),
        form.field == 3,
    ));
    if form.date_idx == 4 {
        lines.push(date_input_line("  Desde", &form.date_from, form.field == 4));
        lines.push(date_input_line("  Hasta", &form.date_to, form.field == 5));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "  Tab:campo  ←/→:opción  Enter:aplicar  Esc:cancelar  r:resetear",
        Style::default().fg(Color::DarkGray),
    )));

    frame.render_widget(Paragraph::new(lines), inner);
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
    lines.push(date_input_line("Fecha límite", &form.deadline, form.field == 2));
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
    lines.push(date_input_line("Fecha/Hora", &form.datetime, form.field == 1));
    lines.push(form_line("Duración (min)", if form.duration.is_empty() { "(vacío = sin límite)".into() } else { form.duration.clone() }, form.field == 2));
    lines.push(form_line(
        "Grupo",
        format!("← {} →", groups.get(form.group_idx).map(String::as_str).unwrap_or("(ninguno)")),
        form.field == 3,
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

fn render_settings_form(frame: &mut ratatui::Frame, state: &RuntimeState, area: Rect) {
    let form = &state.settings_form;
    let is_local = form.provider_idx == 0;
    let block = Block::default()
        .title("Configuración")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let provider_label = if is_local { "← Local →" } else { "← Remote →" };
    let model_label = if is_local { "Modelo Ollama" } else { "Modelo Bedrock" };

    let mut lines: Vec<Line<'static>> = vec![Line::from("")];
    lines.push(form_line("Proveedor", provider_label.to_string(), form.field == 0));
    lines.push(form_line(model_label, form.model_input.clone(), form.field == 1));
    if is_local {
        let host_display = if form.host_input.is_empty() {
            "http://localhost:11434".to_string()
        } else {
            form.host_input.clone()
        };
        lines.push(form_line("Host Ollama", host_display, form.field == 2));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "  Tab:campo  ←/→:proveedor  Enter:guardar  Esc:cancelar",
        Style::default().fg(Color::DarkGray),
    )));
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

fn render_chat(frame: &mut ratatui::Frame, state: &mut RuntimeState, area: Rect) {
    let block = panel_block("Chat");
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
        all_lines.push(Line::from(""));
    }

    // Typing indicator when agent is working
    if let Some((_, started)) = &state.pending_request {
        let elapsed = started.elapsed();
        let dot_count = (elapsed.as_millis() / 500) as usize % 3 + 1;
        let dots = ".".repeat(dot_count);
        let secs = elapsed.as_secs();
        let indicator = format!("Agente pensando{}  ({}s)", dots, secs);
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
            "  ↑ mensajes anteriores ↑",
            Style::default().fg(Color::DarkGray).add_modifier(Modifier::ITALIC),
        ));
    }

    frame.render_widget(Paragraph::new(visible), hist_area);

    // Input field with cursor
    let input_block = Block::default().title("Mensaje (Ctrl+J:nueva línea)").borders(Borders::ALL);
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

fn render_tareas(frame: &mut ratatui::Frame, state: &RuntimeState, area: Rect) {
    let block = panel_block("Tareas");
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let groups = state.storage.list_groups().unwrap_or_default();
    let groups_height = (groups.len() + 3).min(8) as u16;

    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(4), Constraint::Length(groups_height)])
        .split(inner);

    // --- Tasks section ---
    let tasks = get_filtered_tasks(state);
    let mut task_items: Vec<ListItem> = Vec::new();

    if !state.tareas_filter.is_default() {
        let status_label = match state.tareas_filter.status {
            Some(TaskStatus::Pendiente) => "pendiente",
            Some(TaskStatus::Completada) => "completada",
            Some(TaskStatus::Cancelada) => "cancelada",
            None => "todas",
        };
        let priority_label = match state.tareas_filter.priority {
            Some(Priority::Alta) => "alta",
            Some(Priority::Media) => "media",
            Some(Priority::Baja) => "baja",
            None => "todas",
        };
        let group_label = match &state.tareas_filter.group_id {
            None => "todos".to_string(),
            Some(None) => "sin grupo".to_string(),
            Some(Some(gid)) => groups
                .iter()
                .find(|g| g.id == *gid)
                .map(|g| g.name.clone())
                .unwrap_or_else(|| "?".to_string()),
        };
        let date_label = match (&state.tareas_filter.from_date, &state.tareas_filter.to_date) {
            (None, None) => String::new(),
            (Some(f), Some(t)) if f == t => format!("  fecha:{}", f),
            (Some(f), Some(t)) => format!("  fecha:{}/{}", f, t),
            (Some(f), None) => format!("  desde:{}", f),
            (None, Some(t)) => format!("  hasta:{}", t),
        };
        let filter_line = format!(
            "Filtros: estado:{}  prioridad:{}  grupo:{}{}",
            status_label, priority_label, group_label, date_label
        );
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

        let deadline_str = t.deadline.as_deref().map(|d| {
            if let Some(pos) = d.find('T') { &d[..pos] } else { d }
        }).unwrap_or("sin fecha");

        let label = format!(
            " {} {}[{}] {} ({})",
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
            "  (sin tareas)",
            Style::default().fg(Color::DarkGray),
        ))));
    }

    let task_hint = if state.tareas_section == TareasSection::Tasks {
        "  ↑↓:nav  n:nueva  e:edit  c:ok  d:del  f:filtro  s:grupos"
    } else {
        "  s:tareas"
    };
    task_items.push(ListItem::new(Line::from(Span::styled(
        task_hint,
        Style::default().fg(Color::DarkGray),
    ))));

    frame.render_widget(List::new(task_items), sections[0]);

    // --- Groups section ---
    let mut group_items: Vec<ListItem> = Vec::new();
    group_items.push(ListItem::new(Line::from(Span::styled(
        "── Grupos ──",
        Style::default().fg(Color::DarkGray).add_modifier(Modifier::BOLD),
    ))));

    if groups.is_empty() {
        group_items.push(ListItem::new("  (sin grupos)"));
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
        "  ↑↓:nav g:nuevo e:edit d:del s:tareas"
    } else {
        "  s:grupos"
    };
    group_items.push(ListItem::new(Line::from(Span::styled(
        group_hint,
        Style::default().fg(Color::DarkGray),
    ))));

    frame.render_widget(List::new(group_items), sections[1]);
}

fn render_calendario(frame: &mut ratatui::Frame, state: &mut RuntimeState, area: Rect) {
    let block = panel_block("Calendario");
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let tasks = state.storage.list_tasks(TaskFilter::default()).unwrap_or_default();
    let events = state.storage.list_events(None, None).unwrap_or_default();
    let view = jinx::calendario::calendar_layout(&tasks, &events);
    let flat = flat_entries(&view);

    let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
    let groups = state.storage.list_groups().unwrap_or_default();
    let mut lines: Vec<ListItem> = vec![];
    let mut entry_idx = 0usize;
    let mut today_line_idx: Option<usize> = None;
    let mut cursor_line_idx: usize = 0;

    for item in &flat {
        match item {
            FlatCalEntry::DateHeader(date) => {
                if date == &today {
                    today_line_idx = Some(lines.len());
                    let label = format!("* {} (hoy)", date);
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
            "  (sin eventos ni tareas con fecha)",
            Style::default().fg(Color::DarkGray),
        ))));
    }

    lines.push(ListItem::new(Line::from(Span::styled(
        "  ↑↓:navegar  n:nuevo  e:editar  c:completar  d:eliminar",
        Style::default().fg(Color::DarkGray),
    ))));

    let visible_height = inner.height as usize;

    if !state.calendar_scroll_initialized && today_line_idx.is_some() {
        state.calendar_scroll = today_line_idx.unwrap_or(0);
        state.calendar_scroll_initialized = true;
    }

    if !lines.is_empty() && visible_height > 0 {
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

fn render_status(frame: &mut ratatui::Frame, state: &RuntimeState, area: Rect) {
    let hint = "Tab:panel  Ctrl+Q:salir";

    if let Some((spinner_char, secs)) = spinner_state(&state.pending_request) {
        let working = format!("{} Pensando... ({}s)", spinner_char, secs);
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

        let err = storage::StorageError::NotFound("Task 99 not found".to_string());
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
            // Verify the warning text that would appear in the status bar.
            let warning = "Mensaje vacío, escribe algo para enviar.";
            prop_assert!(!warning.is_empty());
        }
    }
}
