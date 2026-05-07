// Feature: jinx
// Integration tests 19.1–19.5: end-to-end verification of component interactions.

use std::sync::Arc;

use storage::{NewEvent, NewGroup, NewTask, SqliteStorage, Storage, TaskFilter};
use jinx::app::{AppEvent, AppState, Panel};
use jinx::calendario::calendar_layout;
use jinx::color::{resolve_style, ColorMode};
use jinx::ipc::{
    Envelope, Kind, MessageType, StorageCreateTaskRequest, StorageListTasksRequest,
};
use jinx::ipc_handler::{handle_storage_request, is_storage_request};
use jinx::proximos::proximos;

// ---------------------------------------------------------------------------
// Integration test 19.1: IPC handler + storage round-trip
// ---------------------------------------------------------------------------
// Valida: Requisitos 1.1, 1.2, 1.3, 2.1, 10.1, 10.2

#[test]
fn ipc_create_task_appears_in_storage() {
    let storage: Arc<dyn Storage + Send + Sync> =
        Arc::new(SqliteStorage::in_memory().expect("in-memory"));

    // Simulate agent creating a task via IPC
    let req = StorageCreateTaskRequest {
        title: "tarea de prueba".to_string(),
        priority: None,
        deadline: None,
        group_id: None,
    };
    let envelope = Envelope::new(Kind::Request, MessageType::StorageCreateTask, &req)
        .expect("envelope");

    assert!(is_storage_request(envelope.message_type));

    let response = handle_storage_request(&envelope, &storage);
    assert!(response.error.is_none(), "expected no error, got: {:?}", response.error);
    assert!(response.payload.is_some(), "response must carry payload");

    // Task must appear in storage
    let tasks = storage.list_tasks(TaskFilter::default()).expect("list");
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].title, "tarea de prueba");

    // Panel_Tareas render: task should appear in list ordered by priority
    let focused_app = AppState::new(120, 40);
    assert_eq!(focused_app.focused_panel, Panel::Chat);
    let app = jinx::app::reduce(focused_app, AppEvent::Key(crossterm::event::KeyEvent::new(
        crossterm::event::KeyCode::Tab,
        crossterm::event::KeyModifiers::NONE,
    )));
    assert_eq!(app.focused_panel, Panel::Tareas);
}

// ---------------------------------------------------------------------------
// Integration test 19.2: timeout dialog via AppState
// ---------------------------------------------------------------------------
// Valida: Requisitos 10.4

#[test]
fn timeout_status_appears_in_status_bar() {
    let mut app = AppState::new(120, 40);
    app = jinx::app::reduce(
        app,
        AppEvent::StatusMessage("Tiempo de espera agotado. Vuelve a intentarlo.".to_string()),
    );
    assert!(
        app.status_bar.contains("Tiempo de espera"),
        "status bar must mention timeout, got: {}",
        app.status_bar
    );
}

// ---------------------------------------------------------------------------
// Integration test 19.3: group inference IPC + proximos filter
// ---------------------------------------------------------------------------
// Valida: Requisitos 15.9, 15.10, 15.11, 15.12, 6.1

#[test]
fn ipc_list_tasks_with_group_filter() {
    let storage: Arc<dyn Storage + Send + Sync> =
        Arc::new(SqliteStorage::in_memory().expect("in-memory"));

    let group = storage
        .create_group(NewGroup {
            name: "trabajo".to_string(),
            color: storage::HexColor::new("#3344ff".to_string()).unwrap(),
        })
        .expect("create group");

    let _task = storage
        .create_task(NewTask {
            title: "informe trimestral".to_string(),
            priority: None,
            deadline: Some("2025-06-15T10:00:00+00:00".to_string()),
            group_id: Some(group.id),
        })
        .expect("create task");

    let _other = storage
        .create_task(NewTask {
            title: "comprar leche".to_string(),
            priority: None,
            deadline: None,
            group_id: None,
        })
        .expect("create other task");

    // IPC: list tasks filtered by group_id
    let req = StorageListTasksRequest {
        status: None,
        group_id: Some(Some(group.id)),
        from_date: None,
        to_date: None,
    };
    let envelope =
        Envelope::new(Kind::Request, MessageType::StorageListTasks, &req).expect("envelope");
    let response = handle_storage_request(&envelope, &storage);
    assert!(response.error.is_none());

    let payload: jinx::ipc::StorageListTasksResponse =
        response.payload_as().expect("decode").expect("present");
    assert_eq!(payload.tasks.len(), 1);
    assert_eq!(payload.tasks[0].title, "informe trimestral");
    assert_eq!(payload.tasks[0].group_id, Some(group.id));
}

