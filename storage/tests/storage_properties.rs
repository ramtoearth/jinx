// Feature: terminal-day-organizer
// Property tests for the SqliteStorage implementation.
// Each test carries the annotation required by design.md § Testing Strategy.

use proptest::prelude::*;
use storage::{
    Event, EventPatch, Group, HexColor, NewEvent, NewGroup, NewTask, Priority, SqliteStorage,
    Storage, StorageError, Task, TaskFilter, TaskPatch, TaskStatus,
};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn db() -> SqliteStorage {
    SqliteStorage::in_memory().expect("in-memory db")
}

fn arb_title() -> impl Strategy<Value = String> {
    r"[\p{L}\p{N} ]{1,40}".prop_map(|s| s)
}

fn arb_priority() -> impl Strategy<Value = Priority> {
    prop_oneof![Just(Priority::Alta), Just(Priority::Media), Just(Priority::Baja)]
}

fn arb_task_status() -> impl Strategy<Value = TaskStatus> {
    prop_oneof![
        Just(TaskStatus::Pendiente),
        Just(TaskStatus::Completada),
        Just(TaskStatus::Cancelada),
    ]
}

fn arb_date() -> impl Strategy<Value = String> {
    r"20[2-3][0-9]-[01][0-9]-[0-3][0-9]".prop_map(|s| s)
}

fn arb_time() -> impl Strategy<Value = String> {
    r"[01][0-9]:[0-5][0-9]".prop_map(|s| s)
}

fn arb_hex() -> impl Strategy<Value = HexColor> {
    r"#[0-9a-fA-F]{6}".prop_map(|s| HexColor::new(s).unwrap())
}

fn arb_iso_ts() -> impl Strategy<Value = String> {
    r"202[0-9]-[01][0-9]-[0-3][0-9]T[0-2][0-9]:[0-5][0-9]:[0-5][0-9]\+00:00"
        .prop_map(|s| s)
}

fn arb_new_task() -> impl Strategy<Value = NewTask> {
    (
        arb_title(),
        proptest::option::of(arb_priority()),
        proptest::option::of(arb_iso_ts()),
    )
        .prop_map(|(title, priority, deadline)| NewTask {
            title,
            priority,
            deadline,
            group_id: None,
        })
}

fn arb_new_event() -> impl Strategy<Value = NewEvent> {
    (arb_title(), arb_date(), arb_time(), proptest::option::of(0u32..=480u32))
        .prop_map(|(title, start_date, start_time, duration_minutes)| NewEvent {
            title,
            start_date,
            start_time,
            duration_minutes,
            group_id: None,
        })
}

fn arb_new_group() -> impl Strategy<Value = NewGroup> {
    (arb_title(), arb_hex()).prop_map(|(name, color)| NewGroup { name, color })
}

// ---------------------------------------------------------------------------
// Feature: terminal-day-organizer, Property 11: Creación con valores por defecto
// ---------------------------------------------------------------------------
// Valida: Requisitos 2.1, 2.7

proptest! {
    #[test]
    fn p11_new_task_defaults(input in arb_new_task()) {
        let s = db();
        let task = s.create_task(input.clone()).unwrap();
        // Tasks without explicit priority must default to Media
        let expected_priority = input.priority.unwrap_or(Priority::Media);
        prop_assert_eq!(task.priority, expected_priority, "priority default");
        // Every new task must be Pendiente
        prop_assert_eq!(task.status, TaskStatus::Pendiente, "status default");
    }
}

// ---------------------------------------------------------------------------
// Feature: terminal-day-organizer, Property 12: Actualización de un único campo
// ---------------------------------------------------------------------------
// Valida: Requisitos 2.5, 3.5, 15.3, 15.4

proptest! {
    #[test]
    fn p12_single_field_update_task(
        original in arb_new_task(),
        new_title in arb_title(),
        new_priority in arb_priority(),
    ) {
        let s = db();
        let created = s.create_task(original.clone()).unwrap();

        // Update only title
        let updated_title = s.update_task(
            created.id,
            TaskPatch { title: Some(new_title.clone()), ..Default::default() },
        ).unwrap();
        prop_assert_eq!(&updated_title.title, &new_title);
        prop_assert_eq!(updated_title.priority, created.priority);
        prop_assert_eq!(updated_title.status, created.status);
        prop_assert_eq!(updated_title.deadline, created.deadline);

        // Update only priority
        let updated_priority = s.update_task(
            created.id,
            TaskPatch { priority: Some(new_priority), ..Default::default() },
        ).unwrap();
        prop_assert_eq!(updated_priority.priority, new_priority);
        prop_assert_eq!(&updated_priority.title, &new_title); // title from prev update
        prop_assert_eq!(updated_priority.status, created.status);
    }
}

