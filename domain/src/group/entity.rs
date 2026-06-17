use crate::shared::HexColor;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Group {
    pub id: i64,
    pub name: String,
    pub color: HexColor,
}

#[derive(Debug, Clone)]
pub struct GroupInfo {
    pub id: i64,
    pub name: String,
    pub member_titles: Vec<String>,
}

pub type GroupsSnapshot = Vec<GroupInfo>;
