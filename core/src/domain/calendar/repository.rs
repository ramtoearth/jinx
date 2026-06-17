use crate::DomainError;
use super::entity::Event;
use super::value_objects::{EventPatch, NewEvent};

pub trait EventRepository: Send + Sync {
    fn list_events(
        &self,
        from_date: Option<&str>,
        to_date: Option<&str>,
    ) -> Result<Vec<Event>, DomainError>;
    fn create_event(&self, input: NewEvent) -> Result<Event, DomainError>;
    fn update_event(&self, id: i64, patch: EventPatch) -> Result<Event, DomainError>;
    fn delete_event(&self, id: i64) -> Result<(), DomainError>;
}
