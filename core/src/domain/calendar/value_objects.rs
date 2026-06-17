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
