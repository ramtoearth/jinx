use super::entity::{Priority, TaskStatus};

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
    pub deadline: Option<Option<String>>,
    pub group_id: Option<Option<i64>>,
}

#[derive(Debug, Clone, Default)]
pub struct TaskFilter {
    pub status: Option<TaskStatus>,
    pub group_id: Option<Option<i64>>,
    pub from_date: Option<String>,
    pub to_date: Option<String>,
    pub no_deadline: bool,
}
