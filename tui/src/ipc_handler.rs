//! IPC handler — dispatches `storage.*` requests from the Agent to the
//! `Storage` layer and sends responses back via the Canal_IPC.
//!
//! This module runs in the IPC reader thread (see `main.rs` startup).

use jinx_core::{
    AppServices, DebtPatch, DomainError, EventPatch, GoalPatch, HexColor, NewBudget, NewDebt,
    NewEvent, NewGoal, NewGroup, NewNote, NewRecurringRule, NewTask, NewTransaction, NotePatch,
    Priority, TaskFilter, TaskPatch, TaskStatus, TransactionFilter,
};

use crate::ipc::{
    Envelope, Kind, MessageType, StorageCompleteTaskRequest, StorageCompleteTaskResponse,
    StorageCreateEventRequest, StorageCreateEventResponse, StorageCreateGroupRequest,
    StorageCreateGroupResponse, StorageCreateNoteRequest, StorageCreateNoteResponse,
    StorageCreateTaskRequest, StorageCreateTaskResponse, StorageDeleteEventRequest,
    StorageDeleteEventResponse, StorageDeleteGroupRequest, StorageDeleteGroupResponse,
    StorageDeleteNoteRequest, StorageDeleteNoteResponse, StorageDeleteTaskRequest,
    StorageDeleteTaskResponse, StorageError as IpcError, StorageEventDto,
    StorageExportMarkdownRequest, StorageExportMarkdownResponse, StorageExportNoteRequest,
    StorageExportNoteResponse, StorageExportSqliteRequest,
    StorageExportSqliteResponse, StorageGroupDto, StorageListEventsRequest,
    StorageListEventsResponse, StorageListGroupsResponse, StorageListNotesResponse,
    StorageListTasksRequest, StorageListTasksResponse, StorageNoteDto, StorageRenameGroupRequest,
    StorageRenameGroupResponse, StorageRecolorGroupRequest, StorageRecolorGroupResponse,
    StorageSearchNotesRequest, StorageSearchNotesResponse, StorageSearchTasksRequest,
    StorageSearchTasksResponse, StorageTaskDto,
    StorageUpdateEventRequest, StorageUpdateEventResponse, StorageUpdateNoteRequest,
    StorageUpdateNoteResponse, StorageUpdateTaskRequest, StorageUpdateTaskResponse,
};

// ---------------------------------------------------------------------------
// DTO conversions
// ---------------------------------------------------------------------------