proptest! {
    #[test]
    fn p12_single_field_update_group(
        original in arb_new_group(),
        new_name in arb_title(),
        new_color in arb_hex(),
    ) {
        let s = db();
        let created = s.create_group(original.clone()).unwrap();

        // Rename
        let renamed = s.rename_group(created.id, new_name.clone()).unwrap();
        prop_assert_eq!(&renamed.name, &new_name);
        prop_assert_eq!(&renamed.color, &created.color);

        // Recolor
        let recolored = s.recolor_group(created.id, new_color.clone()).unwrap();
        prop_assert_eq!(&recolored.color, &new_color);
        prop_assert_eq!(&recolored.name, &new_name); // name from rename
    }
}

// ---------------------------------------------------------------------------
// Feature: terminal-day-organizer, Property 13: Eliminación y cascada a NULL
// ---------------------------------------------------------------------------
// Valida: Requisitos 2.4, 3.4, 15.5

proptest! {
    #[test]
    fn p13_delete_task_removes_from_list(input in arb_new_task()) {
        let s = db();
        let task = s.create_task(input).unwrap();
        s.delete_task(task.id).unwrap();
        let tasks = s.list_tasks(TaskFilter::default()).unwrap();
        prop_assert!(!tasks.iter().any(|t| t.id == task.id));
    }
}

proptest! {
    #[test]
    fn p13_delete_group_nullifies_fks(
        group_input in arb_new_group(),
        task_title in arb_title(),
        event_title in arb_title(),
        event_date in arb_date(),
        event_time in arb_time(),
    ) {
        let s = db();
        let group = s.create_group(group_input).unwrap();

        let task = s.create_task(NewTask {
            title: task_title,
            priority: None,
            deadline: None,
            group_id: Some(group.id),
        }).unwrap();

        let event = s.create_event(NewEvent {
            title: event_title,
            start_date: event_date,
            start_time: event_time,
            duration_minutes: None,
            group_id: Some(group.id),
        }).unwrap();

        // Delete group — ON DELETE SET NULL must fire
        s.delete_group(group.id).unwrap();

        let tasks = s.list_tasks(TaskFilter::default()).unwrap();
        let found_task = tasks.iter().find(|t| t.id == task.id).unwrap();
        prop_assert_eq!(found_task.group_id, None, "task group_id should be NULL");

        let events = s.list_events(None, None).unwrap();
        let found_event = events.iter().find(|e| e.id == event.id).unwrap();
        prop_assert_eq!(found_event.group_id, None, "event group_id should be NULL");
    }
}

// ---------------------------------------------------------------------------
// Feature: terminal-day-organizer, Property 14: Operación sobre entidad inexistente
// ---------------------------------------------------------------------------
// Valida: Requisitos 2.6

proptest! {
    #[test]
    fn p14_not_found_on_missing_task(id in 900_000_i64..=999_999_i64) {
        let s = db();
        let result = s.update_task(id, TaskPatch::default());
        match result {
            Err(StorageError::NotFound(_)) => {}
            other => prop_assert!(false, "expected NotFound, got {other:?}"),
        }
        // Almacén unchanged
        let tasks = s.list_tasks(TaskFilter::default()).unwrap();
        prop_assert!(tasks.is_empty());
    }
}

proptest! {
    #[test]
    fn p14_not_found_on_missing_event(id in 900_000_i64..=999_999_i64) {
        let s = db();
        let result = s.delete_event(id);
        match result {
            Err(StorageError::NotFound(_)) => {}
            other => prop_assert!(false, "expected NotFound, got {other:?}"),
        }
    }
}

proptest! {
    #[test]
    fn p14_not_found_on_missing_group(id in 900_000_i64..=999_999_i64, new_name in arb_title()) {
        let s = db();
        let result = s.rename_group(id, new_name);
        match result {
            Err(StorageError::NotFound(_)) => {}
            other => prop_assert!(false, "expected NotFound, got {other:?}"),
        }
        let groups = s.list_groups().unwrap();
        prop_assert!(groups.is_empty());
    }
}

// ---------------------------------------------------------------------------
// Feature: terminal-day-organizer, Property 15: Durabilidad transaccional al reabrir
// ---------------------------------------------------------------------------
// Valida: Requisitos 7.2, 7.3

#[test]
fn p15_durability_on_reopen() {
    let dir = tempdir();
    let path = dir.path().join("test.sqlite3");

    {
        let s = SqliteStorage::open(&path).expect("open");
        s.create_task(NewTask {
            title: "tarea persistente".to_string(),
            priority: Some(Priority::Alta),
            deadline: None,
            group_id: None,
        })
        .unwrap();
    } // drop closes connection

    let s2 = SqliteStorage::open(&path).expect("reopen");
    let tasks = s2.list_tasks(TaskFilter::default()).unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].title, "tarea persistente");
}

fn tempdir() -> TempDir {
    TempDir::new()
}

