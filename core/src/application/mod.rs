pub mod task_service;
pub mod calendar_service;
pub mod note_service;
pub mod group_service;
pub mod export_service;

pub use task_service::TaskService;
pub use calendar_service::CalendarService;
pub use note_service::NoteService;
pub use group_service::GroupService;
pub use export_service::ExportService;

use std::sync::Arc;
use crate::infrastructure::SqliteStorage;

pub struct AppServices {
    pub tasks: TaskService,
    pub calendar: CalendarService,
    pub notes: NoteService,
    pub groups: GroupService,
    pub export: ExportService,
}

impl AppServices {
    pub fn new(storage: Arc<SqliteStorage>) -> Self {
        Self {
            tasks: TaskService::new(storage.clone()),
            calendar: CalendarService::new(storage.clone()),
            notes: NoteService::new(storage.clone()),
            groups: GroupService::new(storage.clone()),
            export: ExportService::new(storage),
        }
    }
}
