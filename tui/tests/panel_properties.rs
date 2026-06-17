// Feature: jinx
// Property tests P8, P9, P10 for Panel_Proximos and Panel_Calendario.

use proptest::prelude::*;
use storage::{Event, Priority, Task, TaskStatus};
use jinx::calendario::{CalendarView, TASK_PREFIX, EVENT_PREFIX, calendar_layout};
use jinx::proximos::{EntryKind, ProximosEntry, proximos};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn arb_priority() -> impl Strategy<Value = Priority> {
    prop_oneof![Just(Priority::Alta), Just(Priority::Media), Just(Priority::Baja)]
}

fn arb_date() -> impl Strategy<Value = String> {
    r"2025-[01][0-9]-[012][0-9]".prop_map(|s| s)
}

fn arb_time() -> impl Strategy<Value = String> {
    r"[01][0-9]:[0-5][0-9]".prop_map(|s| s)
}

fn arb_iso_ts() -> impl Strategy<Value = String> {
    r"2025-[01][0-9]-[012][0-9]T[01][0-9]:[0-5][0-9]:[0-5][0-9]\+00:00"
        .prop_map(|s| s)
}

fn arb_task(id: i64) -> impl Strategy<Value = Task> {
    (r"[\p{L} ]{1,20}", proptest::option::of(arb_iso_ts()), arb_priority())
        .prop_map(move |(title, deadline, priority)| Task {
            id,
            title,
            priority,
            status: TaskStatus::Pendiente,
            created_at: "2025-01-01T00:00:00+00:00".to_string(),
            deadline,
            group_id: None,
        })
}

fn arb_event(id: i64) -> impl Strategy<Value = Event> {
    (r"[\p{L} ]{1,20}", arb_date(), arb_time())
        .prop_map(move |(title, start_date, start_time)| Event {
            id,
            title,
            start_date,
            start_time,
            duration_minutes: None,
            group_id: None,
        })
}

// ---------------------------------------------------------------------------
// Feature: jinx, Property 8: Selección del Panel_Proximos
// ---------------------------------------------------------------------------
// Valida: Requisitos 6.1

