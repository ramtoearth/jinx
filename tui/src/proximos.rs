//! Panel_Proximos — entries due within the next 24 hours (Requisitos 6.1–6.4).
//!
//! All logic is implemented as a pure function over a storage snapshot so it
//! can be property-tested (Properties 8, 9) without a running terminal.

use storage::{Event, Task};

/// A single entry shown in Panel_Proximos.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProximosEntry {
    pub title: String,
    /// ISO 8601 date string (`YYYY-MM-DD`).
    pub date: String,
    /// `HH:MM` time string.
    pub time: String,
    pub kind: EntryKind,
    pub group_id: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryKind {
    Task { priority: storage::Priority },
    Event,
}

/// Parse an ISO 8601 timestamp (`YYYY-MM-DDTHH:MM:SS±HH:MM`) into
/// `(YYYY-MM-DD, HH:MM)` strings — returns `None` on parse failure.
fn parse_iso(ts: &str) -> Option<(String, String)> {
    let t_pos = ts.find('T')?;
    let date = &ts[..t_pos];
    let time_part = &ts[t_pos + 1..];
    let hhmm = &time_part[..time_part.find(['+', '-']).unwrap_or(5).min(5)];
    Some((date.to_string(), hhmm.to_string()))
}

/// Combine a `YYYY-MM-DD` date and `HH:MM` time into a sortable string.
fn datetime_key(date: &str, time: &str) -> String {
    format!("{date}T{time}")
}

/// Pure function: compute Panel_Proximos entries for the given snapshot and
/// current time.
///
/// Returns entries sorted ascending by `(date, time)`.
pub fn proximos(
    tasks: &[Task],
    events: &[Event],
    now_date: &str, // "YYYY-MM-DD"
    now_time: &str, // "HH:MM"
    end_date: &str, // "YYYY-MM-DD" (now + 24h)
    end_time: &str, // "HH:MM"
) -> Vec<ProximosEntry> {
    let now_key = datetime_key(now_date, now_time);
    let end_key = datetime_key(end_date, end_time);

    let mut entries: Vec<ProximosEntry> = vec![];

    // Tasks with deadline within [now, now+24h]
    for task in tasks {
        if let Some(ref deadline) = task.deadline {
            if let Some((date, time)) = parse_iso(deadline) {
                let key = datetime_key(&date, &time);
                if key >= now_key && key <= end_key {
                    entries.push(ProximosEntry {
                        title: task.title.clone(),
                        date,
                        time,
                        kind: EntryKind::Task { priority: task.priority },
                        group_id: task.group_id,
                    });
                }
            }
        }
    }

    // Events with start within [now, now+24h]
    for event in events {
        let key = datetime_key(&event.start_date, &event.start_time);
        if key >= now_key && key <= end_key {
            entries.push(ProximosEntry {
                title: event.title.clone(),
                date: event.start_date.clone(),
                time: event.start_time.clone(),
                kind: EntryKind::Event,
                group_id: event.group_id,
            });
        }
    }

    // Sort ascending by datetime key
    entries.sort_by_key(|e| datetime_key(&e.date, &e.time));
    entries
}

/// Compute `(end_date, end_time)` from a given `(now_date, now_time)` by
/// adding 24 hours. This is a pure helper used in tests and the render loop.
pub fn add_24h(date: &str, time: &str) -> (String, String) {
    // Simple date arithmetic: parse date parts, add 1 day
    let parts: Vec<&str> = date.split('-').collect();
    if parts.len() != 3 {
        return (date.to_string(), time.to_string());
    }
    let year: u32 = parts[0].parse().unwrap_or(2025);
    let month: u32 = parts[1].parse().unwrap_or(1);
    let day: u32 = parts[2].parse().unwrap_or(1);

    let days_in_month = days_in_month(year, month);
    let (ny, nmo, nd) = if day >= days_in_month {
        if month == 12 {
            (year + 1, 1, 1)
        } else {
            (year, month + 1, 1)
        }
    } else {
        (year, month, day + 1)
    };

    (format!("{ny:04}-{nmo:02}-{nd:02}"), time.to_string())
}

fn days_in_month(year: u32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if (year % 4 == 0 && year % 100 != 0) || year % 400 == 0 {
                29
            } else {
                28
            }
        }
        _ => 30,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use storage::{Priority, TaskStatus};

    fn make_task(deadline: Option<&str>, priority: Priority) -> Task {
        Task {
            id: 1,
            title: "task".to_string(),
            priority,
            status: TaskStatus::Pendiente,
            created_at: "2025-01-01T00:00:00+00:00".to_string(),
            deadline: deadline.map(|s| s.to_string()),
            group_id: None,
        }
    }

    fn make_event(date: &str, time: &str) -> Event {
        Event {
            id: 1,
            title: "event".to_string(),
            start_date: date.to_string(),
            start_time: time.to_string(),
            duration_minutes: None,
            group_id: None,
        }
    }

    #[test]
    fn includes_task_with_deadline_in_window() {
        let task = make_task(Some("2025-06-01T10:00:00+00:00"), Priority::Alta);
        let entries = proximos(
            &[task],
            &[],
            "2025-06-01", "09:00",
            "2025-06-02", "09:00",
        );
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].date, "2025-06-01");
    }

    #[test]
    fn excludes_task_without_deadline() {
        let task = make_task(None, Priority::Media);
        let entries = proximos(&[task], &[], "2025-06-01", "00:00", "2025-06-02", "00:00");
        assert!(entries.is_empty());
    }

    #[test]
    fn includes_event_in_window() {
        let event = make_event("2025-06-01", "14:00");
        let entries = proximos(
            &[],
            &[event],
            "2025-06-01", "09:00",
            "2025-06-02", "09:00",
        );
        assert_eq!(entries.len(), 1);
    }

    #[test]
    fn add_24h_increments_day() {
        assert_eq!(add_24h("2025-06-01", "14:30"), ("2025-06-02".to_string(), "14:30".to_string()));
    }

    #[test]
    fn add_24h_crosses_month() {
        assert_eq!(add_24h("2025-06-30", "00:00"), ("2025-07-01".to_string(), "00:00".to_string()));
    }
}
