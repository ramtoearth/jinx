//! IPC handler — dispatches `storage.*` requests from the Agent to the
//! `Storage` layer and sends responses back via the Canal_IPC.
//!
//! This module runs in the IPC reader thread (see `main.rs` startup).

use std::sync::Arc;

use storage::{
    EventPatch, HexColor, NewEvent, NewGroup, NewNote, NewTask, NotePatch, Priority, Storage,
    StorageError, TaskFilter, TaskPatch, TaskStatus,
};

use crate::ipc::{
    Envelope, Kind, MessageType, StorageCompleteTaskRequest, StorageCompleteTaskResponse,
    StorageCreateEventRequest, StorageCreateEventResponse, StorageCreateGroupRequest,
    StorageCreateGroupResponse, StorageCreateNoteRequest, StorageCreateNoteResponse,
    StorageCreateTaskRequest, StorageCreateTaskResponse, StorageDeleteEventRequest,
    StorageDeleteEventResponse, StorageDeleteGroupRequest, StorageDeleteGroupResponse,
    StorageDeleteNoteRequest, StorageDeleteNoteResponse, StorageDeleteTaskRequest,
    StorageDeleteTaskResponse, StorageError as IpcError, StorageEventDto,
    StorageExportMarkdownRequest, StorageExportMarkdownResponse, StorageExportSqliteRequest,
    StorageExportSqliteResponse, StorageGroupDto, StorageListEventsRequest,
    StorageListEventsResponse, StorageListGroupsResponse, StorageListNotesResponse,
    StorageListTasksRequest, StorageListTasksResponse, StorageNoteDto, StorageRenameGroupRequest,
    StorageRenameGroupResponse, StorageRecolorGroupRequest, StorageRecolorGroupResponse,
    StorageSearchNotesRequest, StorageSearchNotesResponse, StorageTaskDto,
    StorageUpdateEventRequest, StorageUpdateEventResponse, StorageUpdateNoteRequest,
    StorageUpdateNoteResponse, StorageUpdateTaskRequest, StorageUpdateTaskResponse,
};

// ---------------------------------------------------------------------------
// DTO conversions
// ---------------------------------------------------------------------------

fn task_to_dto(t: storage::Task) -> StorageTaskDto {
    use crate::ipc::{Priority as IpcPriority, TaskStatus as IpcTaskStatus};
    StorageTaskDto {
        id: t.id,
        title: t.title,
        priority: match t.priority {
            Priority::Alta => IpcPriority::Alta,
            Priority::Media => IpcPriority::Media,
            Priority::Baja => IpcPriority::Baja,
        },
        status: match t.status {
            TaskStatus::Pendiente => IpcTaskStatus::Pendiente,
            TaskStatus::Completada => IpcTaskStatus::Completada,
            TaskStatus::Cancelada => IpcTaskStatus::Cancelada,
        },
        created_at: t.created_at,
        deadline: t.deadline,
        group_id: t.group_id,
    }
}

fn event_to_dto(e: storage::Event) -> StorageEventDto {
    StorageEventDto {
        id: e.id,
        title: e.title,
        start_date: e.start_date,
        start_time: e.start_time,
        duration_minutes: e.duration_minutes,
        group_id: e.group_id,
    }
}

fn group_to_dto(g: storage::Group) -> StorageGroupDto {
    StorageGroupDto {
        id: g.id,
        name: g.name,
        color: g.color.to_string(),
    }
}

fn note_to_dto(n: storage::Note) -> StorageNoteDto {
    StorageNoteDto {
        id: n.id,
        title: n.title,
        body: n.body,
        created_at: n.created_at,
        updated_at: n.updated_at,
    }
}

fn storage_err_to_ipc(e: StorageError) -> IpcError {
    IpcError {
        code: e.code().to_string(),
        message: e.message(),
    }
}

// ---------------------------------------------------------------------------
// Dispatcher
// ---------------------------------------------------------------------------

