use std::process::{Child, ChildStdin};
use std::sync::mpsc;
use std::time::Instant;

use crossterm::event::KeyCode;
use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
};
use jinx_core::{AppServices, Group, Priority, TaskFilter, TaskStatus};
use uuid::Uuid;

use jinx::app::AppState;
use jinx::color::ColorMode;
use jinx::ipc::Envelope;
use jinx::text_editor::TextEditor;

// ---------------------------------------------------------------------------
// Chat message
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ChatRole {
    Agent,
    System,
    User,
}

#[derive(Debug, Clone)]
pub(crate) struct NotePickerEntry {
    pub(crate) id: i64,
    pub(crate) title: String,
    pub(crate) updated_at: String,
}

#[derive(Debug, Clone)]
pub(crate) struct ChatMsg {
    pub(crate) role: ChatRole,
    pub(crate) text: String,
    pub(crate) note_results: Option<Vec<NotePickerEntry>>,
}

// ---------------------------------------------------------------------------
// Modal form state
// ---------------------------------------------------------------------------

pub(crate) const SPINNER_FRAMES: [char; 10] = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];

pub(crate) const COLOR_PRESETS: [&str; 16] = [
    "#e74c3c", "#e67e22", "#f1c40f", "#2ecc71",
    "#1abc9c", "#3498db", "#9b59b6", "#e91e63",
    "#795548", "#607d8b", "#ff5722", "#009688",
    "#4caf50", "#2196f3", "#9c27b0", "#f44336",
];