struct TempDir(std::path::PathBuf);
impl TempDir {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "storage_test_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .subsec_nanos()
        ));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
    fn path(&self) -> &std::path::Path {
        &self.0
    }
}
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

// ---------------------------------------------------------------------------
// Feature: terminal-day-organizer, Property 16: Unicidad de nombre de Grupo
// ---------------------------------------------------------------------------
// Valida: Requisitos 15.1, 15.2

proptest! {
    #[test]
    fn p16_group_name_unique(name in arb_title(), color1 in arb_hex(), color2 in arb_hex()) {
        let s = db();
        s.create_group(NewGroup { name: name.clone(), color: color1 }).unwrap();
        let result = s.create_group(NewGroup { name: name.clone(), color: color2 });
        match result {
            Err(StorageError::GroupNameNotUnique(_)) => {}
            other => prop_assert!(false, "expected GroupNameNotUnique, got {other:?}"),
        }
        // Only one group in the store
        let groups = s.list_groups().unwrap();
        prop_assert_eq!(groups.len(), 1);
    }
}

proptest! {
    #[test]
    fn p16_group_with_new_name_persists(name in arb_title(), color in arb_hex()) {
        let s = db();
        let g = s.create_group(NewGroup { name: name.clone(), color }).unwrap();
        prop_assert_eq!(&g.name, &name);
    }
}

// ---------------------------------------------------------------------------
// Feature: terminal-day-organizer, Property 17: Invariante de Grupo único por entidad
// ---------------------------------------------------------------------------
// Valida: Requisitos 15.6

proptest! {
    #[test]
    fn p17_group_id_references_existing_group(task_input in arb_new_task()) {
        let s = db();
        // Attempt to create a task with a nonexistent group_id
        let result = s.create_task(NewTask {
            group_id: Some(9_999_999),
            ..task_input
        });
        match result {
            Err(StorageError::ForeignKeyViolation(_)) | Err(StorageError::NotFound(_)) => {}
            other => prop_assert!(false, "expected FK error, got {other:?}"),
        }
    }
}

// ---------------------------------------------------------------------------
// Feature: terminal-day-organizer, Property 6: Orden total de listado y priorización de Tareas
// ---------------------------------------------------------------------------
// Valida: Requisitos 2.2, 4.1

proptest! {
    #[test]
    fn p6_task_list_ordering(tasks in proptest::collection::vec(arb_new_task(), 1..10)) {
        let s = db();
        for t in &tasks {
            s.create_task(t.clone()).unwrap();
        }
        let listed = s
            .list_tasks(TaskFilter { status: Some(TaskStatus::Pendiente), ..Default::default() })
            .unwrap();

        // Verify total order: alta < media < baja
        for pair in listed.windows(2) {
            let a = &pair[0];
            let b = &pair[1];
            let pa = priority_rank(a.priority);
            let pb = priority_rank(b.priority);
            prop_assert!(pa <= pb, "priority order violated: {:?} before {:?}", a.priority, b.priority);
            if pa == pb {
                // Within same priority: tasks with deadline precede those without
                match (&a.deadline, &b.deadline) {
                    (None, Some(_)) => prop_assert!(false, "deadline order violated"),
                    (Some(da), Some(db)) => prop_assert!(da <= db, "deadline asc violated"),
                    _ => {}
                }
            }
        }
    }
}

fn priority_rank(p: Priority) -> u8 {
    match p {
        Priority::Alta => 1,
        Priority::Media => 2,
        Priority::Baja => 3,
    }
}

// ---------------------------------------------------------------------------
// Feature: terminal-day-organizer, Property 7: Listado de Eventos por rango
// ---------------------------------------------------------------------------
// Valida: Requisitos 3.3

proptest! {
    #[test]
    fn p7_event_list_range_filter(
        events in proptest::collection::vec(arb_new_event(), 1..8),
    ) {
        let s = db();
        for e in &events {
            s.create_event(e.clone()).unwrap();
        }

        let from = "2025-01-01";
        let to = "2025-12-31";
        let listed = s.list_events(Some(from), Some(to)).unwrap();

        // All listed events must be within [from, to]
        for e in &listed {
            prop_assert!(
                e.start_date.as_str() >= from && e.start_date.as_str() <= to,
                "event date {} outside range",
                e.start_date
            );
        }

        // Verify ascending order by (start_date, start_time)
        for pair in listed.windows(2) {
            let a = &pair[0];
            let b = &pair[1];
            let key_a = (&a.start_date, &a.start_time);
            let key_b = (&b.start_date, &b.start_time);
            prop_assert!(key_a <= key_b, "event order violated");
        }
    }
}

// ---------------------------------------------------------------------------
// Property 2 & 3 (Export round-trip and Markdown coverage) live in
// export_properties.rs to keep this file focused on storage CRUD.
// ---------------------------------------------------------------------------
