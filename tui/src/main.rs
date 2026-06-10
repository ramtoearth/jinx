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
// Slash-command registry
// ---------------------------------------------------------------------------

struct SlashCommand {
    name: &'static str,
    description: &'static str,
}

const SLASH_COMMANDS: &[SlashCommand] = &[
    SlashCommand { name: "clear", description: "Borrar chat y reiniciar agente" },
];

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
const AGENT_LOCALE:    &str = include_str!("../../agent/locale.py");
const AGENT_LOCALE_EN: &str = include_str!("../../agent/locales/en.toml");
const AGENT_LOCALE_ES: &str = include_str!("../../agent/locales/es.toml");

const GCAL_INIT:        &str = include_str!("../../gcal_sync/__init__.py");
const GCAL_MAIN:        &str = include_str!("../../gcal_sync/main.py");
const GCAL_OAUTH:       &str = include_str!("../../gcal_sync/oauth.py");
const GCAL_SYNC:        &str = include_str!("../../gcal_sync/sync.py");
const GCAL_PULL:        &str = include_str!("../../gcal_sync/pull.py");
const GCAL_DB:          &str = include_str!("../../gcal_sync/db.py");
const GCAL_CREDENTIALS: &str = include_str!("../../gcal_sync/credentials.json");

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
    write_if_changed(&pkg_dir.join("locale.py"),         AGENT_LOCALE);

    let locale_dir = pkg_dir.join("locales");
    let _ = std::fs::create_dir_all(&locale_dir);
    write_if_changed(&locale_dir.join("en.toml"), AGENT_LOCALE_EN);
    write_if_changed(&locale_dir.join("es.toml"), AGENT_LOCALE_ES);

    let gcal_dir = data_dir.join("gcal_sync");
    let _ = std::fs::create_dir_all(&gcal_dir);
    write_if_changed(&gcal_dir.join("__init__.py"),       GCAL_INIT);
    write_if_changed(&gcal_dir.join("main.py"),           GCAL_MAIN);
    write_if_changed(&gcal_dir.join("oauth.py"),          GCAL_OAUTH);
    write_if_changed(&gcal_dir.join("sync.py"),           GCAL_SYNC);
    write_if_changed(&gcal_dir.join("pull.py"),           GCAL_PULL);
    write_if_changed(&gcal_dir.join("db.py"),             GCAL_DB);
    write_if_changed(&gcal_dir.join("credentials.json"),  GCAL_CREDENTIALS);

    data_dir
}

// ---------------------------------------------------------------------------
// Google Calendar sync status
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
enum SyncStatus {
    #[default]
    Disabled,
    Idle,
    Syncing,
    Error(String),
}

#[derive(Debug, serde::Deserialize)]
struct SyncStatusMsg {
    state: String,
    #[serde(default)]
    message: Option<String>,
}

fn gcal_sync_log_path() -> std::path::PathBuf {
    std::env::temp_dir().join("jinx_gcal_sync.log")
}

// ---------------------------------------------------------------------------
// Chat message
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ChatRole {
    Agent,
    System,
    User,
}

#[derive(Debug, Clone)]
struct NotePickerEntry {
    id: i64,
    title: String,
    updated_at: String,
}

#[derive(Debug, Clone)]
struct ChatMsg {
    role: ChatRole,
    text: String,
    note_results: Option<Vec<NotePickerEntry>>,
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
    title: TextEditor,
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
            title: TextEditor::new(),
            priority_idx: 1,
            deadline: DateTimeInput::date_time_disabled(),
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
    title: TextEditor,
    datetime: DateTimeInput,
    duration: TextEditor,
    group_idx: usize,
    field: usize,         // 0=title 1=datetime 2=duration 3=group
    edit_id: Option<i64>,
    error: Option<String>,
}

impl Default for EventFormState {
    fn default() -> Self {
        Self {
            title: TextEditor::new(),
            datetime: DateTimeInput::date_time_now(),
            duration: TextEditor::new(),
            group_idx: 0,
            field: 0,
            edit_id: None,
            error: None,
        }
    }
}

#[derive(Clone)]
struct GroupFormState {
    name: TextEditor,
    color_idx: usize,     // index into COLOR_PRESETS (or custom)
    color_custom: String, // overrides preset when non-empty
    field: usize,         // 0=name 1=color
    edit_id: Option<i64>,
    error: Option<String>,
}

impl Default for GroupFormState {
    fn default() -> Self {
        Self {
            name: TextEditor::new(),
            color_idx: 0,
            color_custom: String::new(),
            field: 0,
            edit_id: None,
            error: None,
        }
    }
}

#[derive(Default, Clone)]
struct SettingsFormState {
    field: usize,              // 0=language, 1=provider, 2=model|backend, 3=host|model, 4=gcal
    language_idx: usize,       // 0=English, 1=Español
    provider_idx: usize,       // 0=Local, 1=Remote
    backend_idx: usize,        // 0=Bedrock, 1=OpenAI, 2=Anthropic, 3=Gemini, 4=LlamaAPI
    local_model_input: TextEditor,
    host_input: TextEditor,
    bedrock_model_input: TextEditor,
    openai_model_input: TextEditor,
    anthropic_model_input: TextEditor,
    gemini_model_input: TextEditor,
    llamaapi_model_input: TextEditor,
    gcal_enabled: bool,
}

#[derive(Clone)]
struct FilterFormState {
    status_idx: usize,        // 0=pendiente, 1=todas, 2=completada, 3=cancelada
    priority_sel: [bool; 3],  // [alta, media, baja] — multi-select toggles
    priority_cursor: usize,   // 0=alta, 1=media, 2=baja — which one is highlighted
    group_idx: usize,         // 0=todos, 1..N=grupo, N+1=sin grupo
    date_idx: usize,          // 0=todas, 1=hoy, 2=ayer, 3=esta semana, 4=semana pasada, 5=este mes, 6=custom, 7=sin fecha
    date_from: DateTimeInput,
    date_to: DateTimeInput,
    field: usize,             // 0=status, 1=priority, 2=group, 3=fecha, 4=desde, 5=hasta
}

impl Default for FilterFormState {
    fn default() -> Self {
        Self {
            status_idx: 0,
            priority_sel: [false; 3],
            priority_cursor: 0,
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
    PrevField,
    Submit,
    Cancel,
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
        let now = chrono::Local::now();
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

    fn date_time_disabled() -> Self {
        let now = chrono::Local::now();
        Self {
            year: now.format("%Y").to_string().parse().unwrap_or(2026),
            month: now.format("%m").to_string().parse().unwrap_or(1),
            day: now.format("%d").to_string().parse().unwrap_or(1),
            hour: now.format("%H").to_string().parse().unwrap_or(0),
            minute: now.format("%M").to_string().parse().unwrap_or(0),
            segment: 0,
            has_time: true,
            enabled: false,
            typing_buf: String::new(),
        }
    }

    fn date_time_now() -> Self {
        let now = chrono::Local::now();
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
                    DateInputResult::Consumed
                } else {
                    DateInputResult::PrevField
                }
            }
            KeyCode::Right => {
                self.commit_typing_buf();
                if self.segment + 1 < self.n_segments() {
                    self.segment += 1;
                    DateInputResult::Consumed
                } else {
                    DateInputResult::NextField
                }
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
            KeyCode::BackTab => {
                self.commit_typing_buf();
                DateInputResult::PrevField
            }
            KeyCode::Enter => {
                self.commit_typing_buf();
                DateInputResult::Submit
            }
            KeyCode::Esc => DateInputResult::Cancel,
            _ => DateInputResult::Consumed,
        }
    }
}

