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
