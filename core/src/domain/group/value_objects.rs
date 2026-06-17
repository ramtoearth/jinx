use crate::shared::HexColor;

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