proptest! {
    #[test]
    fn p8_proximos_selection(
        tasks in proptest::collection::vec(arb_task(1), 0..8),
        events in proptest::collection::vec(arb_event(1), 0..8),
    ) {
        let now_date = "2025-06-01";
        let now_time = "09:00";
        let end_date = "2025-06-02";
        let end_time = "09:00";
        let now_key = format!("{now_date}T{now_time}");
        let end_key = format!("{end_date}T{end_time}");

        let entries = proximos(&tasks, &events, now_date, now_time, end_date, end_time);

        // Every entry must be within [now, end]
        for e in &entries {
            let key = format!("{}T{}", e.date, e.time);
            prop_assert!(key >= now_key, "entry before window: {}", key);
            prop_assert!(key <= end_key, "entry after window: {}", key);
        }

        // Every task with deadline in [now, end] must appear
        for task in &tasks {
            if let Some(ref dl) = task.deadline {
                let date = if let Some(p) = dl.find('T') { &dl[..p] } else { dl.as_str() };
                let time = if let Some(p) = dl.find('T') { &dl[p+1..p+6] } else { "00:00" };
                let key = format!("{date}T{time}");
                if key >= now_key && key <= end_key {
                    let found = entries.iter().any(|e| e.title == task.title && matches!(e.kind, EntryKind::Task { .. }));
                    prop_assert!(found, "task '{}' should be in proximos", task.title);
                }
            }
        }

        // Every event in [now, end] must appear
        for event in &events {
            let key = format!("{}T{}", event.start_date, event.start_time);
            if key >= now_key && key <= end_key {
                let found = entries.iter().any(|e| e.title == event.title && e.kind == EntryKind::Event);
                prop_assert!(found, "event '{}' should be in proximos", event.title);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Feature: jinx, Property 9: Contenido del render del Panel_Proximos
// ---------------------------------------------------------------------------
// Valida: Requisitos 6.3, 6.4

proptest! {
    #[test]
    fn p9_proximos_entry_fields(
        tasks in proptest::collection::vec(arb_task(1), 0..5),
        events in proptest::collection::vec(arb_event(1), 0..5),
    ) {
        let entries = proximos(
            &tasks, &events,
            "2025-01-01", "00:00",
            "2026-12-31", "23:59",
        );

        for e in &entries {
            prop_assert!(!e.title.is_empty(), "title must be non-empty");
            prop_assert!(!e.date.is_empty(), "date must be non-empty");
            prop_assert!(!e.time.is_empty(), "time must be non-empty");
            if let EntryKind::Task { priority } = e.kind {
                // Priority must be a valid variant
                let _ = priority;
            }
        }

        // Sorted ascending
        for pair in entries.windows(2) {
            let key_a = format!("{}T{}", pair[0].date, pair[0].time);
            let key_b = format!("{}T{}", pair[1].date, pair[1].time);
            prop_assert!(key_a <= key_b, "proximos not sorted: {key_a} > {key_b}");
        }
    }
}

// ---------------------------------------------------------------------------
// Feature: jinx, Property 10: Render del Panel_Calendario
// ---------------------------------------------------------------------------
// Valida: Requisitos 17.1, 17.2, 17.3, 17.4, 17.5

fn arb_tasks() -> impl Strategy<Value = Vec<Task>> {
    proptest::collection::vec(arb_date(), 0..6).prop_flat_map(|dates| {
        let n = dates.len();
        proptest::collection::vec(
            (r"[\p{L} ]{1,20}", arb_priority(), prop::bool::ANY),
            n,
        )
        .prop_map(move |attrs| {
            attrs.into_iter().enumerate().map(|(i, (title, priority, has_deadline))| {
                Task {
                    id: (i + 1) as i64,
                    title,
                    priority,
                    status: TaskStatus::Pendiente,
                    created_at: "2025-01-01T00:00:00+00:00".to_string(),
                    deadline: if has_deadline {
                        Some(format!("{}T10:00:00+00:00", dates[i]))
                    } else {
                        None
                    },
                    group_id: None,
                }
            }).collect()
        })
    })
}

fn arb_events_unique() -> impl Strategy<Value = Vec<Event>> {
    proptest::collection::vec((r"[\p{L} ]{1,20}", arb_date(), arb_time()), 0..6)
        .prop_map(|attrs| {
            attrs.into_iter().enumerate().map(|(i, (title, start_date, start_time))| Event {
                id: (i + 1) as i64,
                title,
                start_date,
                start_time,
                duration_minutes: None,
                group_id: None,
            }).collect()
        })
}

// ---------------------------------------------------------------------------
// Feature: jinx, Property 10: Render del Panel_Calendario
// ---------------------------------------------------------------------------
// Valida: Requisitos 17.1, 17.2, 17.3, 17.4, 17.5

proptest! {
    #[test]
    fn p10_calendar_layout_correctness(
        tasks in arb_tasks(),
        events in arb_events_unique(),
    ) {
        let view = calendar_layout(&tasks, &events);

        // Every event appears on its start_date — matched by entity_id
        for event in &events {
            let cell = view.get(&event.start_date);
            let found = cell.map_or(false, |entries| {
                entries.iter().any(|e| !e.is_task && e.entity_id == event.id)
            });
            prop_assert!(found, "event id={} missing from cell {}", event.id, event.start_date);
        }

        // Every task with deadline appears on its deadline date — matched by entity_id
        for task in &tasks {
            if let Some(ref dl) = task.deadline {
                let date = if let Some(p) = dl.find('T') { dl[..p].to_string() } else { dl.clone() };
                let cell = view.get(&date);
                let found = cell.map_or(false, |entries| {
                    entries.iter().any(|e| e.is_task && e.entity_id == task.id)
                });
                prop_assert!(found, "task id={} missing from cell {}", task.id, date);
            }
        }

        // Tasks without deadline do not appear anywhere (by entity_id)
        for task in &tasks {
            if task.deadline.is_none() {
                for entries in view.values() {
                    let found = entries.iter().any(|e| e.is_task && e.entity_id == task.id);
                    prop_assert!(!found, "task id={} without deadline should not appear", task.id);
                }
            }
        }

        // Task entries use TASK_PREFIX, event entries use EVENT_PREFIX
        for entries in view.values() {
            for entry in entries {
                if entry.is_task {
                    prop_assert!(
                        entry.text.starts_with(TASK_PREFIX),
                        "task entry missing TASK_PREFIX: {}", entry.text
                    );
                } else {
                    prop_assert!(
                        entry.text.starts_with(EVENT_PREFIX),
                        "event entry missing EVENT_PREFIX: {}", entry.text
                    );
                }
                prop_assert_ne!(TASK_PREFIX, EVENT_PREFIX);
            }
        }
    }
}