#[derive(Clone)]
pub(crate) struct TaskFormState {
    pub(crate) title: TextEditor,
    pub(crate) priority_idx: usize,  // 0=alta 1=media 2=baja
    pub(crate) deadline: DateTimeInput,
    pub(crate) group_idx: usize,     // 0=ninguno, 1..=N = groups_cache[idx-1]
    pub(crate) status_idx: usize,    // 0=pendiente 1=completada 2=cancelada (edit only)
    pub(crate) field: usize,         // active field: 0=title 1=priority 2=deadline 3=group 4=status(edit)
    pub(crate) edit_id: Option<i64>,
    pub(crate) error: Option<String>,
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
pub(crate) struct EventFormState {
    pub(crate) title: TextEditor,
    pub(crate) datetime: DateTimeInput,
    pub(crate) duration: TextEditor,
    pub(crate) group_idx: usize,
    pub(crate) field: usize,         // 0=title 1=datetime 2=duration 3=group
    pub(crate) edit_id: Option<i64>,
    pub(crate) error: Option<String>,
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
pub(crate) struct GroupFormState {
    pub(crate) name: TextEditor,
    pub(crate) color_idx: usize,     // index into COLOR_PRESETS (or custom)
    pub(crate) color_custom: String, // overrides preset when non-empty
    pub(crate) field: usize,         // 0=name 1=color
    pub(crate) edit_id: Option<i64>,
    pub(crate) error: Option<String>,
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

impl GroupFormState {
    pub(crate) fn effective_color(&self) -> &str {
        if !self.color_custom.is_empty() {
            &self.color_custom
        } else {
            COLOR_PRESETS[self.color_idx % COLOR_PRESETS.len()]
        }
    }
}

#[derive(Default, Clone)]
pub(crate) struct SettingsFormState {
    pub(crate) field: usize,              // 0=language, 1=provider, 2=model|backend, 3=host|model, 4=panels
    pub(crate) language_idx: usize,       // 0=English, 1=Español
    pub(crate) provider_idx: usize,       // 0=Local, 1=Remote
    pub(crate) backend_idx: usize,        // 0=Bedrock, 1=OpenAI, 2=Anthropic, 3=Gemini, 4=LlamaAPI
    pub(crate) local_model_input: TextEditor,
    pub(crate) host_input: TextEditor,
    pub(crate) bedrock_model_input: TextEditor,
    pub(crate) openai_model_input: TextEditor,
    pub(crate) anthropic_model_input: TextEditor,
    pub(crate) gemini_model_input: TextEditor,
    pub(crate) llamaapi_model_input: TextEditor,
    pub(crate) panel_sel: [bool; 5],
    pub(crate) panel_cursor: usize,
}

#[derive(Clone)]
pub(crate) struct FilterFormState {
    pub(crate) status_idx: usize,        // 0=pendiente, 1=todas, 2=completada, 3=cancelada
    pub(crate) priority_sel: [bool; 3],  // [alta, media, baja] — multi-select toggles
    pub(crate) priority_cursor: usize,   // 0=alta, 1=media, 2=baja — which one is highlighted
    pub(crate) group_idx: usize,         // 0=todos, 1..N=grupo, N+1=sin grupo
    pub(crate) date_idx: usize,          // 0=todas, 1=hoy, 2=ayer, 3=esta semana, 4=semana pasada, 5=este mes, 6=custom, 7=sin fecha
    pub(crate) date_from: DateTimeInput,
    pub(crate) date_to: DateTimeInput,
    pub(crate) field: usize,             // 0=status, 1=priority, 2=group, 3=fecha, 4=desde, 5=hasta
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

// ---------------------------------------------------------------------------
// Finance form states
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub(crate) struct TransactionFormState {
    pub(crate) amount: String,
    pub(crate) tx_type_idx: usize, // 0=gasto, 1=ingreso
    pub(crate) category_idx: usize,
    pub(crate) category_adding: bool,
    pub(crate) category_new_name: String,
    pub(crate) description: String,
    pub(crate) date: DateTimeInput,
    pub(crate) field: usize, // 0=amount, 1=type, 2=category, 3=description, 4=date
    pub(crate) error: Option<String>,
}

impl Default for TransactionFormState {
    fn default() -> Self {
        Self {
            amount: String::new(),
            tx_type_idx: 0,
            category_idx: 0,
            category_adding: false,
            category_new_name: String::new(),
            description: String::new(),
            date: DateTimeInput::date_only_now(),
            field: 0,
            error: None,
        }
    }
}

#[derive(Clone)]
pub(crate) struct DebtFormState {
    pub(crate) creditor: String,
    pub(crate) total_amount: String,
    pub(crate) interest_rate: String,
    pub(crate) monthly_payment: String,
    pub(crate) due_day: String,
    pub(crate) start_date: DateTimeInput,
    pub(crate) field: usize,
    pub(crate) error: Option<String>,
}

impl Default for DebtFormState {
    fn default() -> Self {
        Self {
            creditor: String::new(),
            total_amount: String::new(),
            interest_rate: String::new(),
            monthly_payment: String::new(),
            due_day: String::new(),
            start_date: DateTimeInput::date_only_now(),
            field: 0,
            error: None,
        }
    }
}

#[derive(Clone)]
pub(crate) struct GoalFormState {
    pub(crate) name: String,
    pub(crate) target_amount: String,
    pub(crate) current_amount: String,
    pub(crate) deadline: DateTimeInput,
    pub(crate) horizon_idx: usize, // 0=corto, 1=mediano, 2=largo
    pub(crate) field: usize,
    pub(crate) error: Option<String>,
}

impl Default for GoalFormState {
    fn default() -> Self {
        Self {
            name: String::new(),
            target_amount: String::new(),
            current_amount: String::new(),
            deadline: DateTimeInput::date_only_disabled(),
            horizon_idx: 0,
            field: 0,
            error: None,
        }
    }
}

#[derive(Default, Clone)]
pub(crate) struct BudgetFormState {
    pub(crate) category_idx: usize,
    pub(crate) monthly_limit: String,
    pub(crate) field: usize,
    pub(crate) error: Option<String>,
}

pub(crate) struct DirBrowserEntry {
    pub(crate) name: String,
    pub(crate) is_dir: bool,
}

pub(crate) struct DirBrowserState {
    pub(crate) current_dir: std::path::PathBuf,
    pub(crate) entries: Vec<DirBrowserEntry>,
    pub(crate) cursor: usize,
    pub(crate) scroll: usize,
    pub(crate) filename: String,
    pub(crate) field: usize, // 0 = browser, 1 = filename
    pub(crate) note_id: i64,
}

impl Default for DirBrowserState {
    fn default() -> Self {
        Self {
            current_dir: std::env::var("HOME").map(std::path::PathBuf::from).unwrap_or_else(|_| std::path::PathBuf::from("/")),
            entries: Vec::new(),
            cursor: 0,
            scroll: 0,
            filename: String::new(),
            field: 0,
            note_id: 0,
        }
    }
}

impl DirBrowserState {
    pub(crate) fn refresh_entries(&mut self) {
        self.entries.clear();
        if let Ok(read_dir) = std::fs::read_dir(&self.current_dir) {
            let mut dirs = Vec::new();
            let mut files = Vec::new();
            for entry in read_dir.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if name.starts_with('.') {
                    continue;
                }
                let is_dir = entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false);
                if is_dir {
                    dirs.push(DirBrowserEntry { name, is_dir: true });
                } else {
                    files.push(DirBrowserEntry { name, is_dir: false });
                }
            }
            dirs.sort_by_key(|a| a.name.to_lowercase());
            files.sort_by_key(|a| a.name.to_lowercase());
            self.entries.extend(dirs);
            self.entries.extend(files);
        }
        self.cursor = 0;
        self.scroll = 0;
    }
}

// ---------------------------------------------------------------------------
// Date/time segmented input widget
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum DateInputResult {
    Consumed,
    NextField,
    PrevField,
    Submit,
    Cancel,
}

#[derive(Clone)]
pub(crate) struct DateTimeInput {
    pub(crate) year: u16,
    pub(crate) month: u8,
    pub(crate) day: u8,
    pub(crate) hour: u8,
    pub(crate) minute: u8,
    pub(crate) segment: usize,
    pub(crate) has_time: bool,
    pub(crate) enabled: bool,
    pub(crate) typing_buf: String,
}

impl DateTimeInput {
    pub(crate) fn date_only_disabled() -> Self {
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

    pub(crate) fn date_only_now() -> Self {
        let now = chrono::Local::now();
        Self {
            year: now.format("%Y").to_string().parse().unwrap_or(2026),
            month: now.format("%m").to_string().parse().unwrap_or(1),
            day: now.format("%d").to_string().parse().unwrap_or(1),
            hour: 0,
            minute: 0,
            segment: 0,
            has_time: false,
            enabled: true,
            typing_buf: String::new(),
        }
    }

    pub(crate) fn date_time_disabled() -> Self {
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

    pub(crate) fn date_time_now() -> Self {
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

    pub(crate) fn from_iso(s: &str, has_time: bool) -> Self {
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

    pub(crate) fn from_date_time_strings(date: &str, time: &str) -> Self {
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

    pub(crate) fn max_day(&self) -> u8 {
        jinx::proximos::days_in_month(self.year as u32, self.month as u32) as u8
    }

    pub(crate) fn clamp(&mut self) {
        self.year = self.year.clamp(2020, 2099);
        self.month = self.month.clamp(1, 12);
        self.day = self.day.clamp(1, self.max_day());
        self.hour = self.hour.min(23);
        self.minute = self.minute.min(59);
    }

    pub(crate) fn n_segments(&self) -> usize {
        if self.has_time { 5 } else { 3 }
    }

    pub(crate) fn to_date_string(&self) -> Option<String> {
        if !self.enabled { return None; }
        Some(format!("{:04}-{:02}-{:02}", self.year, self.month, self.day))
    }

    pub(crate) fn to_time_string(&self) -> String {
        format!("{:02}:{:02}", self.hour, self.minute)
    }

    pub(crate) fn to_iso_string(&self) -> Option<String> {
        if !self.enabled { return None; }
        Some(format!(
            "{:04}-{:02}-{:02}T{:02}:{:02}:00+00:00",
            self.year, self.month, self.day, self.hour, self.minute
        ))
    }

    pub(crate) fn commit_typing_buf(&mut self) {
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

    pub(crate) fn segment_max_digits(&self) -> usize {
        if self.segment == 0 { 4 } else { 2 }
    }

    pub(crate) fn handle_key(&mut self, code: KeyCode) -> DateInputResult {
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

pub(crate) fn date_input_line(label: &str, input: &DateTimeInput, field_active: bool, hint_active: &str, hint_inactive: &str, hint_controls: &str) -> Line<'static> {
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
// Tareas panel sub-section and filter state
// ---------------------------------------------------------------------------

#[derive(Default, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TareasSection {
    #[default]
    Tasks,
    Groups,
}

#[derive(Default, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NotesView {
    #[default]
    List,
    Preview,
    Edit,
}

#[derive(Clone)]
pub(crate) struct ActiveTaskFilter {
    pub(crate) status: Option<TaskStatus>,
    pub(crate) group_id: Option<Option<i64>>,
    pub(crate) priorities: Vec<Priority>,
    pub(crate) from_date: Option<String>,
    pub(crate) to_date: Option<String>,
    pub(crate) no_deadline: bool,
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
    pub(crate) fn to_storage_filter(&self) -> TaskFilter {
        TaskFilter {
            status: self.status,
            group_id: self.group_id,
            from_date: self.from_date.clone(),
            to_date: self.to_date.clone(),
            no_deadline: self.no_deadline,
        }
    }

    pub(crate) fn is_default(&self) -> bool {
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

pub(crate) struct RuntimeState {
    pub(crate) app: AppState,
    pub(crate) locale: jinx::locale::Locale,
    pub(crate) chat_history: Vec<ChatMsg>,
    pub(crate) chat_editor: TextEditor,
    pub(crate) chat_scroll: usize, // lines from bottom; 0 = pinned to bottom
    pub(crate) prompt_history: Vec<String>,
    pub(crate) prompt_history_idx: Option<usize>,
    pub(crate) prompt_stash: String,
    pub(crate) task_cursor: usize,
    pub(crate) tareas_scroll: usize,
    pub(crate) tareas_search_active: bool,
    pub(crate) tareas_search_query: String,
    pub(crate) calendar_cursor: usize,
    pub(crate) calendar_scroll: usize,
    pub(crate) calendar_scroll_initialized: bool,
    pub(crate) calendar_filter_idx: usize, // 0=all, 1=today, 2=this week, 3=this month
    pub(crate) group_cursor: usize,
    pub(crate) tareas_section: TareasSection,
    pub(crate) tareas_filter: ActiveTaskFilter,
    pub(crate) color_mode: ColorMode,
    pub(crate) visible_panels: [bool; 5],
    pub(crate) services: AppServices,
    pub(crate) agent_child: Option<Child>,
    pub(crate) agent_stdin: Option<ChildStdin>,
    pub(crate) agent_rx: Option<mpsc::Receiver<Envelope>>,
    pub(crate) pending_request: Option<(Uuid, Instant)>,
    // Google Calendar sync daemon
    // Modal form state
    pub(crate) task_form: TaskFormState,
    pub(crate) event_form: EventFormState,
    pub(crate) group_form: GroupFormState,
    pub(crate) settings_form: SettingsFormState,
    pub(crate) filter_form: FilterFormState,
    pub(crate) groups_cache: Vec<Group>,
    pub(crate) delete_confirm_name: String,
    pub(crate) pending_g: bool,
    // Notes panel state
    pub(crate) notes_cache: Vec<jinx_core::Note>,
    pub(crate) notes_cursor: usize,
    pub(crate) notes_scroll: usize,
    pub(crate) notes_view: NotesView,
    pub(crate) notes_editor: TextEditor,
    pub(crate) notes_title_editor: TextEditor,
    pub(crate) notes_title_focused: bool,
    pub(crate) notes_search_active: bool,
    pub(crate) notes_search_query: String,
    pub(crate) notes_current_id: Option<i64>,
    pub(crate) notes_preview_scroll: usize,
    pub(crate) notes_editor_scroll: usize,
    pub(crate) notes_undo_stack: Vec<(Vec<String>, usize, usize)>,
    pub(crate) notes_redo_stack: Vec<(Vec<String>, usize, usize)>,
    pub(crate) notes_pending_g: bool,
    // Note picker (interactive results in chat)
    pub(crate) last_note_results: Option<Vec<NotePickerEntry>>,
    pub(crate) note_picker_active: bool,
    pub(crate) note_picker_cursor: usize,
    pub(crate) note_picker_msg_idx: Option<usize>,
    // Directory browser (export note)
    pub(crate) dir_browser: DirBrowserState,
    // Slash-command picker
    pub(crate) cmd_picker_active: bool,
    pub(crate) cmd_picker_cursor: usize,
    pub(crate) cmd_picker_filtered: Vec<usize>,
    // Finance panel
    pub(crate) finance_month: String,
    pub(crate) finance_categories: Vec<jinx_core::FinCategory>,
    pub(crate) transaction_form: TransactionFormState,
    pub(crate) debt_form: DebtFormState,
    pub(crate) goal_form: GoalFormState,
    pub(crate) budget_form: BudgetFormState,
    // Layout rects for mouse hit-testing
    pub(crate) panel_area: Option<Rect>,
    pub(crate) input_area: Option<Rect>,
    pub(crate) history_area: Option<Rect>,
}