/// Handle a single `storage.*` request envelope and return a response envelope.
pub fn handle_storage_request(
    envelope: &Envelope,
    storage: &Arc<dyn Storage + Send + Sync>,
) -> Envelope {
    let response_base = Envelope::new_empty(Kind::Response, envelope.message_type)
        .with_ref(envelope.id);

    match envelope.message_type {
        // ------------------------------------------------------------------
        // Tasks
        // ------------------------------------------------------------------
        MessageType::StorageListTasks => {
            let req: StorageListTasksRequest = envelope
                .payload_as()
                .unwrap_or(None)
                .unwrap_or(None)
                .unwrap_or_default();
            use crate::ipc::TaskStatus as IpcStatus;
            let filter = TaskFilter {
                status: req.status.map(|s| match s {
                    IpcStatus::Pendiente => TaskStatus::Pendiente,
                    IpcStatus::Completada => TaskStatus::Completada,
                    IpcStatus::Cancelada => TaskStatus::Cancelada,
                }),
                group_id: req.group_id,
                from_date: req.from_date,
                to_date: req.to_date,
            };
            match storage.list_tasks(filter) {
                Ok(tasks) => response_base.clone()
                    .with_payload(&StorageListTasksResponse {
                        tasks: tasks.into_iter().map(task_to_dto).collect(),
                    })
                    .unwrap_or(response_base),
                Err(e) => response_base.with_error(storage_err_to_ipc(e)),
            }
        }

        MessageType::StorageCreateTask => {
            let req: Option<StorageCreateTaskRequest> =
                envelope.payload_as().unwrap_or(None).unwrap_or(None);
            match req {
                None => response_base.with_error(IpcError::new("VALIDATION_FAILED", "missing payload")),
                Some(r) => {
                    use crate::ipc::Priority as IpcPriority;
                    let input = NewTask {
                        title: r.title,
                        priority: r.priority.map(|p| match p {
                            IpcPriority::Alta => Priority::Alta,
                            IpcPriority::Media => Priority::Media,
                            IpcPriority::Baja => Priority::Baja,
                        }),
                        deadline: r.deadline,
                        group_id: r.group_id,
                    };
                    match storage.create_task(input) {
                        Ok(t) => response_base.clone()
                            .with_payload(&StorageCreateTaskResponse { task: task_to_dto(t) })
                            .unwrap_or(response_base),
                        Err(e) => response_base.with_error(storage_err_to_ipc(e)),
                    }
                }
            }
        }

        MessageType::StorageUpdateTask => {
            let req: Option<StorageUpdateTaskRequest> =
                envelope.payload_as().unwrap_or(None).unwrap_or(None);
            match req {
                None => response_base.with_error(IpcError::new("VALIDATION_FAILED", "missing payload")),
                Some(r) => {
                    use crate::ipc::{Priority as IpcPriority, TaskStatus as IpcStatus};
                    let patch = TaskPatch {
                        title: r.patch.title,
                        priority: r.patch.priority.map(|p| match p {
                            IpcPriority::Alta => Priority::Alta,
                            IpcPriority::Media => Priority::Media,
                            IpcPriority::Baja => Priority::Baja,
                        }),
                        status: r.patch.status.map(|s| match s {
                            IpcStatus::Pendiente => TaskStatus::Pendiente,
                            IpcStatus::Completada => TaskStatus::Completada,
                            IpcStatus::Cancelada => TaskStatus::Cancelada,
                        }),
                        deadline: r.patch.deadline,
                        group_id: r.patch.group_id,
                    };
                    match storage.update_task(r.id, patch) {
                        Ok(t) => response_base.clone()
                            .with_payload(&StorageUpdateTaskResponse { task: task_to_dto(t) })
                            .unwrap_or(response_base),
                        Err(e) => response_base.with_error(storage_err_to_ipc(e)),
                    }
                }
            }
        }

        MessageType::StorageCompleteTask => {
            let req: Option<StorageCompleteTaskRequest> =
                envelope.payload_as().unwrap_or(None).unwrap_or(None);
            match req {
                None => response_base.with_error(IpcError::new("VALIDATION_FAILED", "missing payload")),
                Some(r) => match storage.complete_task(r.id) {
                    Ok(t) => response_base.clone()
                        .with_payload(&StorageCompleteTaskResponse { task: task_to_dto(t) })
                        .unwrap_or(response_base),
                    Err(e) => response_base.with_error(storage_err_to_ipc(e)),
                },
            }
        }

        MessageType::StorageDeleteTask => {
            let req: Option<StorageDeleteTaskRequest> =
                envelope.payload_as().unwrap_or(None).unwrap_or(None);
            match req {
                None => response_base.with_error(IpcError::new("VALIDATION_FAILED", "missing payload")),
                Some(r) => match storage.delete_task(r.id) {
                    Ok(()) => response_base.clone()
                        .with_payload(&StorageDeleteTaskResponse {})
                        .unwrap_or(response_base),
                    Err(e) => response_base.with_error(storage_err_to_ipc(e)),
                },
            }
        }

        // ------------------------------------------------------------------
        // Events
        // ------------------------------------------------------------------
        MessageType::StorageListEvents => {
            let req: StorageListEventsRequest = envelope
                .payload_as()
                .unwrap_or(None)
                .unwrap_or(None)
                .unwrap_or_default();
            match storage.list_events(req.from_date.as_deref(), req.to_date.as_deref()) {
                Ok(events) => response_base.clone()
                    .with_payload(&StorageListEventsResponse {
                        events: events.into_iter().map(event_to_dto).collect(),
                    })
                    .unwrap_or(response_base),
                Err(e) => response_base.with_error(storage_err_to_ipc(e)),
            }
        }

        MessageType::StorageCreateEvent => {
            let req: Option<StorageCreateEventRequest> =
                envelope.payload_as().unwrap_or(None).unwrap_or(None);
            match req {
                None => response_base.with_error(IpcError::new("VALIDATION_FAILED", "missing payload")),
                Some(r) => {
                    let input = NewEvent {
                        title: r.title,
                        start_date: r.start_date,
                        start_time: r.start_time,
                        duration_minutes: r.duration_minutes,
                        group_id: r.group_id,
                    };
                    match storage.create_event(input) {
                        Ok(e) => response_base.clone()
                            .with_payload(&StorageCreateEventResponse { event: event_to_dto(e) })
                            .unwrap_or(response_base),
                        Err(e) => response_base.with_error(storage_err_to_ipc(e)),
                    }
                }
            }
        }

        MessageType::StorageUpdateEvent => {
            let req: Option<StorageUpdateEventRequest> =
                envelope.payload_as().unwrap_or(None).unwrap_or(None);
            match req {
                None => response_base.with_error(IpcError::new("VALIDATION_FAILED", "missing payload")),
                Some(r) => {
                    let patch = EventPatch {
                        title: r.patch.title,
                        start_date: r.patch.start_date,
                        start_time: r.patch.start_time,
                        duration_minutes: r.patch.duration_minutes,
                        group_id: r.patch.group_id,
                    };
                    match storage.update_event(r.id, patch) {
                        Ok(e) => response_base.clone()
                            .with_payload(&StorageUpdateEventResponse { event: event_to_dto(e) })
                            .unwrap_or(response_base),
                        Err(e) => response_base.with_error(storage_err_to_ipc(e)),
                    }
                }
            }
        }

        MessageType::StorageDeleteEvent => {
            let req: Option<StorageDeleteEventRequest> =
                envelope.payload_as().unwrap_or(None).unwrap_or(None);
            match req {
                None => response_base.with_error(IpcError::new("VALIDATION_FAILED", "missing payload")),
                Some(r) => match storage.delete_event(r.id) {
                    Ok(()) => response_base.clone()
                        .with_payload(&StorageDeleteEventResponse {})
                        .unwrap_or(response_base),
                    Err(e) => response_base.with_error(storage_err_to_ipc(e)),
                },
            }
        }

        // ------------------------------------------------------------------
        // Groups
        // ------------------------------------------------------------------
        MessageType::StorageListGroups => {
            match storage.list_groups() {
                Ok(groups) => response_base.clone()
                    .with_payload(&StorageListGroupsResponse {
                        groups: groups.into_iter().map(group_to_dto).collect(),
                    })
                    .unwrap_or(response_base),
                Err(e) => response_base.with_error(storage_err_to_ipc(e)),
            }
        }

        MessageType::StorageCreateGroup => {
            let req: Option<StorageCreateGroupRequest> =
                envelope.payload_as().unwrap_or(None).unwrap_or(None);
            match req {
                None => response_base.with_error(IpcError::new("VALIDATION_FAILED", "missing payload")),
                Some(r) => {
                    match HexColor::new(r.color) {
                        Err(msg) => response_base.with_error(IpcError::new("VALIDATION_FAILED", msg)),
                        Ok(color) => match storage.create_group(NewGroup { name: r.name, color }) {
                            Ok(g) => response_base.clone()
                                .with_payload(&StorageCreateGroupResponse { group: group_to_dto(g) })
                                .unwrap_or(response_base),
                            Err(e) => response_base.with_error(storage_err_to_ipc(e)),
                        },
                    }
                }
            }
        }

        MessageType::StorageRenameGroup => {
            let req: Option<StorageRenameGroupRequest> =
                envelope.payload_as().unwrap_or(None).unwrap_or(None);
            match req {
                None => response_base.with_error(IpcError::new("VALIDATION_FAILED", "missing payload")),
                Some(r) => match storage.rename_group(r.id, r.name) {
                    Ok(g) => response_base.clone()
                        .with_payload(&StorageRenameGroupResponse { group: group_to_dto(g) })
                        .unwrap_or(response_base),
                    Err(e) => response_base.with_error(storage_err_to_ipc(e)),
                },
            }
        }

        MessageType::StorageRecolorGroup => {
            let req: Option<StorageRecolorGroupRequest> =
                envelope.payload_as().unwrap_or(None).unwrap_or(None);
            match req {
                None => response_base.with_error(IpcError::new("VALIDATION_FAILED", "missing payload")),
                Some(r) => {
                    match HexColor::new(r.color) {
                        Err(msg) => response_base.with_error(IpcError::new("VALIDATION_FAILED", msg)),
                        Ok(color) => match storage.recolor_group(r.id, color) {
                            Ok(g) => response_base.clone()
                                .with_payload(&StorageRecolorGroupResponse { group: group_to_dto(g) })
                                .unwrap_or(response_base),
                            Err(e) => response_base.with_error(storage_err_to_ipc(e)),
                        },
                    }
                }
            }
        }

        MessageType::StorageDeleteGroup => {
            let req: Option<StorageDeleteGroupRequest> =
                envelope.payload_as().unwrap_or(None).unwrap_or(None);
            match req {
                None => response_base.with_error(IpcError::new("VALIDATION_FAILED", "missing payload")),
                Some(r) => match storage.delete_group(r.id) {
                    Ok(()) => response_base.clone()
                        .with_payload(&StorageDeleteGroupResponse {})
                        .unwrap_or(response_base),
                    Err(e) => response_base.with_error(storage_err_to_ipc(e)),
                },
            }
        }

        // ------------------------------------------------------------------
        // Export
        // ------------------------------------------------------------------
        MessageType::StorageExportMarkdown => {
            let req: Option<StorageExportMarkdownRequest> =
                envelope.payload_as().unwrap_or(None).unwrap_or(None);
            match req {
                None => response_base.with_error(IpcError::new("VALIDATION_FAILED", "missing payload")),
                Some(r) => {
                    let path = std::path::Path::new(&r.output_path);
                    match storage.export_markdown(path) {
                        Ok(p) => response_base.clone()
                            .with_payload(&StorageExportMarkdownResponse {
                                written_path: p.to_string_lossy().to_string(),
                            })
                            .unwrap_or(response_base),
                        Err(e) => response_base.with_error(storage_err_to_ipc(e)),
                    }
                }
            }
        }

        MessageType::StorageExportSqlite => {
            let req: Option<StorageExportSqliteRequest> =
                envelope.payload_as().unwrap_or(None).unwrap_or(None);
            match req {
                None => response_base.with_error(IpcError::new("VALIDATION_FAILED", "missing payload")),
                Some(r) => {
                    let path = std::path::Path::new(&r.output_path);
                    match storage.export_sqlite(path) {
                        Ok(p) => response_base.clone()
                            .with_payload(&StorageExportSqliteResponse {
                                written_path: p.to_string_lossy().to_string(),
                            })
                            .unwrap_or(response_base),
                        Err(e) => response_base.with_error(storage_err_to_ipc(e)),
                    }
                }
            }
        }

        // ------------------------------------------------------------------
        // Notes
        // ------------------------------------------------------------------
        MessageType::StorageListNotes => {
            match storage.list_notes() {
                Ok(notes) => response_base.clone()
                    .with_payload(&StorageListNotesResponse {
                        notes: notes.into_iter().map(note_to_dto).collect(),
                    })
                    .unwrap_or(response_base),
                Err(e) => response_base.with_error(storage_err_to_ipc(e)),
            }
        }

        MessageType::StorageSearchNotes => {
            let req: Option<StorageSearchNotesRequest> =
                envelope.payload_as().unwrap_or(None).unwrap_or(None);
            match req {
                None => response_base.with_error(IpcError::new("VALIDATION_FAILED", "missing payload")),
                Some(r) => {
                    match storage.search_notes(&r.query) {
                        Ok(notes) => response_base.clone()
                            .with_payload(&StorageSearchNotesResponse {
                                notes: notes.into_iter().map(note_to_dto).collect(),
                            })
                            .unwrap_or(response_base),
                        Err(e) => response_base.with_error(storage_err_to_ipc(e)),
                    }
                }
            }
        }

        MessageType::StorageCreateNote => {
            let req: Option<StorageCreateNoteRequest> =
                envelope.payload_as().unwrap_or(None).unwrap_or(None);
            match req {
                None => response_base.with_error(IpcError::new("VALIDATION_FAILED", "missing payload")),
                Some(r) => {
                    let input = NewNote {
                        title: r.title,
                        body: r.body,
                    };
                    match storage.create_note(input) {
                        Ok(n) => response_base.clone()
                            .with_payload(&StorageCreateNoteResponse { note: note_to_dto(n) })
                            .unwrap_or(response_base),
                        Err(e) => response_base.with_error(storage_err_to_ipc(e)),
                    }
                }
            }
        }

        MessageType::StorageUpdateNote => {
            let req: Option<StorageUpdateNoteRequest> =
                envelope.payload_as().unwrap_or(None).unwrap_or(None);
            match req {
                None => response_base.with_error(IpcError::new("VALIDATION_FAILED", "missing payload")),
                Some(r) => {
                    let patch = NotePatch {
                        title: r.patch.title,
                        body: r.patch.body,
                    };
                    match storage.update_note(r.id, patch) {
                        Ok(n) => response_base.clone()
                            .with_payload(&StorageUpdateNoteResponse { note: note_to_dto(n) })
                            .unwrap_or(response_base),
                        Err(e) => response_base.with_error(storage_err_to_ipc(e)),
                    }
                }
            }
        }

        MessageType::StorageDeleteNote => {
            let req: Option<StorageDeleteNoteRequest> =
                envelope.payload_as().unwrap_or(None).unwrap_or(None);
            match req {
                None => response_base.with_error(IpcError::new("VALIDATION_FAILED", "missing payload")),
                Some(r) => {
                    match storage.delete_note(r.id) {
                        Ok(()) => response_base.clone()
                            .with_payload(&StorageDeleteNoteResponse {})
                            .unwrap_or(response_base),
                        Err(e) => response_base.with_error(storage_err_to_ipc(e)),
                    }
                }
            }
        }

        // Unhandled message types (not storage requests)
        _ => response_base.with_error(IpcError::new(
            "INTERNAL_ERROR",
            "unexpected message type for storage handler",
        )),
    }
}

/// Returns `true` if the given `MessageType` is a `storage.*` request.
pub fn is_storage_request(mt: MessageType) -> bool {
    matches!(
        mt,
        MessageType::StorageListTasks
            | MessageType::StorageCreateTask
            | MessageType::StorageUpdateTask
            | MessageType::StorageCompleteTask
            | MessageType::StorageDeleteTask
            | MessageType::StorageListEvents
            | MessageType::StorageCreateEvent
            | MessageType::StorageUpdateEvent
            | MessageType::StorageDeleteEvent
            | MessageType::StorageListGroups
            | MessageType::StorageCreateGroup
            | MessageType::StorageRenameGroup
            | MessageType::StorageRecolorGroup
            | MessageType::StorageDeleteGroup
            | MessageType::StorageExportMarkdown
            | MessageType::StorageExportSqlite
            | MessageType::StorageListNotes
            | MessageType::StorageSearchNotes
            | MessageType::StorageCreateNote
            | MessageType::StorageUpdateNote
            | MessageType::StorageDeleteNote
    )
}
