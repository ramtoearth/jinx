use std::sync::Arc;

use domain::{
    DomainError, Task,
    task::{NewTask, TaskFilter, TaskPatch, TaskRepository},
};

pub struct TaskService {
    repo: Arc<dyn TaskRepository>,
}

impl TaskService {
    pub fn new(repo: Arc<dyn TaskRepository>) -> Self {
        Self { repo }
    }

    pub fn list(&self, filter: TaskFilter) -> Result<Vec<Task>, DomainError> {
        self.repo.list_tasks(filter)
    }

    pub fn search(&self, query: &str) -> Result<Vec<Task>, DomainError> {
        self.repo.search_tasks(query)
    }

    pub fn create(&self, input: NewTask) -> Result<Task, DomainError> {
        self.repo.create_task(input)
    }

    pub fn update(&self, id: i64, patch: TaskPatch) -> Result<Task, DomainError> {
        self.repo.update_task(id, patch)
    }

    pub fn complete(&self, id: i64) -> Result<Task, DomainError> {
        self.repo.complete_task(id)
    }

    pub fn delete(&self, id: i64) -> Result<(), DomainError> {
        self.repo.delete_task(id)
    }
}
