#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Event {
    pub id: i64,
    pub title: String,
    pub start_date: String,
    pub start_time: String,
    pub duration_minutes: Option<u32>,
    pub group_id: Option<i64>,
}
