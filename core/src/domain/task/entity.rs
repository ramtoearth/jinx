use serde::{Deserialize, Serialize};

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Task {
    pub id: i64,
    pub title: String,
    pub priority: Priority,
    pub status: TaskStatus,
    pub created_at: String,
    pub deadline: Option<String>,
    pub group_id: Option<i64>,
}
