use crate::DomainError;
use crate::shared::HexColor;
use super::entity::{Group, GroupsSnapshot};
use super::value_objects::NewGroup;

pub trait GroupRepository: Send + Sync {
    fn list_groups(&self) -> Result<Vec<Group>, DomainError>;
    fn find_group_by_name(&self, name: &str) -> Result<Option<Group>, DomainError>;
    fn create_group(&self, input: NewGroup) -> Result<Group, DomainError>;
    fn rename_group(&self, id: i64, name: String) -> Result<Group, DomainError>;
    fn recolor_group(&self, id: i64, color: HexColor) -> Result<Group, DomainError>;
    fn delete_group(&self, id: i64) -> Result<(), DomainError>;
    fn snapshot_for_inference(&self) -> Result<GroupsSnapshot, DomainError>;
}
