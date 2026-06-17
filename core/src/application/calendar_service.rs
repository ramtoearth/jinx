use std::sync::Arc;

use crate::domain::{
    DomainError, Event,
    calendar::{EventPatch, EventRepository, NewEvent},
};

pub struct CalendarService {
    repo: Arc<dyn EventRepository>,
}

impl CalendarService {
    pub fn new(repo: Arc<dyn EventRepository>) -> Self {
        Self { repo }
    }

    pub fn list(&self, from_date: Option<&str>, to_date: Option<&str>) -> Result<Vec<Event>, DomainError> {
        self.repo.list_events(from_date, to_date)
    }

    pub fn create(&self, input: NewEvent) -> Result<Event, DomainError> {
        self.repo.create_event(input)
    }

    pub fn update(&self, id: i64, patch: EventPatch) -> Result<Event, DomainError> {
        self.repo.update_event(id, patch)
    }

    pub fn delete(&self, id: i64) -> Result<(), DomainError> {
        self.repo.delete_event(id)
    }
}