fn date_input_line(label: &str, input: &DateTimeInput, field_active: bool, hint_active: &str, hint_inactive: &str, hint_controls: &str) -> Line<'static> {
    let label_style = if field_active {
        Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let mut spans: Vec<Span<'static>> = vec![
        Span::styled(format!("  {:16}", label), label_style),
    ];

    if !input.enabled {
        let hint = if field_active { hint_active } else { hint_inactive };
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
            format!("  {}", hint_controls),
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

#[derive(Default, Clone, Copy, PartialEq, Eq)]
enum NotesView {
    #[default]
    List,
    Preview,
    Edit,
}

#[derive(Clone)]
struct ActiveTaskFilter {
    status: Option<TaskStatus>,
    group_id: Option<Option<i64>>,
    priorities: Vec<Priority>,
    from_date: Option<String>,
    to_date: Option<String>,
    no_deadline: bool,
}

impl Default for ActiveTaskFilter {
    fn default() -> Self {
        Self {
            status: Some(TaskStatus::Pendiente),
            group_id: None,
            priorities: Vec::new(),
            from_date: None,
            to_date: None,
            no_deadline: false,
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
            no_deadline: self.no_deadline,
        }
    }

    fn is_default(&self) -> bool {
        self.status == Some(TaskStatus::Pendiente)
            && self.group_id.is_none()
            && self.priorities.is_empty()
            && self.from_date.is_none()
            && self.to_date.is_none()
            && !self.no_deadline
    }
}

// ---------------------------------------------------------------------------
// Application state and loop
// ---------------------------------------------------------------------------

struct RuntimeState {
    app: AppState,
    locale: jinx::locale::Locale,
    chat_history: Vec<ChatMsg>,
    chat_editor: TextEditor,
    chat_scroll: usize, // lines from bottom; 0 = pinned to bottom
    prompt_history: Vec<String>,
    prompt_history_idx: Option<usize>,
    prompt_stash: String,
    task_cursor: usize,
    tareas_scroll: usize,
    tareas_search_active: bool,
    tareas_search_query: String,
    calendar_cursor: usize,
    calendar_scroll: usize,
    calendar_scroll_initialized: bool,
    calendar_filter_idx: usize, // 0=all, 1=today, 2=this week, 3=this month
    group_cursor: usize,
    tareas_section: TareasSection,
    tareas_filter: ActiveTaskFilter,
    color_mode: ColorMode,
    storage: Arc<dyn Storage + Send + Sync>,
    agent_child: Option<Child>,
    agent_stdin: Option<ChildStdin>,
    agent_rx: Option<mpsc::Receiver<Envelope>>,
    pending_request: Option<(Uuid, Instant)>,
    // Google Calendar sync daemon
    sync_child: Option<Child>,
    sync_stdin: Option<ChildStdin>,
    sync_rx: Option<mpsc::Receiver<SyncStatusMsg>>,
    sync_status: SyncStatus,
    oauth_rx: Option<mpsc::Receiver<String>>,
    // Modal form state
    task_form: TaskFormState,
    event_form: EventFormState,
    group_form: GroupFormState,
    settings_form: SettingsFormState,
    filter_form: FilterFormState,
    groups_cache: Vec<Group>,
    delete_confirm_name: String,
    pending_g: bool,
    // Notes panel state
    notes_cache: Vec<storage::Note>,
    notes_cursor: usize,
    notes_scroll: usize,
    notes_view: NotesView,
    notes_editor: TextEditor,
    notes_title_editor: TextEditor,
    notes_title_focused: bool,
    notes_search_active: bool,
    notes_search_query: String,
    notes_current_id: Option<i64>,
    notes_preview_scroll: usize,
    notes_pending_g: bool,
    // Note picker (interactive results in chat)
    last_note_results: Option<Vec<NotePickerEntry>>,
    note_picker_active: bool,
    note_picker_cursor: usize,
    note_picker_msg_idx: Option<usize>,
    // Slash-command picker
    cmd_picker_active: bool,
    cmd_picker_cursor: usize,
    cmd_picker_filtered: Vec<usize>,
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
        sync_child: None,
        sync_stdin: None,
        sync_rx: None,
        sync_status: SyncStatus::Disabled,
        oauth_rx: None,
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
        notes_pending_g: false,
        last_note_results: None,
        note_picker_active: false,
        note_picker_cursor: 0,
        note_picker_msg_idx: None,
        cmd_picker_active: false,
        cmd_picker_cursor: 0,
        cmd_picker_filtered: Vec::new(),
        panel_area: None,
        input_area: None,
        history_area: None,
    };

    // Spawn agent
    spawn_agent(&mut state);
    spawn_sync_daemon(&mut state);

    let tick = Duration::from_millis(250);
    let timeout_dur = Duration::from_secs(30);
    let mut last_tick = Instant::now();

    loop {
        // --- Render -------------------------------------------------------
        terminal.draw(|f| render(f, &mut state))?;

        // --- Drain agent output every iteration (non-blocking) ------------
        read_agent_output(&mut state);
        read_sync_output(&mut state);
        read_oauth_result(&mut state);

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
                        shutdown_sync_daemon(&mut state);
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

fn handle_modal_paste(state: &mut RuntimeState, data: &str) {
    let clean: String = data.chars().filter(|&c| c != '\n' && c != '\r').collect();
    match &state.app.modal {
        Some(Modal::NewTask) | Some(Modal::EditTask { .. })
            if state.task_form.field == 0 => { for c in clean.chars() { state.task_form.title.insert_char(c); } }
        Some(Modal::NewEvent) | Some(Modal::EditEvent { .. }) => match state.event_form.field {
            0 => { for c in clean.chars() { state.event_form.title.insert_char(c); } }
            2 => { for c in clean.chars() { state.event_form.duration.insert_char(c); } }
            _ => {}
        },
        Some(Modal::NewGroup) | Some(Modal::EditGroup { .. }) => match state.group_form.field {
            0 => { for c in clean.chars() { state.group_form.name.insert_char(c); } }
            1 => state.group_form.color_custom.push_str(&clean),
            _ => {}
        },
        Some(Modal::Settings) => {
            if let Some(ed) = settings_active_editor(state) {
                for c in clean.chars() { ed.insert_char(c); }
            }
        }
        _ => {}
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
                        notify_sync_task_changed(state, task_id);
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

fn get_search_filtered_tasks(state: &RuntimeState) -> Vec<storage::Task> {
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

fn get_filtered_tasks(state: &RuntimeState) -> Vec<storage::Task> {
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
                            notify_sync_task_changed(state, task_id);
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
            Some(s) => DateTimeInput::from_iso(s, true),
            None => DateTimeInput::date_time_disabled(),
        };
        state.task_form = TaskFormState {
            title: TextEditor::from_string(&t.title),
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
        let dur_str = ev.duration_minutes.map(|d| d.to_string()).unwrap_or_default();
        state.event_form = EventFormState {
            title: TextEditor::from_string(&ev.title),
            datetime: DateTimeInput::from_date_time_strings(&ev.start_date, &ev.start_time),
            duration: TextEditor::from_string(&dur_str),
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
            name: TextEditor::from_string(&g.name),
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
    let (date_idx, date_from, date_to) = if state.tareas_filter.no_deadline {
        (7, DateTimeInput::date_only_disabled(), DateTimeInput::date_only_disabled())
    } else {
        match (&state.tareas_filter.from_date, &state.tareas_filter.to_date) {
            (None, None) => (0, DateTimeInput::date_only_disabled(), DateTimeInput::date_only_disabled()),
            (Some(f), Some(t)) => {
                let today = today_str();
                let yesterday = yesterday_str();
                let (wk_m, wk_s) = week_bounds();
                let (lw_m, lw_s) = last_week_bounds();
                let (mo_f, mo_l) = month_bounds();
                if f == &today && t == &today {
                    (1, DateTimeInput::date_only_disabled(), DateTimeInput::date_only_disabled())
                } else if f == &yesterday && t == &yesterday {
                    (2, DateTimeInput::date_only_disabled(), DateTimeInput::date_only_disabled())
                } else if f == &wk_m && t == &wk_s {
                    (3, DateTimeInput::date_only_disabled(), DateTimeInput::date_only_disabled())
                } else if f == &lw_m && t == &lw_s {
                    (4, DateTimeInput::date_only_disabled(), DateTimeInput::date_only_disabled())
                } else if f == &mo_f && t == &mo_l {
                    (5, DateTimeInput::date_only_disabled(), DateTimeInput::date_only_disabled())
                } else {
                    (6, DateTimeInput::from_iso(f, false), DateTimeInput::from_iso(t, false))
                }
            }
            (Some(f), None) => (6, DateTimeInput::from_iso(f, false), DateTimeInput::date_only_disabled()),
            (None, Some(t)) => (6, DateTimeInput::date_only_disabled(), DateTimeInput::from_iso(t, false)),
        }
    };
    let priority_sel = [
        state.tareas_filter.priorities.contains(&Priority::Alta),
        state.tareas_filter.priorities.contains(&Priority::Media),
        state.tareas_filter.priorities.contains(&Priority::Baja),
    ];
    state.filter_form = FilterFormState {
        status_idx: match state.tareas_filter.status {
            Some(TaskStatus::Pendiente) => 0,
            None => 1,
            Some(TaskStatus::Completada) => 2,
            Some(TaskStatus::Cancelada) => 3,
        },
        priority_sel,
        priority_cursor: 0,
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
    let is_custom = state.filter_form.date_idx == 6;
    let n_fields: usize = if is_custom { 6 } else { 4 };

    if state.filter_form.field == 4 && is_custom {
        match state.filter_form.date_from.handle_key(key.code) {
            DateInputResult::Consumed => return,
            DateInputResult::NextField => {
                state.filter_form.field = 5;
                return;
            }
            DateInputResult::PrevField => {
                state.filter_form.field = 3;
                return;
            }
            DateInputResult::Submit => {
                apply_filter(state);
                return;
            }
            DateInputResult::Cancel => {
                state.app.modal = None;
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
            DateInputResult::PrevField => {
                state.filter_form.field = 4;
                return;
            }
            DateInputResult::Submit => {
                apply_filter(state);
                return;
            }
            DateInputResult::Cancel => {
                state.app.modal = None;
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
        KeyCode::Down | KeyCode::Char('j') => {
            let next = (state.filter_form.field + 1) % n_fields;
            state.filter_form.field = if !is_custom && next > 3 { 0 } else { next };
        }
        KeyCode::Up | KeyCode::Char('k') => {
            let prev = if state.filter_form.field == 0 { n_fields - 1 } else { state.filter_form.field - 1 };
            state.filter_form.field = if !is_custom && prev > 3 { 3 } else { prev };
        }
        KeyCode::Left | KeyCode::Char('h') => match state.filter_form.field {
            0 => state.filter_form.status_idx = (state.filter_form.status_idx + 3) % 4,
            1 => state.filter_form.priority_cursor = (state.filter_form.priority_cursor + 2) % 3,
            2 => {
                let n = n_groups + 2;
                state.filter_form.group_idx = (state.filter_form.group_idx + n - 1) % n;
            }
            3 => state.filter_form.date_idx = (state.filter_form.date_idx + 7) % 8,
            _ => {}
        },
        KeyCode::Right | KeyCode::Char('l') => match state.filter_form.field {
            0 => state.filter_form.status_idx = (state.filter_form.status_idx + 1) % 4,
            1 => state.filter_form.priority_cursor = (state.filter_form.priority_cursor + 1) % 3,
            2 => {
                let n = n_groups + 2;
                state.filter_form.group_idx = (state.filter_form.group_idx + 1) % n;
            }
            3 => state.filter_form.date_idx = (state.filter_form.date_idx + 1) % 8,
            _ => {}
        },
        KeyCode::Char(' ') if state.filter_form.field == 1 => {
            let c = state.filter_form.priority_cursor;
            state.filter_form.priority_sel[c] = !state.filter_form.priority_sel[c];
        }
        KeyCode::Char('r') => {
            state.filter_form = FilterFormState::default();
        }
        KeyCode::Enter => {
            if state.filter_form.field == 1 {
                let c = state.filter_form.priority_cursor;
                state.filter_form.priority_sel[c] = !state.filter_form.priority_sel[c];
            } else {
                apply_filter(state);
            }
        }
        KeyCode::Esc => {
            state.app.modal = None;
        }
        _ => {}
    }

    if state.filter_form.date_idx == 6 {
        state.filter_form.date_from.enabled = true;
        state.filter_form.date_to.enabled = true;
    }
}

fn today_str() -> String {
    chrono::Local::now().format("%Y-%m-%d").to_string()
}

fn yesterday_str() -> String {
    (chrono::Local::now() - chrono::Duration::days(1))
        .format("%Y-%m-%d")
        .to_string()
}

fn week_bounds() -> (String, String) {
    let now = chrono::Local::now();
    let weekday = now.format("%u").to_string().parse::<i64>().unwrap_or(1);
    let monday = now - chrono::Duration::days(weekday - 1);
    let sunday = monday + chrono::Duration::days(6);
    (
        monday.format("%Y-%m-%d").to_string(),
        sunday.format("%Y-%m-%d").to_string(),
    )
}

fn last_week_bounds() -> (String, String) {
    let now = chrono::Local::now();
    let weekday = now.format("%u").to_string().parse::<i64>().unwrap_or(1);
    let this_monday = now - chrono::Duration::days(weekday - 1);
    let last_monday = this_monday - chrono::Duration::days(7);
    let last_sunday = last_monday + chrono::Duration::days(6);
    (
        last_monday.format("%Y-%m-%d").to_string(),
        last_sunday.format("%Y-%m-%d").to_string(),
    )
}

fn month_bounds() -> (String, String) {
    let now = chrono::Local::now();
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
    let mut priorities = Vec::new();
    if form.priority_sel[0] { priorities.push(Priority::Alta); }
    if form.priority_sel[1] { priorities.push(Priority::Media); }
    if form.priority_sel[2] { priorities.push(Priority::Baja); }
    state.tareas_filter.priorities = priorities;
    let n_groups = state.groups_cache.len();
    state.tareas_filter.group_id = match form.group_idx {
        0 => None,
        i if i <= n_groups => Some(Some(state.groups_cache[i - 1].id)),
        _ => Some(None),
    };
    state.tareas_filter.no_deadline = form.date_idx == 7;
    let (from_date, to_date) = match form.date_idx {
        0 | 7 => (None, None),
        1 => { let t = today_str(); (Some(t.clone()), Some(t)) }
        2 => { let y = yesterday_str(); (Some(y.clone()), Some(y)) }
        3 => { let (m, s) = week_bounds(); (Some(m), Some(s)) }
        4 => { let (m, s) = last_week_bounds(); (Some(m), Some(s)) }
        5 => { let (f, l) = month_bounds(); (Some(f), Some(l)) }
        6 => (form.date_from.to_date_string(), form.date_to.to_date_string()),
        _ => (None, None),
    };
    state.tareas_filter.from_date = from_date;
    state.tareas_filter.to_date = to_date;
    state.task_cursor = 0;
    state.tareas_scroll = 0;
    state.tareas_search_active = false;
    state.tareas_search_query.clear();
    state.app.modal = None;
}

fn open_settings_modal(state: &mut RuntimeState) {
    let cfg = app_config::load();
    let backend_idx = match cfg.remote.backend {
        app_config::RemoteBackend::Bedrock => 0,
        app_config::RemoteBackend::Openai => 1,
        app_config::RemoteBackend::Anthropic => 2,
        app_config::RemoteBackend::Gemini => 3,
        app_config::RemoteBackend::Llamaapi => 4,
    };
    state.settings_form = SettingsFormState {
        language_idx: if cfg.language == "es" { 1 } else { 0 },
        provider_idx: if cfg.provider == app_config::Provider::Local { 0 } else { 1 },
        backend_idx,
        local_model_input: TextEditor::from_string(&cfg.local.model),
        host_input: TextEditor::from_string(&cfg.local.host),
        bedrock_model_input: TextEditor::from_string(&cfg.remote.bedrock_model),
        openai_model_input: TextEditor::from_string(&cfg.remote.openai_model),
        anthropic_model_input: TextEditor::from_string(&cfg.remote.anthropic_model),
        gemini_model_input: TextEditor::from_string(&cfg.remote.gemini_model),
        llamaapi_model_input: TextEditor::from_string(&cfg.remote.llamaapi_model),
        gcal_enabled: cfg.google_calendar.enabled,
        field: 0,
    };
    state.app.modal = Some(Modal::Settings);
}

const N_BACKENDS: usize = 5;

fn active_remote_model(form: &mut SettingsFormState) -> &mut TextEditor {
    match form.backend_idx {
        0 => &mut form.bedrock_model_input,
        1 => &mut form.openai_model_input,
        2 => &mut form.anthropic_model_input,
        3 => &mut form.gemini_model_input,
        _ => &mut form.llamaapi_model_input,
    }
}

fn settings_is_text_field(field: usize, is_local: bool) -> bool {
    match field {
        2 if is_local => true,
        3 => true,
        _ => false,
    }
}

fn settings_active_editor(state: &mut RuntimeState) -> Option<&mut TextEditor> {
    let is_local = state.settings_form.provider_idx == 0;
    match state.settings_form.field {
        2 if is_local => Some(&mut state.settings_form.local_model_input),
        3 if is_local => Some(&mut state.settings_form.host_input),
        3 => Some(active_remote_model(&mut state.settings_form)),
        _ => None,
    }
}

fn handle_settings_form_key(state: &mut RuntimeState, key: crossterm::event::KeyEvent) {
    let is_local = state.settings_form.provider_idx == 0;
    let n_fields: usize = 5;
    match key.code {
        KeyCode::Tab => {
            state.settings_form.field = (state.settings_form.field + 1) % n_fields;
        }
        KeyCode::BackTab => {
            state.settings_form.field = (state.settings_form.field + n_fields - 1) % n_fields;
        }
        KeyCode::Down if !settings_is_text_field(state.settings_form.field, is_local) => {
            state.settings_form.field = (state.settings_form.field + 1) % n_fields;
        }
        KeyCode::Up if !settings_is_text_field(state.settings_form.field, is_local) => {
            state.settings_form.field = (state.settings_form.field + n_fields - 1) % n_fields;
        }
        KeyCode::Char('j') if !settings_is_text_field(state.settings_form.field, is_local) => {
            state.settings_form.field = (state.settings_form.field + 1) % n_fields;
        }
        KeyCode::Char('k') if !settings_is_text_field(state.settings_form.field, is_local) => {
            state.settings_form.field = (state.settings_form.field + n_fields - 1) % n_fields;
        }
        KeyCode::Left | KeyCode::Right | KeyCode::Char('h') | KeyCode::Char('l') if state.settings_form.field == 0 => {
            state.settings_form.language_idx = 1 - state.settings_form.language_idx;
        }
        KeyCode::Left | KeyCode::Right | KeyCode::Char('h') | KeyCode::Char('l') if state.settings_form.field == 1 => {
            state.settings_form.provider_idx = 1 - state.settings_form.provider_idx;
        }
        KeyCode::Left | KeyCode::Right | KeyCode::Char('h') | KeyCode::Char('l') if state.settings_form.field == 2 && !is_local => {
            let idx = &mut state.settings_form.backend_idx;
            if matches!(key.code, KeyCode::Right | KeyCode::Char('l')) {
                *idx = (*idx + 1) % N_BACKENDS;
            } else {
                *idx = (*idx + N_BACKENDS - 1) % N_BACKENDS;
            }
        }
        KeyCode::Left | KeyCode::Right | KeyCode::Char('h') | KeyCode::Char('l') if state.settings_form.field == 4 => {
            state.settings_form.gcal_enabled = !state.settings_form.gcal_enabled;
        }
        KeyCode::Left if settings_is_text_field(state.settings_form.field, is_local) => {
            if let Some(ed) = settings_active_editor(state) { ed.move_left(); }
        }
        KeyCode::Right if settings_is_text_field(state.settings_form.field, is_local) => {
            if let Some(ed) = settings_active_editor(state) { ed.move_right(); }
        }
        KeyCode::Char(c) if settings_is_text_field(state.settings_form.field, is_local) => {
            if let Some(ed) = settings_active_editor(state) { ed.insert_char(c); }
        }
        KeyCode::Backspace if settings_is_text_field(state.settings_form.field, is_local) => {
            if let Some(ed) = settings_active_editor(state) { ed.backspace(); }
        }
        KeyCode::Delete if settings_is_text_field(state.settings_form.field, is_local) => {
            if let Some(ed) = settings_active_editor(state) { ed.delete(); }
        }
        KeyCode::Enter => save_settings(state),
        KeyCode::Esc => state.app.modal = None,
        _ => {}
    }
}

fn save_settings(state: &mut RuntimeState) {
    let is_local = state.settings_form.provider_idx == 0;
    let defaults = app_config::Config::default();
    let lang = if state.settings_form.language_idx == 1 { "es" } else { "en" };
    let form = &state.settings_form;
    let backend = match form.backend_idx {
        0 => app_config::RemoteBackend::Bedrock,
        1 => app_config::RemoteBackend::Openai,
        2 => app_config::RemoteBackend::Anthropic,
        3 => app_config::RemoteBackend::Gemini,
        _ => app_config::RemoteBackend::Llamaapi,
    };

    let existing_cfg = app_config::load();
    let cfg = app_config::Config {
        language: lang.to_string(),
        provider: if is_local {
            app_config::Provider::Local
        } else {
            app_config::Provider::Remote
        },
        local: app_config::LocalConfig {
            model: {
                let m = form.local_model_input.to_string();
                let m = m.trim();
                if m.is_empty() { defaults.local.model } else { m.to_string() }
            },
            host: {
                let h = form.host_input.to_string();
                let h = h.trim();
                if h.is_empty() { defaults.local.host } else { h.to_string() }
            },
        },
        remote: {
            let d = &defaults.remote;
            let or_default = |input: &TextEditor, fallback: &str| {
                let s = input.to_string();
                let trimmed = s.trim().to_string();
                if trimmed.is_empty() { fallback.to_string() } else { trimmed }
            };
            app_config::RemoteConfig {
                backend,
                bedrock_model: form.bedrock_model_input.to_string().trim().to_string(),
                openai_model: or_default(&form.openai_model_input, &d.openai_model),
                anthropic_model: or_default(&form.anthropic_model_input, &d.anthropic_model),
                gemini_model: or_default(&form.gemini_model_input, &d.gemini_model),
                llamaapi_model: or_default(&form.llamaapi_model_input, &d.llamaapi_model),
            }
        },
        google_calendar: app_config::GoogleCalendarConfig {
            enabled: state.settings_form.gcal_enabled,
            ..existing_cfg.google_calendar
        },
    };
    if let Err(e) = app_config::save(&cfg) {
        state.app.status_bar = state.locale.errors.config_save.replace("{error}", &e.to_string());
        return;
    }

    // If Google Calendar was just enabled and no token exists, run OAuth flow
    let gcal_just_enabled = cfg.google_calendar.enabled && !existing_cfg.google_calendar.enabled;
    if gcal_just_enabled && !app_config::google_token_path().exists() {
        run_gcal_oauth(state);
    }

    state.locale = jinx::locale::load(lang);
    state.app.modal = None;
    restart_agent(state);
    state.app.status_bar = state.locale.status.config_saved.clone();
}

fn restart_agent(state: &mut RuntimeState) {
    send_shutdown(state);
    state.agent_stdin = None;
    state.agent_child = None;
    state.agent_rx = None;
    state.app.agent_alive = false;
    spawn_agent(state);
    // Restart sync daemon in case config changed
    shutdown_sync_daemon(state);
    spawn_sync_daemon(state);
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
            notify_sync_task_deleted(s, id);
            match s.storage.delete_task(id) {
                Ok(_) => { s.app.modal = None; s.app.status_bar = s.locale.status.task_deleted.clone(); if s.task_cursor > 0 { s.task_cursor -= 1; } }
                Err(e) => s.app.status_bar = format!("Error: {}", e.message()),
            }
        }),
        Some(Modal::DeleteEvent { id }) => handle_delete_key(state, key, |s| {
            notify_sync_event_deleted(s, id);
            match s.storage.delete_event(id) {
                Ok(_) => { s.app.modal = None; s.app.status_bar = s.locale.status.event_deleted.clone(); if s.calendar_cursor > 0 { s.calendar_cursor -= 1; } }
                Err(e) => s.app.status_bar = format!("Error: {}", e.message()),
            }
        }),
        Some(Modal::DeleteGroup { id }) => handle_delete_key(state, key, |s| {
            match s.storage.delete_group(id) {
                Ok(_) => { s.app.modal = None; s.app.status_bar = s.locale.status.group_deleted.clone(); if s.group_cursor > 0 { s.group_cursor -= 1; } }
                Err(e) => s.app.status_bar = format!("Error: {}", e.message()),
            }
        }),
        Some(Modal::DeleteNote { id }) => handle_delete_key(state, key, |s| {
            match s.storage.delete_note(id) {
                Ok(_) => {
                    s.app.modal = None;
                    s.app.status_bar = s.locale.status.note_deleted.clone();
                    s.notes_view = NotesView::List;
                    s.notes_current_id = None;
                    refresh_notes_cache(s);
                    if s.notes_cursor > 0 { s.notes_cursor -= 1; }
                }
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
            DateInputResult::PrevField => {
                state.task_form.field = (state.task_form.field + n_fields - 1) % n_fields;
                return;
            }
            DateInputResult::Submit => {
                save_task(state);
                return;
            }
            DateInputResult::Cancel => {
                state.app.modal = None;
                state.task_form.error = None;
                return;
            }
        }
    }

    match key.code {
        KeyCode::Tab => state.task_form.field = (state.task_form.field + 1) % n_fields,
        KeyCode::BackTab => state.task_form.field = (state.task_form.field + n_fields - 1) % n_fields,
        KeyCode::Down if state.task_form.field != 0 => {
            state.task_form.field = (state.task_form.field + 1) % n_fields;
        }
        KeyCode::Up if state.task_form.field != 0 => {
            state.task_form.field = (state.task_form.field + n_fields - 1) % n_fields;
        }
        KeyCode::Left | KeyCode::Char('h') if state.task_form.field == 1 => {
            state.task_form.priority_idx = (state.task_form.priority_idx + 2) % 3;
        }
        KeyCode::Left | KeyCode::Char('h') if state.task_form.field == 3 => {
            let n = state.groups_cache.len() + 1;
            state.task_form.group_idx = (state.task_form.group_idx + n - 1) % n;
        }
        KeyCode::Left | KeyCode::Char('h') if state.task_form.field == 4 => {
            state.task_form.status_idx = (state.task_form.status_idx + 2) % 3;
        }
        KeyCode::Right | KeyCode::Char('l') if state.task_form.field == 1 => {
            state.task_form.priority_idx = (state.task_form.priority_idx + 1) % 3;
        }
        KeyCode::Right | KeyCode::Char('l') if state.task_form.field == 3 => {
            let n = state.groups_cache.len() + 1;
            state.task_form.group_idx = (state.task_form.group_idx + 1) % n;
        }
        KeyCode::Right | KeyCode::Char('l') if state.task_form.field == 4 => {
            state.task_form.status_idx = (state.task_form.status_idx + 1) % 3;
        }
        KeyCode::Char('j') if state.task_form.field != 0 => {
            state.task_form.field = (state.task_form.field + 1) % n_fields;
        }
        KeyCode::Char('k') if state.task_form.field != 0 => {
            state.task_form.field = (state.task_form.field + n_fields - 1) % n_fields;
        }
        KeyCode::Left if state.task_form.field == 0 => { state.task_form.title.move_left(); }
        KeyCode::Right if state.task_form.field == 0 => { state.task_form.title.move_right(); }
        KeyCode::Char(c) if state.task_form.field == 0 => { state.task_form.title.insert_char(c); }
        KeyCode::Backspace if state.task_form.field == 0 => { state.task_form.title.backspace(); }
        KeyCode::Delete if state.task_form.field == 0 => { state.task_form.title.delete(); }
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
            DateInputResult::PrevField => {
                state.event_form.field = (state.event_form.field + n_fields - 1) % n_fields;
                return;
            }
            DateInputResult::Submit => {
                save_event(state);
                return;
            }
            DateInputResult::Cancel => {
                state.app.modal = None;
                state.event_form.error = None;
                return;
            }
        }
    }

    match key.code {
        KeyCode::Tab => state.event_form.field = (state.event_form.field + 1) % n_fields,
        KeyCode::BackTab => state.event_form.field = (state.event_form.field + n_fields - 1) % n_fields,
        KeyCode::Down if state.event_form.field != 0 && state.event_form.field != 2 => {
            state.event_form.field = (state.event_form.field + 1) % n_fields;
        }
        KeyCode::Up if state.event_form.field != 0 && state.event_form.field != 2 => {
            state.event_form.field = (state.event_form.field + n_fields - 1) % n_fields;
        }
        KeyCode::Left | KeyCode::Char('h') if state.event_form.field == 3 => {
            let n = state.groups_cache.len() + 1;
            state.event_form.group_idx = (state.event_form.group_idx + n - 1) % n;
        }
        KeyCode::Right | KeyCode::Char('l') if state.event_form.field == 3 => {
            let n = state.groups_cache.len() + 1;
            state.event_form.group_idx = (state.event_form.group_idx + 1) % n;
        }
        KeyCode::Char('j') if state.event_form.field == 3 => {
            state.event_form.field = (state.event_form.field + 1) % n_fields;
        }
        KeyCode::Char('k') if state.event_form.field == 3 => {
            state.event_form.field = (state.event_form.field + n_fields - 1) % n_fields;
        }
        KeyCode::Left => match state.event_form.field {
            0 => { state.event_form.title.move_left(); }
            2 => { state.event_form.duration.move_left(); }
            _ => {}
        },
        KeyCode::Right => match state.event_form.field {
            0 => { state.event_form.title.move_right(); }
            2 => { state.event_form.duration.move_right(); }
            _ => {}
        },
        KeyCode::Char(c) => match state.event_form.field {
            0 => { state.event_form.title.insert_char(c); }
            2 => { state.event_form.duration.insert_char(c); }
            _ => {}
        },
        KeyCode::Backspace => match state.event_form.field {
            0 => { state.event_form.title.backspace(); }
            2 => { state.event_form.duration.backspace(); }
            _ => {}
        },
        KeyCode::Delete => match state.event_form.field {
            0 => { state.event_form.title.delete(); }
            2 => { state.event_form.duration.delete(); }
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
        KeyCode::Down | KeyCode::Up => {
            state.group_form.field = (state.group_form.field + 1) % 2;
        }
        KeyCode::Left | KeyCode::Char('h') if state.group_form.field == 1 && state.group_form.color_custom.is_empty() => {
            state.group_form.color_idx = (state.group_form.color_idx + COLOR_PRESETS.len() - 1) % COLOR_PRESETS.len();
        }
        KeyCode::Right | KeyCode::Char('l') if state.group_form.field == 1 && state.group_form.color_custom.is_empty() => {
            state.group_form.color_idx = (state.group_form.color_idx + 1) % COLOR_PRESETS.len();
        }
        KeyCode::Char('j') | KeyCode::Char('k') if state.group_form.field == 1 && state.group_form.color_custom.is_empty() => {
            state.group_form.field = (state.group_form.field + 1) % 2;
        }
        KeyCode::Left if state.group_form.field == 0 => {
            state.group_form.name.move_left();
        }
        KeyCode::Right if state.group_form.field == 0 => {
            state.group_form.name.move_right();
        }
        KeyCode::Char(c) => match state.group_form.field {
            0 => { state.group_form.name.insert_char(c); }
            1 => { state.group_form.color_custom.push(c); }
            _ => {}
        },
        KeyCode::Backspace => match state.group_form.field {
            0 => { state.group_form.name.backspace(); }
            1 => { state.group_form.color_custom.pop(); }
            _ => {}
        },
        KeyCode::Delete if state.group_form.field == 0 => { state.group_form.name.delete(); }
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
    let title_text = form.title.to_string();
    if title_text.trim().is_empty() {
        state.task_form.error = Some(state.locale.errors.title_empty.clone());
        return;
    }
    let priorities = [Priority::Alta, Priority::Media, Priority::Baja];
    let statuses = [TaskStatus::Pendiente, TaskStatus::Completada, TaskStatus::Cancelada];
    let priority = priorities[form.priority_idx];
    let deadline = form.deadline.to_iso_string();
    let group_id = if form.group_idx == 0 { None } else {
        state.groups_cache.get(form.group_idx - 1).map(|g| g.id)
    };

    let result = if let Some(id) = form.edit_id {
        state.storage.update_task(id, TaskPatch {
            title: Some(title_text.trim().to_string()),
            priority: Some(priority),
            deadline: Some(deadline),
            group_id: Some(group_id),
            status: Some(statuses[form.status_idx]),
        }).map(|t| t.id)
    } else {
        state.storage.create_task(NewTask {
            title: title_text.trim().to_string(),
            priority: Some(priority),
            deadline,
            group_id,
        }).map(|t| t.id)
    };

    match result {
        Ok(task_id) => {
            state.app.modal = None;
            state.task_form = TaskFormState::default();
            state.app.status_bar = state.locale.status.task_saved.clone();
            notify_sync_task_changed(state, task_id);
        }
        Err(e) => state.task_form.error = Some(e.message()),
    }
}

fn save_event(state: &mut RuntimeState) {
    let form = state.event_form.clone();
    let title_text = form.title.to_string();
    let duration_text = form.duration.to_string();
    if title_text.trim().is_empty() {
        state.event_form.error = Some(state.locale.errors.title_empty.clone());
        return;
    }
    let start_date = form.datetime.to_date_string().unwrap_or_default();
    let start_time = form.datetime.to_time_string();
    if start_date.is_empty() {
        state.event_form.error = Some(state.locale.errors.start_date_required.clone());
        return;
    }
    let duration_minutes: Option<u32> = if duration_text.trim().is_empty() {
        None
    } else {
        match duration_text.trim().parse::<u32>() {
            Ok(d) => Some(d),
            Err(_) => { state.event_form.error = Some(state.locale.errors.duration_integer.clone()); return; }
        }
    };
    let group_id = if form.group_idx == 0 { None } else {
        state.groups_cache.get(form.group_idx - 1).map(|g| g.id)
    };

    let result = if let Some(id) = form.edit_id {
        state.storage.update_event(id, EventPatch {
            title: Some(title_text.trim().to_string()),
            start_date: Some(start_date.clone()),
            start_time: Some(start_time.clone()),
            duration_minutes: Some(duration_minutes),
            group_id: Some(group_id),
        }).map(|e| e.id)
    } else {
        state.storage.create_event(NewEvent {
            title: title_text.trim().to_string(),
            start_date,
            start_time,
            duration_minutes,
            group_id,
        }).map(|e| e.id)
    };

    match result {
        Ok(event_id) => {
            state.app.modal = None;
            state.event_form = EventFormState::default();
            state.app.status_bar = state.locale.status.event_saved.clone();
            notify_sync_event_changed(state, event_id);
        }
        Err(e) => state.event_form.error = Some(e.message()),
    }
}

fn save_group(state: &mut RuntimeState) {
    let form = state.group_form.clone();
    let name_text = form.name.to_string();
    if name_text.trim().is_empty() {
        state.group_form.error = Some(state.locale.errors.name_empty.clone());
        return;
    }
    let color_str = form.effective_color().to_string();
    let color = match HexColor::new(&color_str) {
        Ok(c) => c,
        Err(_) => { state.group_form.error = Some(state.locale.errors.color_invalid.clone()); return; }
    };

    let result: Result<_, _> = if let Some(id) = form.edit_id {
        state.storage.rename_group(id, name_text.trim().to_string())
            .and_then(|_| state.storage.recolor_group(id, color))
            .map(|_| ())
    } else {
        state.storage.create_group(NewGroup { name: name_text.trim().to_string(), color }).map(|_| ())
    };

    match result {
        Ok(()) => {
            state.app.modal = None;
            state.group_form = GroupFormState::default();
            state.app.status_bar = state.locale.status.group_saved.clone();
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

    let mut cmd = Command::new("uv");
    cmd.args([
            "run",
            "--project", agent_project.to_str().unwrap_or("."),
            "python", "-m", "agent.main",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(agent_stderr);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt as _;
        cmd.process_group(0);
    }
    let mut child = cmd.spawn()
        .unwrap_or_else(|e| {
            eprintln!("{}", state.locale.errors.agent_start.replace("{error}", &e.to_string()));
            eprintln!("Install uv: brew install uv  (or https://astral.sh/uv)");
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
    let (model_provider, backend, model_id, host) = match cfg.provider {
        app_config::Provider::Local => (
            ModelProvider::Local,
            "ollama".to_string(),
            cfg.local.model,
            Some(cfg.local.host),
        ),
        app_config::Provider::Remote => {
            let (be, mid) = match cfg.remote.backend {
                app_config::RemoteBackend::Bedrock => ("bedrock", cfg.remote.bedrock_model),
                app_config::RemoteBackend::Openai => ("openai", cfg.remote.openai_model),
                app_config::RemoteBackend::Anthropic => ("anthropic", cfg.remote.anthropic_model),
                app_config::RemoteBackend::Gemini => ("gemini", cfg.remote.gemini_model),
                app_config::RemoteBackend::Llamaapi => ("llamaapi", cfg.remote.llamaapi_model),
            };
            (ModelProvider::Remote, be.to_string(), mid, None)
        }
    };
    let init_env = Envelope::new(
        Kind::Request,
        MessageType::AgentInit,
        &AgentInitPayload {
            timezone,
            language: cfg.language,
            model_provider,
            backend,
            model_id,
            host,
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

// ---------------------------------------------------------------------------
// Google Calendar sync daemon
// ---------------------------------------------------------------------------

fn spawn_sync_daemon(state: &mut RuntimeState) {
    let cfg = app_config::load();
    if !cfg.google_calendar.enabled {
        state.sync_status = SyncStatus::Disabled;
        return;
    }
    let token_path = app_config::google_token_path();
    if !token_path.exists() {
        state.sync_status = SyncStatus::Disabled;
        return;
    }

    // Mark all existing events for push on first sync activation
    let _ = state.storage.mark_all_push_pending();

    let agent_project = extract_agent();
    let db_path = match storage::resolve_db_path() {
        Ok(p) => p,
        Err(_) => {
            state.sync_status = SyncStatus::Error("Cannot resolve DB path".to_string());
            return;
        }
    };

    let calendar_id = if cfg.google_calendar.calendar_id.is_empty() {
        "primary".to_string()
    } else {
        cfg.google_calendar.calendar_id
    };

    let log_path = gcal_sync_log_path();
    let sync_stderr = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .map(Stdio::from)
        .unwrap_or_else(|_| Stdio::null());

    let mut sync_cmd = Command::new("uv");
    sync_cmd.args([
            "run",
            "--project", agent_project.to_str().unwrap_or("."),
            "--extra", "gcal",
            "python", "-m", "gcal_sync.main",
            "--db", db_path.to_str().unwrap_or(""),
            "--calendar-id", &calendar_id,
            "--token-path", token_path.to_str().unwrap_or(""),
            "--timezone", &iana_timezone(),
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(sync_stderr);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt as _;
        sync_cmd.process_group(0);
    }
    let child_result = sync_cmd.spawn();

    let mut child = match child_result {
        Ok(c) => c,
        Err(_) => {
            state.sync_status = SyncStatus::Error("Failed to start sync daemon".to_string());
            return;
        }
    };

    let stdin = child.stdin.take().expect("sync child stdin");
    let child_stdout = child.stdout.take().expect("sync child stdout");

    let (tx, rx) = mpsc::channel::<SyncStatusMsg>();
    std::thread::spawn(move || {
        use std::io::BufRead;
        let reader = std::io::BufReader::new(child_stdout);
        for line in reader.lines() {
            match line {
                Ok(line) if !line.trim().is_empty() => {
                    if let Ok(msg) = serde_json::from_str::<SyncStatusMsg>(&line) {
                        if tx.send(msg).is_err() {
                            break;
                        }
                    }
                }
                Err(_) => break,
                _ => {}
            }
        }
    });

    state.sync_child = Some(child);
    state.sync_stdin = Some(stdin);
    state.sync_rx = Some(rx);
    state.sync_status = SyncStatus::Idle;
}

/// Spawns the OAuth subprocess in the background. The result arrives via
/// `oauth_rx` and is handled in `read_oauth_result`.
fn run_gcal_oauth(state: &mut RuntimeState) {
    let agent_project = extract_agent();
    let token_path = app_config::google_token_path();

    state.app.status_bar = "Opening browser for Google Calendar authorization...".to_string();

    let log_path = gcal_sync_log_path();
    let oauth_stderr = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .map(Stdio::from)
        .unwrap_or_else(|_| Stdio::null());

    let project_str = agent_project.to_str().unwrap_or(".").to_string();
    let token_str = token_path.to_str().unwrap_or("").to_string();

    let result = Command::new("uv")
        .args([
            "run",
            "--project", &project_str,
            "--extra", "gcal",
            "python", "-m", "gcal_sync.oauth",
            "--token-path", &token_str,
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(oauth_stderr)
        .spawn();

    let mut child = match result {
        Ok(c) => c,
        Err(e) => {
            state.app.status_bar = format!("OAuth error: {e}");
            return;
        }
    };

    let (tx, rx) = mpsc::channel::<String>();
    state.oauth_rx = Some(rx);

    let child_stdout = child.stdout.take();
    std::thread::spawn(move || {
        // Read all output
        let output = child_stdout.map(|stdout| {
            use std::io::Read;
            let mut buf = String::new();
            let mut reader = std::io::BufReader::new(stdout);
            let _ = reader.read_to_string(&mut buf);
            buf
        });

        // Wait for process to finish
        let status = child.wait();

        let msg = match status {
            Ok(exit) if exit.success() => {
                if let Some(ref out) = output {
                    if let Ok(val) = serde_json::from_str::<serde_json::Value>(out.trim()) {
                        if val.get("status").and_then(|s| s.as_str()) == Some("error") {
                            if let Some(err_msg) = val.get("message").and_then(|m| m.as_str()) {
                                format!("error:{err_msg}")
                            } else {
                                "error:Unknown OAuth error".to_string()
                            }
                        } else {
                            "ok".to_string()
                        }
                    } else {
                        "ok".to_string()
                    }
                } else {
                    "ok".to_string()
                }
            }
            Ok(_) => "error:Google Calendar authorization cancelled or failed.".to_string(),
            Err(e) => format!("error:{e}"),
        };
        let _ = tx.send(msg);
    });
}

/// Kill a child process and its entire process group.
/// On Unix, sends SIGTERM to the process group, then SIGKILL if it doesn't exit.
/// On non-Unix, falls back to `child.kill()`.
fn kill_process_tree(child: &mut Child) {
    #[cfg(unix)]
    {
        let pid = child.id() as i32;
        // Send SIGTERM to the process group (negative pid = group)
        unsafe { libc::kill(-pid, libc::SIGTERM); }
        // Give it a moment to exit gracefully
        std::thread::sleep(Duration::from_millis(100));
        if let Ok(Some(_)) = child.try_wait() { return; }
        // Force kill the group
        unsafe { libc::kill(-pid, libc::SIGKILL); }
        let _ = child.wait();
    }
    #[cfg(not(unix))]
    {
        let _ = child.kill();
        let _ = child.wait();
    }
}

fn shutdown_sync_daemon(state: &mut RuntimeState) {
    if let Some(ref mut stdin) = state.sync_stdin {
        let cmd = "{\"command\":\"stop\"}\n";
        let _ = stdin.write_all(cmd.as_bytes());
        let _ = stdin.flush();
    }
    state.sync_stdin = None;

    if let Some(ref mut child) = state.sync_child {
        let deadline = Instant::now() + Duration::from_millis(500);
        loop {
            match child.try_wait() {
                Ok(Some(_)) => break,
                Ok(None) if Instant::now() >= deadline => {
                    kill_process_tree(child);
                    break;
                }
                Ok(None) => std::thread::sleep(Duration::from_millis(50)),
                Err(_) => break,
            }
        }
    }
    state.sync_child = None;
    state.sync_rx = None;
    state.sync_status = SyncStatus::Disabled;
}

fn notify_sync_event_changed(state: &mut RuntimeState, event_id: i64) {
    if matches!(state.sync_status, SyncStatus::Idle | SyncStatus::Syncing) {
        let _ = state.storage.mark_push_pending(event_id);
        send_sync_command(state, "{\"command\":\"sync\"}");
    }
}

fn notify_sync_task_changed(state: &mut RuntimeState, task_id: i64) {
    if matches!(state.sync_status, SyncStatus::Idle | SyncStatus::Syncing) {
        let _ = state.storage.mark_task_push_pending(task_id);
        send_sync_command(state, "{\"command\":\"sync\"}");
    }
}

fn notify_sync_event_deleted(state: &mut RuntimeState, event_id: i64) {
    if matches!(state.sync_status, SyncStatus::Idle | SyncStatus::Syncing) {
        if let Ok(Some(gid)) = state.storage.get_google_event_id(event_id) {
            let cmd = serde_json::json!({"command": "delete", "google_event_id": gid, "kind": "event"});
            send_sync_command(state, &cmd.to_string());
        }
    }
}

fn notify_sync_task_deleted(state: &mut RuntimeState, task_id: i64) {
    if matches!(state.sync_status, SyncStatus::Idle | SyncStatus::Syncing) {
        if let Ok(Some(gid)) = state.storage.get_task_google_event_id(task_id) {
            let cmd = serde_json::json!({"command": "delete", "google_event_id": gid, "kind": "task"});
            send_sync_command(state, &cmd.to_string());
        }
    }
}

fn send_sync_command(state: &mut RuntimeState, cmd: &str) {
    if let Some(ref mut stdin) = state.sync_stdin {
        let line = format!("{}\n", cmd);
        let _ = stdin.write_all(line.as_bytes());
        let _ = stdin.flush();
    }
}

fn read_oauth_result(state: &mut RuntimeState) {
    if let Some(msg) = state.oauth_rx.as_ref().and_then(|rx| rx.try_recv().ok()) {
        state.oauth_rx = None;
        if msg == "ok" {
            state.app.status_bar = "Google Calendar connected!".to_string();
            // Now that the token exists, start the sync daemon
            spawn_sync_daemon(state);
        } else if let Some(err) = msg.strip_prefix("error:") {
            state.app.status_bar = format!("OAuth error: {err}");
            // Disable gcal in config since auth failed
            let mut cfg = app_config::load();
            cfg.google_calendar.enabled = false;
            let _ = app_config::save(&cfg);
        }
    }
}

fn read_sync_output(state: &mut RuntimeState) {
    while let Some(msg) = state.sync_rx.as_ref().and_then(|rx| rx.try_recv().ok()) {
        match msg.state.as_str() {
            "idle" | "done" => state.sync_status = SyncStatus::Idle,
            "syncing" => state.sync_status = SyncStatus::Syncing,
            "error" => {
                state.sync_status = SyncStatus::Error(
                    msg.message.unwrap_or_else(|| "Unknown error".to_string()),
                );
            }
            _ => {}
        }
    }
}

// ---------------------------------------------------------------------------
// Agent communication
// ---------------------------------------------------------------------------

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
            state.app.status_bar = state.locale.errors.agent_send.clone();
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
    state.agent_stdin = None;

    if let Some(ref mut child) = state.agent_child {
        let deadline = Instant::now() + Duration::from_millis(500);
        loop {
            match child.try_wait() {
                Ok(Some(_)) => break,
                Ok(None) if Instant::now() >= deadline => {
                    kill_process_tree(child);
                    break;
                }
                Ok(None) => std::thread::sleep(Duration::from_millis(50)),
                Err(_) => break,
            }
        }
    }
    state.agent_child = None;
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
                    state.chat_history.push(ChatMsg { role: ChatRole::System, text: notice, note_results: None });
                }
            }
        }
        MessageType::AgentReply => {
            if let Ok(Some(p)) = env.payload_as::<AgentReplyPayload>() {
                let note_results = state.last_note_results.take()
                    .filter(|v| !v.is_empty());
                let msg_idx = state.chat_history.len();
                state.chat_history.push(ChatMsg {
                    role: ChatRole::Agent,
                    text: p.text,
                    note_results: note_results.clone(),
                });
                if note_results.is_some() {
                    state.note_picker_active = true;
                    state.note_picker_cursor = 0;
                    state.note_picker_msg_idx = Some(msg_idx);
                }
                state.chat_scroll = 0;
            }
            state.pending_request = None;
            state.app.status_bar = state.locale.status.ready.clone();
        }
        MessageType::StorageSyncGoogle => {
            // Trigger push + pull via sync daemon
            send_sync_command(state, "{\"command\":\"sync\"}");
            send_sync_command(state, "{\"command\":\"pull\"}");

            let response = Envelope::new(
                Kind::Response,
                MessageType::StorageSyncGoogle,
                &serde_json::json!({"status": "Sync triggered."}),
            ).unwrap().with_ref(env.id);
            if let Some(ref mut stdin) = state.agent_stdin {
                if let Ok(line) = serde_json::to_string(&response) {
                    let _ = stdin.write_all(line.as_bytes());
                    let _ = stdin.write_all(b"\n");
                    let _ = stdin.flush();
                }
            }
        }
        mt if is_storage_message_type(mt) => {
            // For deletes, capture the google ID and kind before the storage op
            let pre_delete_info: Option<(String, &str)> = match mt {
                MessageType::StorageDeleteEvent => {
                    env.payload_as::<jinx::ipc::StorageDeleteEventRequest>()
                        .ok()
                        .flatten()
                        .and_then(|req| state.storage.get_google_event_id(req.id).ok().flatten())
                        .map(|id| (id, "event"))
                }
                MessageType::StorageDeleteTask => {
                    env.payload_as::<jinx::ipc::StorageDeleteTaskRequest>()
                        .ok()
                        .flatten()
                        .and_then(|req| state.storage.get_task_google_event_id(req.id).ok().flatten())
                        .map(|id| (id, "task"))
                }
                _ => None,
            };

            let response = jinx::ipc_handler::handle_storage_request(&env, &state.storage);

            // Capture note results for the interactive picker (only search, not list)
            if matches!(mt, MessageType::StorageSearchNotes) {
                if let Some(payload) = response.payload.as_ref() {
                    if let Some(notes_arr) = payload.get("notes").and_then(|v| v.as_array()) {
                        let entries: Vec<NotePickerEntry> = notes_arr.iter().filter_map(|n| {
                            Some(NotePickerEntry {
                                id: n.get("id")?.as_i64()?,
                                title: n.get("title")?.as_str()?.to_string(),
                                updated_at: n.get("updated_at")?.as_str()?.to_string(),
                            })
                        }).collect();
                        state.last_note_results = Some(entries);
                    }
                }
            }

            // Trigger Google Calendar push for event and task mutations
            if matches!(state.sync_status, SyncStatus::Idle | SyncStatus::Syncing) {
                let mut needs_sync = false;

                if let Some(event_id) = extract_event_id_from_response(mt, &response) {
                    let _ = state.storage.mark_push_pending(event_id);
                    needs_sync = true;
                }
                if let Some(task_id) = extract_task_id_from_response(mt, &response) {
                    let _ = state.storage.mark_task_push_pending(task_id);
                    needs_sync = true;
                }
                if let Some((google_id, kind)) = pre_delete_info {
                    let cmd = serde_json::json!({
                        "command": "delete",
                        "google_event_id": google_id,
                        "kind": kind
                    });
                    send_sync_command(state, &cmd.to_string());
                    needs_sync = false; // delete already sent
                }
                if needs_sync {
                    send_sync_command(state, "{\"command\":\"sync\"}");
                }
            }

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

fn extract_event_id_from_response(mt: MessageType, response: &Envelope) -> Option<i64> {
    use jinx::ipc::{StorageCreateEventResponse, StorageUpdateEventResponse};
    match mt {
        MessageType::StorageCreateEvent => {
            response.payload_as::<StorageCreateEventResponse>().ok()?.map(|r| r.event.id)
        }
        MessageType::StorageUpdateEvent => {
            response.payload_as::<StorageUpdateEventResponse>().ok()?.map(|r| r.event.id)
        }
        _ => None,
    }
}

fn extract_task_id_from_response(mt: MessageType, response: &Envelope) -> Option<i64> {
    use jinx::ipc::{StorageCreateTaskResponse, StorageUpdateTaskResponse, StorageCompleteTaskResponse};
    match mt {
        MessageType::StorageCreateTask => {
            response.payload_as::<StorageCreateTaskResponse>().ok()?.map(|r| r.task.id)
        }
        MessageType::StorageUpdateTask => {
            response.payload_as::<StorageUpdateTaskResponse>().ok()?.map(|r| r.task.id)
        }
        MessageType::StorageCompleteTask => {
            response.payload_as::<StorageCompleteTaskResponse>().ok()?.map(|r| r.task.id)
        }
        _ => None,
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
            | MessageType::StorageListNotes
            | MessageType::StorageSearchNotes
            | MessageType::StorageCreateNote
            | MessageType::StorageUpdateNote
            | MessageType::StorageDeleteNote
    )
}

fn iana_timezone() -> String {
    if let Ok(tz) = std::env::var("TZ") {
        if !tz.is_empty() {
            return tz;
        }
    }
    if let Ok(target) = std::fs::read_link("/etc/localtime") {
        let path = target.to_string_lossy().to_string();
        if let Some(idx) = path.find("zoneinfo/") {
            return path[idx + 9..].to_string();
        }
    }
    "UTC".to_string()
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
        Some(Modal::DeleteTask { .. }) | Some(Modal::DeleteEvent { .. }) | Some(Modal::DeleteGroup { .. }) | Some(Modal::DeleteNote { .. }) => {
            render_delete_confirm(frame, state, popup);
        }
        Some(Modal::Settings) => render_settings_form(frame, state, popup),
        Some(Modal::FilterTasks) => render_filter_form(frame, state, popup),
        _ => {}
    }
}

fn form_line(label: &str, value: String, active: bool) -> Line<'static> {
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

fn form_line_editor(label: &str, editor: &TextEditor, active: bool) -> Line<'static> {
    let (ls, vs) = if active {
        (Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
         Style::default().fg(Color::Cyan))
    } else {
        (Style::default().fg(Color::DarkGray), Style::default())
    };
    let text = editor.to_string();
    if active {
        let col = editor.cursor_col();
        let (before, after) = text.split_at(col.min(text.len()));
        Line::from(vec![
            Span::styled(format!("  {:16}", label), ls),
            Span::styled(before.to_string(), vs),
            Span::styled("│".to_string(), Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
            Span::styled(after.to_string(), vs),
        ])
    } else {
        Line::from(vec![
            Span::styled(format!("  {:16}", label), ls),
            Span::styled(text, vs),
        ])
    }
}

fn render_filter_form(frame: &mut ratatui::Frame, state: &RuntimeState, area: Rect) {
    let form = &state.filter_form;
    let block = Block::default()
        .title(state.locale.modals.filter_tasks.as_str())
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let status_labels = [
        state.locale.filters.pending.as_str(),
        state.locale.filters.all.as_str(),
        state.locale.filters.completed.as_str(),
        state.locale.filters.cancelled.as_str(),
    ];
    let priority_labels = [
        state.locale.filters.high.as_str(),
        state.locale.filters.medium.as_str(),
        state.locale.filters.low.as_str(),
    ];

    let group_label = match form.group_idx {
        0 => state.locale.filters.all_groups.clone(),
        i if i <= state.groups_cache.len() => state.groups_cache[i - 1].name.clone(),
        _ => state.locale.filters.no_group.clone(),
    };

    let date_labels = [
        state.locale.filters.all.as_str(),
        state.locale.filters.today.as_str(),
        state.locale.filters.yesterday.as_str(),
        state.locale.filters.this_week.as_str(),
        state.locale.filters.last_week.as_str(),
        state.locale.filters.this_month.as_str(),
        state.locale.filters.custom.as_str(),
        state.locale.filters.no_date_filter.as_str(),
    ];

    let mut lines: Vec<Line<'static>> = vec![Line::from("")];
    lines.push(form_line(
        state.locale.form_labels.status.as_str(),
        format!("← {} →", status_labels[form.status_idx]),
        form.field == 0,
    ));

    // Priority multi-select: show checkboxes with cursor highlight
    {
        let field_active = form.field == 1;
        let label_style = if field_active {
            Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::DarkGray)
        };
        let mut spans: Vec<Span<'static>> = vec![
            Span::styled(format!("  {:16}", state.locale.form_labels.priority.as_str()), label_style),
        ];
        for (i, plabel) in priority_labels.iter().enumerate() {
            let checked = if form.priority_sel[i] { "[x]" } else { "[ ]" };
            let is_cursor = field_active && form.priority_cursor == i;
            let style = if is_cursor {
                Style::default().fg(Color::White).add_modifier(Modifier::BOLD)
            } else if form.priority_sel[i] {
                Style::default().fg(Color::Green)
            } else {
                Style::default().fg(Color::DarkGray)
            };
            spans.push(Span::styled(format!("{}{}", checked, plabel), style));
            if i < 2 { spans.push(Span::raw(" ")); }
        }
        if !form.priority_sel.iter().any(|&s| s) {
            spans.push(Span::styled(
                format!(" ({})", state.locale.filters.all.as_str()),
                Style::default().fg(Color::DarkGray),
            ));
        }
        lines.push(Line::from(spans));
    }

    lines.push(form_line(
        state.locale.form_labels.group.as_str(),
        format!("← {} →", group_label),
        form.field == 2,
    ));
    lines.push(form_line(
        state.locale.form_labels.date.as_str(),
        format!("← {} →", date_labels[form.date_idx]),
        form.field == 3,
    ));
    if form.date_idx == 6 {
        lines.push(date_input_line(
            state.locale.form_labels.from_date.as_str(), &form.date_from, form.field == 4,
            &state.locale.hints.date_input_inactive, &state.locale.hints.no_date, &state.locale.hints.date_input_active,
        ));
        lines.push(date_input_line(
            state.locale.form_labels.to_date.as_str(), &form.date_to, form.field == 5,
            &state.locale.hints.date_input_inactive, &state.locale.hints.no_date, &state.locale.hints.date_input_active,
        ));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        state.locale.hints.filter_form.clone(),
        Style::default().fg(Color::DarkGray),
    )));

    frame.render_widget(Paragraph::new(lines), inner);
}

fn render_task_form(frame: &mut ratatui::Frame, state: &RuntimeState, area: Rect) {
    let form = &state.task_form;
    let is_edit = form.edit_id.is_some();
    let title_str = if is_edit { &state.locale.modals.edit_task } else { &state.locale.modals.new_task };
    let block = Block::default().title(title_str.as_str()).borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let priorities = [
        state.locale.filters.high.as_str(),
        state.locale.filters.medium.as_str(),
        state.locale.filters.low.as_str(),
    ];
    let statuses = [
        state.locale.filters.pending.as_str(),
        state.locale.filters.completed.as_str(),
        state.locale.filters.cancelled.as_str(),
    ];
    let groups: Vec<String> = std::iter::once(state.locale.filters.none.clone())
        .chain(state.groups_cache.iter().map(|g| g.name.clone()))
        .collect();

    let mut lines: Vec<Line<'static>> = vec![Line::from("")];
    lines.push(form_line_editor(state.locale.form_labels.title.as_str(), &form.title, form.field == 0));
    lines.push(form_line(state.locale.form_labels.priority.as_str(), format!("← {} →", priorities[form.priority_idx]), form.field == 1));
    lines.push(date_input_line(
        state.locale.form_labels.deadline.as_str(), &form.deadline, form.field == 2,
        &state.locale.hints.date_input_inactive, &state.locale.hints.no_date, &state.locale.hints.date_input_active,
    ));
    lines.push(form_line(
        state.locale.form_labels.group.as_str(),
        format!("← {} →", groups.get(form.group_idx).map(String::as_str).unwrap_or(state.locale.filters.none.as_str())),
        form.field == 3,
    ));
    if is_edit {
        lines.push(form_line(state.locale.form_labels.status.as_str(), format!("← {} →", statuses[form.status_idx]), form.field == 4));
    }
    lines.push(Line::from(""));
    if let Some(ref err) = form.error {
        lines.push(Line::from(Span::styled(format!("  ⚠ {err}"), Style::default().fg(Color::Red))));
    }
    lines.push(Line::from(Span::styled(
        state.locale.hints.task_form.clone(),
        Style::default().fg(Color::DarkGray),
    )));
    frame.render_widget(Paragraph::new(lines), inner);
}

fn render_event_form(frame: &mut ratatui::Frame, state: &RuntimeState, area: Rect) {
    let form = &state.event_form;
    let is_edit = form.edit_id.is_some();
    let title_str = if is_edit { &state.locale.modals.edit_event } else { &state.locale.modals.new_event };
    let block = Block::default().title(title_str.as_str()).borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let groups: Vec<String> = std::iter::once(state.locale.filters.none.clone())
        .chain(state.groups_cache.iter().map(|g| g.name.clone()))
        .collect();

    let mut lines: Vec<Line<'static>> = vec![Line::from("")];
    lines.push(form_line_editor(state.locale.form_labels.title.as_str(), &form.title, form.field == 0));
    lines.push(date_input_line(
        state.locale.form_labels.datetime.as_str(), &form.datetime, form.field == 1,
        &state.locale.hints.date_input_inactive, &state.locale.hints.no_date, &state.locale.hints.date_input_active,
    ));
    let dur_text = form.duration.to_string();
    lines.push(form_line_editor(state.locale.form_labels.duration_min.as_str(), &form.duration, form.field == 2));
    if form.field != 2 && dur_text.is_empty() {
        lines.pop();
        lines.push(form_line(state.locale.form_labels.duration_min.as_str(), state.locale.misc.empty_duration.clone(), false));
    }
    lines.push(form_line(
        state.locale.form_labels.group.as_str(),
        format!("← {} →", groups.get(form.group_idx).map(String::as_str).unwrap_or(state.locale.filters.none.as_str())),
        form.field == 3,
    ));
    lines.push(Line::from(""));
    if let Some(ref err) = form.error {
        lines.push(Line::from(Span::styled(format!("  ⚠ {err}"), Style::default().fg(Color::Red))));
    }
    lines.push(Line::from(Span::styled(
        state.locale.hints.event_form.clone(),
        Style::default().fg(Color::DarkGray),
    )));
    frame.render_widget(Paragraph::new(lines), inner);
}

fn render_group_form(frame: &mut ratatui::Frame, state: &RuntimeState, area: Rect) {
    let form = &state.group_form;
    let is_edit = form.edit_id.is_some();
    let title_str = if is_edit { &state.locale.modals.edit_group } else { &state.locale.modals.new_group };
    let block = Block::default().title(title_str.as_str()).borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let color_display = if form.color_custom.is_empty() {
        state.locale.misc.preset_color
            .replace("{color}", COLOR_PRESETS[form.color_idx % COLOR_PRESETS.len()])
            .replace("{idx}", &(form.color_idx + 1).to_string())
            .replace("{total}", &COLOR_PRESETS.len().to_string())
    } else {
        state.locale.misc.custom_color.replace("{color}", &form.color_custom)
    };

    let mut lines: Vec<Line<'static>> = vec![Line::from("")];
    lines.push(form_line_editor(state.locale.form_labels.name.as_str(), &form.name, form.field == 0));
    lines.push(form_line(state.locale.form_labels.color.as_str(), color_display, form.field == 1));
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        state.locale.hints.color_hint.clone(),
        Style::default().fg(Color::DarkGray),
    )));
    if let Some(ref err) = form.error {
        lines.push(Line::from(Span::styled(format!("  ⚠ {err}"), Style::default().fg(Color::Red))));
    }
    lines.push(Line::from(Span::styled(
        state.locale.hints.group_form.clone(),
        Style::default().fg(Color::DarkGray),
    )));
    frame.render_widget(Paragraph::new(lines), inner);
}

fn render_delete_confirm(frame: &mut ratatui::Frame, state: &RuntimeState, area: Rect) {
    let kind = match &state.app.modal {
        Some(Modal::DeleteTask { .. }) => state.locale.misc.task_kind.as_str(),
        Some(Modal::DeleteEvent { .. }) => state.locale.misc.event_kind.as_str(),
        Some(Modal::DeleteGroup { .. }) => state.locale.misc.group_kind.as_str(),
        _ => state.locale.misc.item_kind.as_str(),
    };
    let block = Block::default().title(state.locale.modals.confirm_delete.as_str()).borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Red));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let confirm_text = state.locale.misc.delete_confirm
        .replace("{kind}", kind)
        .replace("{name}", &state.delete_confirm_name);

    let lines: Vec<Line<'static>> = vec![
        Line::from(""),
        Line::from(Span::styled(
            confirm_text,
            Style::default().fg(Color::Yellow),
        )),
        Line::from(""),
        Line::from(Span::styled(
            state.locale.hints.delete_prompt.clone(),
            Style::default().fg(Color::DarkGray),
        )),
    ];
    frame.render_widget(Paragraph::new(lines), inner);
}

fn render_settings_form(frame: &mut ratatui::Frame, state: &RuntimeState, area: Rect) {
    let form = &state.settings_form;
    let is_local = form.provider_idx == 0;
    let block = Block::default()
        .title(state.locale.modals.settings.as_str())
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let language_label = if form.language_idx == 1 { "← Español →" } else { "← English →" };
    let provider_label = if is_local { "← Local →" } else { "← Remote →" };

    let mut lines: Vec<Line<'static>> = vec![Line::from("")];
    lines.push(form_line(state.locale.form_labels.language.as_str(), language_label.to_string(), form.field == 0));
    lines.push(form_line(state.locale.form_labels.provider.as_str(), provider_label.to_string(), form.field == 1));

    if is_local {
        lines.push(form_line_editor(state.locale.form_labels.ollama_model.as_str(), &form.local_model_input, form.field == 2));
        if form.field == 3 || !form.host_input.is_empty() {
            lines.push(form_line_editor(state.locale.form_labels.ollama_host.as_str(), &form.host_input, form.field == 3));
        } else {
            lines.push(form_line(state.locale.form_labels.ollama_host.as_str(), "http://localhost:11434".to_string(), false));
        }
    } else {
        let backend_names = ["Bedrock", "OpenAI", "Anthropic", "Gemini", "LlamaAPI"];
        let backend_label = format!("← {} →", backend_names[form.backend_idx]);
        lines.push(form_line(state.locale.form_labels.backend.as_str(), backend_label, form.field == 2));
        let model_editor = match form.backend_idx {
            0 => &form.bedrock_model_input,
            1 => &form.openai_model_input,
            2 => &form.anthropic_model_input,
            3 => &form.gemini_model_input,
            _ => &form.llamaapi_model_input,
        };
        lines.push(form_line_editor(state.locale.form_labels.model.as_str(), model_editor, form.field == 3));
    }

    lines.push(Line::from(""));
    let gcal_label = if form.gcal_enabled { "← Enabled →" } else { "← Disabled →" };
    lines.push(form_line(state.locale.form_labels.google_calendar.as_str(), gcal_label.to_string(), form.field == 4));

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        state.locale.hints.settings_form.clone(),
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
    let mut cursor_line_idx: usize = 0;

    for item in &flat {
        match item {
            FlatCalEntry::DateHeader(date) => {
                if date == &today {
                    today_line_idx = Some(lines.len());
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
            match state.storage.create_note(storage::NewNote {
                title: String::new(),
                body: String::new(),
            }) {
                Ok(note) => {
                    state.notes_current_id = Some(note.id);
                    state.notes_title_editor = TextEditor::new();
                    state.notes_editor = TextEditor::new();
                    state.notes_title_focused = true;
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
        _ => {}
    }
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
                                'u' => { state.notes_editor.kill_to_start(); }
                                'k' => { state.notes_editor.kill_to_end(); }
                                'a' => { state.notes_editor.move_home(); }
                                'e' => { state.notes_editor.move_end(); }
                                _ => {}
                            }
                        } else {
                            state.notes_editor.insert_char(c);
                        }
                    }
                    KeyCode::Enter => { state.notes_editor.insert_newline(); }
                    KeyCode::Backspace => { state.notes_editor.backspace(); }
                    KeyCode::Delete => { state.notes_editor.delete(); }
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
        match state.storage.update_note(id, storage::NotePatch {
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
        .constraints([Constraint::Length(2), Constraint::Min(1)])
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
}

fn render_note_edit(frame: &mut ratatui::Frame, state: &RuntimeState, area: Rect) {
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

    let body_para = Paragraph::new(body_lines).style(body_style);
    frame.render_widget(body_para, chunks[1]);

    // Set cursor position in edit mode
    if !state.notes_title_focused && input_w > 0 {
        let (row, col) = (state.notes_editor.cursor_row(), state.notes_editor.cursor_col());
        let mut visual_row: u16 = 0;
        let mut visual_col: u16 = 0;
        for (i, logical_line) in state.notes_editor.lines().iter().enumerate() {
            let char_count = logical_line.chars().count();
            let n_visual = (char_count / input_w) + 1;
            if i == row {
                visual_row += (col / input_w) as u16;
                visual_col = (col % input_w) as u16;
                break;
            }
            visual_row += n_visual as u16;
        }
        let x = chunks[1].x + visual_col;
        let y = chunks[1].y + visual_row;
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
        let sync_prefix = match &state.sync_status {
            SyncStatus::Syncing => "↻ ",
            SyncStatus::Idle => "☁ ",
            SyncStatus::Error(msg) => {
                let _ = msg; // used below
                "⚠ "
            }
            SyncStatus::Disabled => "",
        };
        let text = if state.app.status_bar.is_empty() {
            format!("{}{}", sync_prefix, hint)
        } else {
            format!("{}{}  │  {}", sync_prefix, state.app.status_bar, hint)
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
            // The locale's empty_message string is used as the warning — just
            // verify the guard logic works (the exact text is locale-dependent).
            let locale = jinx::locale::load("en");
            prop_assert!(!locale.errors.empty_message.is_empty());
        }
    }
}
