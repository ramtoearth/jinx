use std::sync::Arc;

use domain::{
    DomainError, Group, GroupsSnapshot, HexColor,
    group::{GroupRepository, NewGroup},
};

pub struct GroupService {
    repo: Arc<dyn GroupRepository>,
}

impl GroupService {
    pub fn new(repo: Arc<dyn GroupRepository>) -> Self {
        Self { repo }
    }

    pub fn list(&self) -> Result<Vec<Group>, DomainError> {
        self.repo.list_groups()
    }

    pub fn find_by_name(&self, name: &str) -> Result<Option<Group>, DomainError> {
        self.repo.find_group_by_name(name)
    }

    pub fn create(&self, input: NewGroup) -> Result<Group, DomainError> {
        self.repo.create_group(input)
    }

    pub fn rename(&self, id: i64, name: String) -> Result<Group, DomainError> {
        self.repo.rename_group(id, name)
    }

    pub fn recolor(&self, id: i64, color: HexColor) -> Result<Group, DomainError> {
        self.repo.recolor_group(id, color)
    }

    pub fn delete(&self, id: i64) -> Result<(), DomainError> {
        self.repo.delete_group(id)
    }

    pub fn snapshot_for_inference(&self) -> Result<GroupsSnapshot, DomainError> {
        self.repo.snapshot_for_inference()
    }
}