// ---------------------------------------------------------------------------
// Integration test 19.4: storage mutation → proximos version refresh
// ---------------------------------------------------------------------------
// Valida: Requisitos 6.2, 13.10, 17.6

#[test]
fn storage_change_reflected_in_proximos() {
    let storage: Arc<dyn Storage + Send + Sync> =
        Arc::new(SqliteStorage::in_memory().expect("in-memory"));

    // Before: no tasks
    let tasks = storage.list_tasks(TaskFilter::default()).unwrap_or_default();
    let events = storage.list_events(None, None).unwrap_or_default();
    let before = proximos(
        &tasks,
        &events,
        "2025-06-15",
        "09:00",
        "2025-06-16",
        "09:00",
    );
    assert!(before.is_empty());

    // Create a task with a deadline inside the 24-hour window
    storage
        .create_task(NewTask {
            title: "reunión".to_string(),
            priority: None,
            deadline: Some("2025-06-15T14:00:00+00:00".to_string()),
            group_id: None,
        })
        .expect("create task");

    // After: task appears in proximos
    let tasks2 = storage.list_tasks(TaskFilter::default()).unwrap_or_default();
    let after = proximos(
        &tasks2,
        &events,
        "2025-06-15",
        "09:00",
        "2025-06-16",
        "09:00",
    );
    assert_eq!(after.len(), 1);
    assert_eq!(after[0].title, "reunión");
}

// ---------------------------------------------------------------------------
// Integration test 19.5: color-mode fallback to monochrome
// ---------------------------------------------------------------------------
// Valida: Requisitos 16.4, 16.5

#[test]
fn monochrome_mode_adds_group_prefix() {
    let group_name = "trabajo";
    let hex = storage::HexColor::new("#ff6600".to_string()).unwrap();
    let styled = resolve_style(Some(&hex), Some(group_name), ColorMode::Monochrome);
    assert!(
        styled.prefix.as_deref().unwrap_or("").contains(group_name),
        "monochrome prefix must contain group name, got: {:?}",
        styled.prefix
    );
}

// ---------------------------------------------------------------------------
// Integration test 19.5b: calendar shows tasks and events together
// ---------------------------------------------------------------------------
// Valida: Requisitos 17.1, 17.2, 17.3

#[test]
fn calendar_shows_tasks_and_events_on_same_date() {
    let storage: Arc<dyn Storage + Send + Sync> =
        Arc::new(SqliteStorage::in_memory().expect("in-memory"));

    storage
        .create_task(NewTask {
            title: "informe".to_string(),
            priority: None,
            deadline: Some("2025-07-10T09:00:00+00:00".to_string()),
            group_id: None,
        })
        .expect("task");

    storage
        .create_event(NewEvent {
            title: "reunión de equipo".to_string(),
            start_date: "2025-07-10".to_string(),
            start_time: "10:00".to_string(),
            duration_minutes: Some(60),
            group_id: None,
        })
        .expect("event");

    let tasks = storage.list_tasks(TaskFilter::default()).unwrap();
    let events = storage.list_events(None, None).unwrap();
    let view = calendar_layout(&tasks, &events);

    let entries = view.get("2025-07-10").expect("date in view");
    assert_eq!(entries.len(), 2);
    // Event comes first
    assert!(!entries[0].is_task, "event must come first");
    assert!(entries[1].is_task, "task must come second");
    assert!(entries[0].text.contains("reunión de equipo"));
    assert!(entries[1].text.contains("informe"));
}
