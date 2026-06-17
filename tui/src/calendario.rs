//! Panel_Calendario — layout and rendering logic (Requisitos 17.1–17.6).
//!
//! The pure `calendar_layout` function is property-testable (Property 10).

use std::collections::HashMap;
use storage::{Event, Task};

/// Prefixes used to distinguish Tasks from Events in the Calendar view.
pub const TASK_PREFIX: &str = "▸";
pub const EVENT_PREFIX: &str = "●";

/// A single cell entry in the calendar grid.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CalendarEntry {
    /// The displayed text (prefix + title).
    pub text: String,
    /// Whether this is a Task (true) or Event (false).
    pub is_task: bool,
    pub group_id: Option<i64>,
    /// Id of the underlying entity.
    pub entity_id: i64,
}

/// Calendar layout: a map from `YYYY-MM-DD` → sorted list of entries.
pub type CalendarView = HashMap<String, Vec<CalendarEntry>>;

/// Build the calendar layout for the given tasks and events.
///
/// - Events appear on their `start_date`.
/// - Tasks with a `deadline` appear on the deadline's date.
/// - Tasks without a deadline are omitted.
/// - Within a date, Events are listed before Tasks; both are sorted by title.
pub fn calendar_layout(tasks: &[Task], events: &[Event]) -> CalendarView {
    let mut view: CalendarView = HashMap::new();

    // Place events first
    for event in events {
        let entry = CalendarEntry {
            text: format!("{EVENT_PREFIX} {}", event.title),
            is_task: false,
            group_id: event.group_id,
            entity_id: event.id,
        };
        view.entry(event.start_date.clone()).or_default().push(entry);
    }

    // Place tasks with deadlines
    for task in tasks {
        if let Some(ref deadline) = task.deadline {
            // Extract YYYY-MM-DD from ISO 8601 timestamp
            let date = if let Some(t_pos) = deadline.find('T') {
                deadline[..t_pos].to_string()
            } else {
                deadline.clone()
            };
            let entry = CalendarEntry {
                text: format!("{TASK_PREFIX} {}", task.title),
                is_task: true,
                group_id: task.group_id,
                entity_id: task.id,
            };
            view.entry(date).or_default().push(entry);
        }
    }

    // Sort entries within each date: events first, then tasks; alphabetically
    for entries in view.values_mut() {
        entries.sort_by(|a, b| {
            // Events before tasks
            match (a.is_task, b.is_task) {
                (false, true) => std::cmp::Ordering::Less,
                (true, false) => std::cmp::Ordering::Greater,
                _ => a.text.cmp(&b.text),
            }
        });
    }

    view
}

// ---------------------------------------------------------------------------
// Flat entry helpers for interactive calendar navigation
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FlatCalEntry {
    DateHeader(String),
    Entry(CalendarEntry),
}

pub fn flat_entries(view: &CalendarView) -> Vec<FlatCalEntry> {
    let mut dates: Vec<&String> = view.keys().collect();
    dates.sort();
    let mut flat = vec![];
    for date in dates {
        flat.push(FlatCalEntry::DateHeader(date.clone()));
        if let Some(entries) = view.get(date) {
            for e in entries {
                flat.push(FlatCalEntry::Entry(e.clone()));
            }
        }
    }
    flat
}

pub fn nth_entry(flat: &[FlatCalEntry], n: usize) -> Option<&CalendarEntry> {
    flat.iter()
        .filter_map(|f| match f {
            FlatCalEntry::Entry(e) => Some(e),
            _ => None,
        })
        .nth(n)
}

pub fn entry_count(flat: &[FlatCalEntry]) -> usize {
    flat.iter()
        .filter(|f| matches!(f, FlatCalEntry::Entry(_)))
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;
    use storage::{Priority, TaskStatus};

    fn make_task(id: i64, title: &str, deadline: Option<&str>) -> Task {
        Task {
            id,
            title: title.to_string(),
            priority: Priority::Media,
            status: TaskStatus::Pendiente,
            created_at: "2025-01-01T00:00:00+00:00".to_string(),
            deadline: deadline.map(|s| s.to_string()),
            group_id: None,
        }
    }

    fn make_event(id: i64, title: &str, date: &str) -> Event {
        Event {
            id,
            title: title.to_string(),
            start_date: date.to_string(),
            start_time: "10:00".to_string(),
            duration_minutes: None,
            group_id: None,
        }
    }

    #[test]
    fn event_appears_on_start_date() {
        let event = make_event(1, "Dentista", "2025-06-15");
        let view = calendar_layout(&[], &[event]);
        let entries = view.get("2025-06-15").unwrap();
        assert_eq!(entries.len(), 1);
        assert!(entries[0].text.starts_with(EVENT_PREFIX));
    }

    #[test]
    fn task_with_deadline_appears_on_date() {
        let task = make_task(1, "Informe", Some("2025-06-15T09:00:00+00:00"));
        let view = calendar_layout(&[task], &[]);
        let entries = view.get("2025-06-15").unwrap();
        assert_eq!(entries.len(), 1);
        assert!(entries[0].text.starts_with(TASK_PREFIX));
    }

    #[test]
    fn task_without_deadline_is_omitted() {
        let task = make_task(1, "Sin fecha", None);
        let view = calendar_layout(&[task], &[]);
        assert!(view.is_empty());
    }

    #[test]
    fn events_before_tasks_same_date() {
        let event = make_event(1, "Reunión", "2025-06-15");
        let task = make_task(2, "Tarea", Some("2025-06-15T00:00:00+00:00"));
        let view = calendar_layout(&[task], &[event]);
        let entries = view.get("2025-06-15").unwrap();
        assert_eq!(entries.len(), 2);
        assert!(!entries[0].is_task, "event should come first");
        assert!(entries[1].is_task, "task should come second");
    }
}