fn task_to_dto(t: jinx_core::Task) -> StorageTaskDto {
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

fn event_to_dto(e: jinx_core::Event) -> StorageEventDto {
    StorageEventDto {
        id: e.id,
        title: e.title,
        start_date: e.start_date,
        start_time: e.start_time,
        duration_minutes: e.duration_minutes,
        group_id: e.group_id,
    }
}

fn group_to_dto(g: jinx_core::Group) -> StorageGroupDto {
    StorageGroupDto {
        id: g.id,
        name: g.name,
        color: g.color.to_string(),
    }
}

fn note_to_dto(n: jinx_core::Note) -> StorageNoteDto {
    StorageNoteDto {
        id: n.id,
        title: n.title,
        body: n.body,
        created_at: n.created_at,
        updated_at: n.updated_at,
    }
}

fn domain_err_to_ipc(e: DomainError) -> IpcError {
    IpcError {
        code: e.code().to_string(),
        message: e.message(),
    }
}

fn tx_to_dto(t: jinx_core::Transaction) -> crate::ipc::FinanceTransactionDto {
    crate::ipc::FinanceTransactionDto {
        id: t.id, amount: t.amount, tx_type: t.tx_type.as_str().to_string(),
        category: t.category, description: t.description, date: t.date,
        recurring_id: t.recurring_id, group_id: t.group_id,
    }
}

fn recurring_to_dto(r: jinx_core::RecurringRule) -> crate::ipc::FinanceRecurringRuleDto {
    crate::ipc::FinanceRecurringRuleDto {
        id: r.id, amount: r.amount, tx_type: r.tx_type.as_str().to_string(),
        category: r.category, description: r.description,
        period: r.period.as_str().to_string(), day_of_month: r.day_of_month,
        next_due: r.next_due, active: r.active, group_id: r.group_id,
    }
}

fn budget_to_dto(b: jinx_core::Budget) -> crate::ipc::FinanceBudgetDto {
    crate::ipc::FinanceBudgetDto {
        id: b.id, category: b.category, monthly_limit: b.monthly_limit, month: b.month,
    }
}

fn debt_to_dto(d: jinx_core::Debt) -> crate::ipc::FinanceDebtDto {
    crate::ipc::FinanceDebtDto {
        id: d.id, creditor: d.creditor, total_amount: d.total_amount,
        remaining_amount: d.remaining_amount, interest_rate: d.interest_rate,
        monthly_payment: d.monthly_payment, due_day: d.due_day, start_date: d.start_date,
    }
}

fn goal_to_dto(g: jinx_core::Goal) -> crate::ipc::FinanceGoalDto {
    crate::ipc::FinanceGoalDto {
        id: g.id, name: g.name, target_amount: g.target_amount,
        current_amount: g.current_amount, deadline: g.deadline,
        horizon: g.horizon.as_str().to_string(),
    }
}

// ---------------------------------------------------------------------------
// Dispatcher
// ---------------------------------------------------------------------------

/// Handle a single `storage.*` request envelope and return a response envelope.
pub fn handle_storage_request(
    envelope: &Envelope,
    services: &AppServices,
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
                no_deadline: false,
            };
            match services.tasks.list(filter) {
                Ok(tasks) => response_base.clone()
                    .with_payload(&StorageListTasksResponse {
                        tasks: tasks.into_iter().map(task_to_dto).collect(),
                    })
                    .unwrap_or(response_base),
                Err(e) => response_base.with_error(domain_err_to_ipc(e)),
            }
        }

        MessageType::StorageSearchTasks => {
            let req: Option<StorageSearchTasksRequest> =
                envelope.payload_as().unwrap_or(None).unwrap_or(None);
            match req {
                None => response_base.with_error(IpcError::new("VALIDATION_FAILED", "missing payload")),
                Some(r) => match services.tasks.search(&r.query) {
                    Ok(tasks) => response_base.clone()
                        .with_payload(&StorageSearchTasksResponse {
                            tasks: tasks.into_iter().map(task_to_dto).collect(),
                        })
                        .unwrap_or(response_base),
                    Err(e) => response_base.with_error(domain_err_to_ipc(e)),
                },
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
                    match services.tasks.create(input) {
                        Ok(t) => response_base.clone()
                            .with_payload(&StorageCreateTaskResponse { task: task_to_dto(t) })
                            .unwrap_or(response_base),
                        Err(e) => response_base.with_error(domain_err_to_ipc(e)),
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
                    match services.tasks.update(r.id, patch) {
                        Ok(t) => response_base.clone()
                            .with_payload(&StorageUpdateTaskResponse { task: task_to_dto(t) })
                            .unwrap_or(response_base),
                        Err(e) => response_base.with_error(domain_err_to_ipc(e)),
                    }
                }
            }
        }

        MessageType::StorageCompleteTask => {
            let req: Option<StorageCompleteTaskRequest> =
                envelope.payload_as().unwrap_or(None).unwrap_or(None);
            match req {
                None => response_base.with_error(IpcError::new("VALIDATION_FAILED", "missing payload")),
                Some(r) => match services.tasks.complete(r.id) {
                    Ok(t) => response_base.clone()
                        .with_payload(&StorageCompleteTaskResponse { task: task_to_dto(t) })
                        .unwrap_or(response_base),
                    Err(e) => response_base.with_error(domain_err_to_ipc(e)),
                },
            }
        }

        MessageType::StorageDeleteTask => {
            let req: Option<StorageDeleteTaskRequest> =
                envelope.payload_as().unwrap_or(None).unwrap_or(None);
            match req {
                None => response_base.with_error(IpcError::new("VALIDATION_FAILED", "missing payload")),
                Some(r) => match services.tasks.delete(r.id) {
                    Ok(()) => response_base.clone()
                        .with_payload(&StorageDeleteTaskResponse {})
                        .unwrap_or(response_base),
                    Err(e) => response_base.with_error(domain_err_to_ipc(e)),
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
            match services.calendar.list(req.from_date.as_deref(), req.to_date.as_deref()) {
                Ok(events) => response_base.clone()
                    .with_payload(&StorageListEventsResponse {
                        events: events.into_iter().map(event_to_dto).collect(),
                    })
                    .unwrap_or(response_base),
                Err(e) => response_base.with_error(domain_err_to_ipc(e)),
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
                    match services.calendar.create(input) {
                        Ok(e) => response_base.clone()
                            .with_payload(&StorageCreateEventResponse { event: event_to_dto(e) })
                            .unwrap_or(response_base),
                        Err(e) => response_base.with_error(domain_err_to_ipc(e)),
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
                    match services.calendar.update(r.id, patch) {
                        Ok(e) => response_base.clone()
                            .with_payload(&StorageUpdateEventResponse { event: event_to_dto(e) })
                            .unwrap_or(response_base),
                        Err(e) => response_base.with_error(domain_err_to_ipc(e)),
                    }
                }
            }
        }

        MessageType::StorageDeleteEvent => {
            let req: Option<StorageDeleteEventRequest> =
                envelope.payload_as().unwrap_or(None).unwrap_or(None);
            match req {
                None => response_base.with_error(IpcError::new("VALIDATION_FAILED", "missing payload")),
                Some(r) => match services.calendar.delete(r.id) {
                    Ok(()) => response_base.clone()
                        .with_payload(&StorageDeleteEventResponse {})
                        .unwrap_or(response_base),
                    Err(e) => response_base.with_error(domain_err_to_ipc(e)),
                },
            }
        }

        // ------------------------------------------------------------------
        // Groups
        // ------------------------------------------------------------------
        MessageType::StorageListGroups => {
            match services.groups.list() {
                Ok(groups) => response_base.clone()
                    .with_payload(&StorageListGroupsResponse {
                        groups: groups.into_iter().map(group_to_dto).collect(),
                    })
                    .unwrap_or(response_base),
                Err(e) => response_base.with_error(domain_err_to_ipc(e)),
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
                        Ok(color) => match services.groups.create(NewGroup { name: r.name, color }) {
                            Ok(g) => response_base.clone()
                                .with_payload(&StorageCreateGroupResponse { group: group_to_dto(g) })
                                .unwrap_or(response_base),
                            Err(e) => response_base.with_error(domain_err_to_ipc(e)),
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
                Some(r) => match services.groups.rename(r.id, r.name) {
                    Ok(g) => response_base.clone()
                        .with_payload(&StorageRenameGroupResponse { group: group_to_dto(g) })
                        .unwrap_or(response_base),
                    Err(e) => response_base.with_error(domain_err_to_ipc(e)),
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
                        Ok(color) => match services.groups.recolor(r.id, color) {
                            Ok(g) => response_base.clone()
                                .with_payload(&StorageRecolorGroupResponse { group: group_to_dto(g) })
                                .unwrap_or(response_base),
                            Err(e) => response_base.with_error(domain_err_to_ipc(e)),
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
                Some(r) => match services.groups.delete(r.id) {
                    Ok(()) => response_base.clone()
                        .with_payload(&StorageDeleteGroupResponse {})
                        .unwrap_or(response_base),
                    Err(e) => response_base.with_error(domain_err_to_ipc(e)),
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
                    match services.export.export_markdown(path) {
                        Ok(p) => response_base.clone()
                            .with_payload(&StorageExportMarkdownResponse {
                                written_path: p.to_string_lossy().to_string(),
                            })
                            .unwrap_or(response_base),
                        Err(e) => response_base.with_error(domain_err_to_ipc(e)),
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
                    match services.export.export_sqlite(path) {
                        Ok(p) => response_base.clone()
                            .with_payload(&StorageExportSqliteResponse {
                                written_path: p.to_string_lossy().to_string(),
                            })
                            .unwrap_or(response_base),
                        Err(e) => response_base.with_error(domain_err_to_ipc(e)),
                    }
                }
            }
        }

        // ------------------------------------------------------------------
        // Notes
        // ------------------------------------------------------------------
        MessageType::StorageListNotes => {
            match services.notes.list() {
                Ok(notes) => response_base.clone()
                    .with_payload(&StorageListNotesResponse {
                        notes: notes.into_iter().map(note_to_dto).collect(),
                    })
                    .unwrap_or(response_base),
                Err(e) => response_base.with_error(domain_err_to_ipc(e)),
            }
        }

        MessageType::StorageSearchNotes => {
            let req: Option<StorageSearchNotesRequest> =
                envelope.payload_as().unwrap_or(None).unwrap_or(None);
            match req {
                None => response_base.with_error(IpcError::new("VALIDATION_FAILED", "missing payload")),
                Some(r) => {
                    match services.notes.search(&r.query) {
                        Ok(notes) => response_base.clone()
                            .with_payload(&StorageSearchNotesResponse {
                                notes: notes.into_iter().map(note_to_dto).collect(),
                            })
                            .unwrap_or(response_base),
                        Err(e) => response_base.with_error(domain_err_to_ipc(e)),
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
                    match services.notes.create(input) {
                        Ok(n) => response_base.clone()
                            .with_payload(&StorageCreateNoteResponse { note: note_to_dto(n) })
                            .unwrap_or(response_base),
                        Err(e) => response_base.with_error(domain_err_to_ipc(e)),
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
                    match services.notes.update(r.id, patch) {
                        Ok(n) => response_base.clone()
                            .with_payload(&StorageUpdateNoteResponse { note: note_to_dto(n) })
                            .unwrap_or(response_base),
                        Err(e) => response_base.with_error(domain_err_to_ipc(e)),
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
                    match services.notes.delete(r.id) {
                        Ok(()) => response_base.clone()
                            .with_payload(&StorageDeleteNoteResponse {})
                            .unwrap_or(response_base),
                        Err(e) => response_base.with_error(domain_err_to_ipc(e)),
                    }
                }
            }
        }

        MessageType::StorageExportNote => {
            let req: Option<StorageExportNoteRequest> =
                envelope.payload_as().unwrap_or(None).unwrap_or(None);
            match req {
                None => response_base.with_error(IpcError::new("VALIDATION_FAILED", "missing payload")),
                Some(r) => {
                    let path = std::path::Path::new(&r.output_path);
                    match services.notes.export(r.id, path) {
                        Ok(p) => response_base.clone()
                            .with_payload(&StorageExportNoteResponse {
                                written_path: p.to_string_lossy().to_string(),
                            })
                            .unwrap_or(response_base),
                        Err(e) => response_base.with_error(domain_err_to_ipc(e)),
                    }
                }
            }
        }

        // ------------------------------------------------------------------
        // Finance
        // ------------------------------------------------------------------
        MessageType::FinanceListTransactions => {
            let req: crate::ipc::FinanceListTransactionsRequest = envelope
                .payload_as().unwrap_or(None).unwrap_or(None).unwrap_or_default();
            let filter = TransactionFilter {
                tx_type: req.tx_type.as_deref().and_then(|s| s.parse().ok()),
                category: req.category,
                from_date: req.from_date,
                to_date: req.to_date,
                group_id: None,
            };
            match services.finance.list_transactions(filter) {
                Ok(txs) => response_base.clone()
                    .with_payload(&crate::ipc::FinanceListTransactionsResponse {
                        transactions: txs.into_iter().map(tx_to_dto).collect(),
                    }).unwrap_or(response_base),
                Err(e) => response_base.with_error(domain_err_to_ipc(e)),
            }
        }

        MessageType::FinanceCreateTransaction => {
            let req: Option<crate::ipc::FinanceCreateTransactionRequest> =
                envelope.payload_as().unwrap_or(None).unwrap_or(None);
            match req {
                None => response_base.with_error(IpcError::new("VALIDATION_FAILED", "missing payload")),
                Some(r) => {
                    let tx_type = match r.tx_type.parse() {
                        Ok(t) => t,
                        Err(_) => return response_base.with_error(IpcError::new("VALIDATION_FAILED", "invalid tx_type")),
                    };
                    match services.finance.create_transaction(NewTransaction {
                        amount: r.amount, tx_type, category: r.category,
                        description: r.description, date: r.date,
                        recurring_id: None, group_id: r.group_id,
                    }) {
                        Ok(t) => response_base.clone()
                            .with_payload(&crate::ipc::FinanceCreateTransactionResponse { transaction: tx_to_dto(t) })
                            .unwrap_or(response_base),
                        Err(e) => response_base.with_error(domain_err_to_ipc(e)),
                    }
                }
            }
        }

        MessageType::FinanceDeleteTransaction => {
            let req: Option<crate::ipc::FinanceDeleteTransactionRequest> =
                envelope.payload_as().unwrap_or(None).unwrap_or(None);
            match req {
                None => response_base.with_error(IpcError::new("VALIDATION_FAILED", "missing payload")),
                Some(r) => match services.finance.delete_transaction(r.id) {
                    Ok(_) => response_base.clone()
                        .with_payload(&crate::ipc::FinanceDeleteTransactionResponse {})
                        .unwrap_or(response_base),
                    Err(e) => response_base.with_error(domain_err_to_ipc(e)),
                },
            }
        }

        MessageType::FinanceMonthlySummary => {
            let req: Option<crate::ipc::FinanceMonthlySummaryRequest> =
                envelope.payload_as().unwrap_or(None).unwrap_or(None);
            match req {
                None => response_base.with_error(IpcError::new("VALIDATION_FAILED", "missing payload")),
                Some(r) => match services.finance.monthly_summary(&r.month) {
                    Ok(s) => response_base.clone()
                        .with_payload(&crate::ipc::FinanceMonthlySummaryResponse {
                            month: s.month, total_income: s.total_income,
                            total_expenses: s.total_expenses, balance: s.balance,
                            savings_rate: s.savings_rate,
                        }).unwrap_or(response_base),
                    Err(e) => response_base.with_error(domain_err_to_ipc(e)),
                },
            }
        }

        MessageType::FinanceListRecurringRules => {
            match services.finance.list_recurring_rules() {
                Ok(rules) => response_base.clone()
                    .with_payload(&crate::ipc::FinanceListRecurringRulesResponse {
                        rules: rules.into_iter().map(recurring_to_dto).collect(),
                    }).unwrap_or(response_base),
                Err(e) => response_base.with_error(domain_err_to_ipc(e)),
            }
        }

        MessageType::FinanceCreateRecurringRule => {
            let req: Option<crate::ipc::FinanceCreateRecurringRuleRequest> =
                envelope.payload_as().unwrap_or(None).unwrap_or(None);
            match req {
                None => response_base.with_error(IpcError::new("VALIDATION_FAILED", "missing payload")),
                Some(r) => {
                    let tx_type = match r.tx_type.parse() {
                        Ok(t) => t,
                        Err(_) => return response_base.with_error(IpcError::new("VALIDATION_FAILED", "invalid tx_type")),
                    };
                    let period = match r.period.parse() {
                        Ok(p) => p,
                        Err(_) => return response_base.with_error(IpcError::new("VALIDATION_FAILED", "invalid period")),
                    };
                    match services.finance.create_recurring_rule(NewRecurringRule {
                        amount: r.amount, tx_type, category: r.category,
                        description: r.description, period, day_of_month: r.day_of_month,
                        next_due: r.next_due, group_id: r.group_id,
                    }) {
                        Ok(rule) => response_base.clone()
                            .with_payload(&crate::ipc::FinanceCreateRecurringRuleResponse { rule: recurring_to_dto(rule) })
                            .unwrap_or(response_base),
                        Err(e) => response_base.with_error(domain_err_to_ipc(e)),
                    }
                }
            }
        }

        MessageType::FinanceDeleteRecurringRule => {
            let req: Option<crate::ipc::FinanceDeleteRecurringRuleRequest> =
                envelope.payload_as().unwrap_or(None).unwrap_or(None);
            match req {
                None => response_base.with_error(IpcError::new("VALIDATION_FAILED", "missing payload")),
                Some(r) => match services.finance.deactivate_recurring_rule(r.id) {
                    Ok(_) => response_base.clone()
                        .with_payload(&crate::ipc::FinanceDeleteRecurringRuleResponse {})
                        .unwrap_or(response_base),
                    Err(e) => response_base.with_error(domain_err_to_ipc(e)),
                },
            }
        }

        MessageType::FinanceListBudgets => {
            let req: Option<crate::ipc::FinanceListBudgetsRequest> =
                envelope.payload_as().unwrap_or(None).unwrap_or(None);
            match req {
                None => response_base.with_error(IpcError::new("VALIDATION_FAILED", "missing payload")),
                Some(r) => match services.finance.list_budgets(&r.month) {
                    Ok(bs) => response_base.clone()
                        .with_payload(&crate::ipc::FinanceListBudgetsResponse {
                            budgets: bs.into_iter().map(budget_to_dto).collect(),
                        }).unwrap_or(response_base),
                    Err(e) => response_base.with_error(domain_err_to_ipc(e)),
                },
            }
        }

        MessageType::FinanceSetBudget => {
            let req: Option<crate::ipc::FinanceSetBudgetRequest> =
                envelope.payload_as().unwrap_or(None).unwrap_or(None);
            match req {
                None => response_base.with_error(IpcError::new("VALIDATION_FAILED", "missing payload")),
                Some(r) => match services.finance.set_budget(NewBudget {
                    category: r.category, monthly_limit: r.monthly_limit, month: r.month,
                }) {
                    Ok(b) => response_base.clone()
                        .with_payload(&crate::ipc::FinanceSetBudgetResponse { budget: budget_to_dto(b) })
                        .unwrap_or(response_base),
                    Err(e) => response_base.with_error(domain_err_to_ipc(e)),
                },
            }
        }

        MessageType::FinanceBudgetStatus => {
            let req: Option<crate::ipc::FinanceBudgetStatusRequest> =
                envelope.payload_as().unwrap_or(None).unwrap_or(None);
            match req {
                None => response_base.with_error(IpcError::new("VALIDATION_FAILED", "missing payload")),
                Some(r) => match services.finance.budget_status(&r.month) {
                    Ok(items) => response_base.clone()
                        .with_payload(&crate::ipc::FinanceBudgetStatusResponse {
                            items: items.into_iter().map(|(b, spent)| crate::ipc::FinanceBudgetStatusItem {
                                category: b.category, monthly_limit: b.monthly_limit, spent,
                            }).collect(),
                        }).unwrap_or(response_base),
                    Err(e) => response_base.with_error(domain_err_to_ipc(e)),
                },
            }
        }

        MessageType::FinanceListDebts => {
            match services.finance.list_debts() {
                Ok(debts) => response_base.clone()
                    .with_payload(&crate::ipc::FinanceListDebtsResponse {
                        debts: debts.into_iter().map(debt_to_dto).collect(),
                    }).unwrap_or(response_base),
                Err(e) => response_base.with_error(domain_err_to_ipc(e)),
            }
        }

        MessageType::FinanceCreateDebt => {
            let req: Option<crate::ipc::FinanceCreateDebtRequest> =
                envelope.payload_as().unwrap_or(None).unwrap_or(None);
            match req {
                None => response_base.with_error(IpcError::new("VALIDATION_FAILED", "missing payload")),
                Some(r) => match services.finance.create_debt(NewDebt {
                    creditor: r.creditor, total_amount: r.total_amount,
                    remaining_amount: r.remaining_amount, interest_rate: r.interest_rate,
                    monthly_payment: r.monthly_payment, due_day: r.due_day, start_date: r.start_date,
                }) {
                    Ok(d) => response_base.clone()
                        .with_payload(&crate::ipc::FinanceCreateDebtResponse { debt: debt_to_dto(d) })
                        .unwrap_or(response_base),
                    Err(e) => response_base.with_error(domain_err_to_ipc(e)),
                },
            }
        }

        MessageType::FinanceUpdateDebt => {
            let req: Option<crate::ipc::FinanceUpdateDebtRequest> =
                envelope.payload_as().unwrap_or(None).unwrap_or(None);
            match req {
                None => response_base.with_error(IpcError::new("VALIDATION_FAILED", "missing payload")),
                Some(r) => match services.finance.update_debt(r.id, DebtPatch {
                    remaining_amount: r.remaining_amount, monthly_payment: r.monthly_payment,
                }) {
                    Ok(d) => response_base.clone()
                        .with_payload(&crate::ipc::FinanceUpdateDebtResponse { debt: debt_to_dto(d) })
                        .unwrap_or(response_base),
                    Err(e) => response_base.with_error(domain_err_to_ipc(e)),
                },
            }
        }

        MessageType::FinanceDeleteDebt => {
            let req: Option<crate::ipc::FinanceDeleteDebtRequest> =
                envelope.payload_as().unwrap_or(None).unwrap_or(None);
            match req {
                None => response_base.with_error(IpcError::new("VALIDATION_FAILED", "missing payload")),
                Some(r) => match services.finance.delete_debt(r.id) {
                    Ok(_) => response_base.clone()
                        .with_payload(&crate::ipc::FinanceDeleteDebtResponse {})
                        .unwrap_or(response_base),
                    Err(e) => response_base.with_error(domain_err_to_ipc(e)),
                },
            }
        }

        MessageType::FinanceListGoals => {
            match services.finance.list_goals() {
                Ok(goals) => response_base.clone()
                    .with_payload(&crate::ipc::FinanceListGoalsResponse {
                        goals: goals.into_iter().map(goal_to_dto).collect(),
                    }).unwrap_or(response_base),
                Err(e) => response_base.with_error(domain_err_to_ipc(e)),
            }
        }

        MessageType::FinanceCreateGoal => {
            let req: Option<crate::ipc::FinanceCreateGoalRequest> =
                envelope.payload_as().unwrap_or(None).unwrap_or(None);
            match req {
                None => response_base.with_error(IpcError::new("VALIDATION_FAILED", "missing payload")),
                Some(r) => {
                    let horizon = match r.horizon.parse() {
                        Ok(h) => h,
                        Err(_) => return response_base.with_error(IpcError::new("VALIDATION_FAILED", "invalid horizon")),
                    };
                    match services.finance.create_goal(NewGoal {
                        name: r.name, target_amount: r.target_amount,
                        current_amount: r.current_amount, deadline: r.deadline, horizon,
                    }) {
                        Ok(g) => response_base.clone()
                            .with_payload(&crate::ipc::FinanceCreateGoalResponse { goal: goal_to_dto(g) })
                            .unwrap_or(response_base),
                        Err(e) => response_base.with_error(domain_err_to_ipc(e)),
                    }
                }
            }
        }

        MessageType::FinanceUpdateGoal => {
            let req: Option<crate::ipc::FinanceUpdateGoalRequest> =
                envelope.payload_as().unwrap_or(None).unwrap_or(None);
            match req {
                None => response_base.with_error(IpcError::new("VALIDATION_FAILED", "missing payload")),
                Some(r) => match services.finance.update_goal(r.id, GoalPatch {
                    current_amount: r.current_amount, target_amount: r.target_amount,
                    deadline: r.deadline,
                }) {
                    Ok(g) => response_base.clone()
                        .with_payload(&crate::ipc::FinanceUpdateGoalResponse { goal: goal_to_dto(g) })
                        .unwrap_or(response_base),
                    Err(e) => response_base.with_error(domain_err_to_ipc(e)),
                },
            }
        }

        MessageType::FinanceDeleteGoal => {
            let req: Option<crate::ipc::FinanceDeleteGoalRequest> =
                envelope.payload_as().unwrap_or(None).unwrap_or(None);
            match req {
                None => response_base.with_error(IpcError::new("VALIDATION_FAILED", "missing payload")),
                Some(r) => match services.finance.delete_goal(r.id) {
                    Ok(_) => response_base.clone()
                        .with_payload(&crate::ipc::FinanceDeleteGoalResponse {})
                        .unwrap_or(response_base),
                    Err(e) => response_base.with_error(domain_err_to_ipc(e)),
                },
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
            | MessageType::StorageSearchTasks
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
            | MessageType::StorageExportNote
            | MessageType::FinanceListTransactions
            | MessageType::FinanceCreateTransaction
            | MessageType::FinanceDeleteTransaction
            | MessageType::FinanceMonthlySummary
            | MessageType::FinanceListRecurringRules
            | MessageType::FinanceCreateRecurringRule
            | MessageType::FinanceDeleteRecurringRule
            | MessageType::FinanceListBudgets
            | MessageType::FinanceSetBudget
            | MessageType::FinanceBudgetStatus
            | MessageType::FinanceListDebts
            | MessageType::FinanceCreateDebt
            | MessageType::FinanceUpdateDebt
            | MessageType::FinanceDeleteDebt
            | MessageType::FinanceListGoals
            | MessageType::FinanceCreateGoal
            | MessageType::FinanceUpdateGoal
            | MessageType::FinanceDeleteGoal
    )
}
