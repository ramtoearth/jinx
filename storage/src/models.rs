//! Domain model structs, enums, and input/patch types.

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// HexColor
// ---------------------------------------------------------------------------

/// A validated `#RRGGBB` hex colour string.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HexColor(String);

static HEX_RE: std::sync::OnceLock<regex_lite::Regex> = std::sync::OnceLock::new();

impl HexColor {
    /// Create a `HexColor`, returning an error if the string is not a valid
    /// `#RRGGBB` hex colour.
    pub fn new(s: impl Into<String>) -> Result<Self, String> {
        let s = s.into();
        let re = HEX_RE.get_or_init(|| {
            regex_lite::Regex::new(r"^#[0-9A-Fa-f]{6}$").expect("valid hex regex")
        });
        if re.is_match(&s) {
            Ok(Self(s))
        } else {
            Err(format!("invalid hex colour: {s:?} (expected #RRGGBB)"))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for HexColor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

// ---------------------------------------------------------------------------
// Enums
// ---------------------------------------------------------------------------

/// Task priority — maps to the SQLite `CHECK` constraint values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Priority {
    Alta,
    Media,
    Baja,
}

impl Priority {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Alta => "alta",
            Self::Media => "media",
            Self::Baja => "baja",
        }
    }
}

impl std::str::FromStr for Priority {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "alta" => Ok(Self::Alta),
            "media" => Ok(Self::Media),
            "baja" => Ok(Self::Baja),
            _ => Err(format!("unknown priority: {s:?}")),
        }
    }
}

/// Task status — maps to the SQLite `CHECK` constraint values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TaskStatus {
    Pendiente,
    Completada,
    Cancelada,
}

impl TaskStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pendiente => "pendiente",
            Self::Completada => "completada",
            Self::Cancelada => "cancelada",
        }
    }
}

impl std::str::FromStr for TaskStatus {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "pendiente" => Ok(Self::Pendiente),
            "completada" => Ok(Self::Completada),
            "cancelada" => Ok(Self::Cancelada),
            _ => Err(format!("unknown task status: {s:?}")),
        }
    }
}

// ---------------------------------------------------------------------------
// Entity structs
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Group {
    pub id: i64,
    pub name: String,
    pub color: HexColor,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Task {
    pub id: i64,
    pub title: String,
    pub priority: Priority,
    pub status: TaskStatus,
    /// ISO 8601 timestamp with timezone offset.
    pub created_at: String,
    /// ISO 8601 timestamp with timezone offset; `None` when no deadline.
    pub deadline: Option<String>,
    pub group_id: Option<i64>,
    pub google_event_id: Option<String>,
    pub push_pending: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Event {
    pub id: i64,
    pub title: String,
    /// `YYYY-MM-DD`
    pub start_date: String,
    /// `HH:MM`
    pub start_time: String,
    pub duration_minutes: Option<u32>,
    pub group_id: Option<i64>,
    pub google_event_id: Option<String>,
    pub push_pending: bool,
}

// ---------------------------------------------------------------------------
// Input types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct NewTask {
    pub title: String,
    pub priority: Option<Priority>,
    pub deadline: Option<String>,
    pub group_id: Option<i64>,
}

#[derive(Debug, Clone, Default)]
pub struct TaskPatch {
    pub title: Option<String>,
    pub priority: Option<Priority>,
    pub status: Option<TaskStatus>,
    /// `Some(None)` sets deadline to NULL; `None` leaves it unchanged.
    pub deadline: Option<Option<String>>,
    /// `Some(None)` sets group_id to NULL; `None` leaves it unchanged.
    pub group_id: Option<Option<i64>>,
}

#[derive(Debug, Clone)]
pub struct NewEvent {
    pub title: String,
    pub start_date: String,
    pub start_time: String,
    pub duration_minutes: Option<u32>,
    pub group_id: Option<i64>,
}

#[derive(Debug, Clone, Default)]
pub struct EventPatch {
    pub title: Option<String>,
    pub start_date: Option<String>,
    pub start_time: Option<String>,
    pub duration_minutes: Option<Option<u32>>,
    pub group_id: Option<Option<i64>>,
}

#[derive(Debug, Clone)]
pub struct NewGroup {
    pub name: String,
    pub color: HexColor,
}

#[derive(Debug, Clone, Default)]
pub struct GroupPatch {
    pub name: Option<String>,
    pub color: Option<HexColor>,
}

// ---------------------------------------------------------------------------
// Filter types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub struct TaskFilter {
    pub status: Option<TaskStatus>,
    pub group_id: Option<Option<i64>>,
    pub from_date: Option<String>,
    pub to_date: Option<String>,
}

// ---------------------------------------------------------------------------
// Note
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Note {
    pub id: i64,
    pub title: String,
    pub body: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone)]
pub struct NewNote {
    pub title: String,
    pub body: String,
}

#[derive(Debug, Clone, Default)]
pub struct NotePatch {
    pub title: Option<String>,
    pub body: Option<String>,
}

// ---------------------------------------------------------------------------
// Snapshot for group inference
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct GroupInfo {
    pub id: i64,
    pub name: String,
    /// Titles of all Tasks and Events associated with this Group.
    pub member_titles: Vec<String>,
}

/// Lightweight snapshot of Groups fed to the inference engine.
pub type GroupsSnapshot = Vec<GroupInfo>;
