use crate::DomainError;
use super::entity::Task;
use super::value_objects::{NewTask, TaskFilter, TaskPatch};

pub trait TaskRepository: Send + Sync {
    fn list_tasks(&self, filter: TaskFilter) -> Result<Vec<Task>, DomainError>;
    fn search_tasks(&self, query: &str) -> Result<Vec<Task>, DomainError>;
    fn create_task(&self, input: NewTask) -> Result<Task, DomainError>;
    fn update_task(&self, id: i64, patch: TaskPatch) -> Result<Task, DomainError>;
    fn complete_task(&self, id: i64) -> Result<Task, DomainError>;
    fn delete_task(&self, id: i64) -> Result<(), DomainError>;
}
