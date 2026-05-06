// Feature: terminal-day-organizer

use proptest::prelude::*;
use storage::{
    HexColor, NewEvent, NewGroup, NewTask, Priority, SqliteStorage, Storage, StorageError,
    TaskFilter,
};

fn db() -> SqliteStorage {
    SqliteStorage::in_memory().expect("in-memory db")
}

fn arb_title() -> impl Strategy<Value = String> {
    r"[\p{L}\p{N} ]{1,30}".prop_map(|s| s)
}

fn arb_hex() -> impl Strategy<Value = HexColor> {
    r"#[0-9a-fA-F]{6}".prop_map(|s| HexColor::new(s).unwrap())
}

fn arb_date() -> impl Strategy<Value = String> {
    r"20[2-3][0-9]-[01][0-9]-[0-3][0-9]".prop_map(|s| s)
}

fn arb_time() -> impl Strategy<Value = String> {
    r"[01][0-9]:[0-5][0-9]".prop_map(|s| s)
}

struct TempDir(std::path::PathBuf);
impl TempDir {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "export_test_{}",
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
// Feature: terminal-day-organizer, Property 2: Round-trip de exportación SQLite
// ---------------------------------------------------------------------------
// Valida: Requisitos 8.2, 8.5

proptest! {
    #![proptest_config(proptest::test_runner::Config::with_cases(50))]

    #[test]
    fn p2_sqlite_export_round_trip(
        groups in proptest::collection::vec(
            (arb_title(), arb_hex()),
            0..3,
        ),
        tasks in proptest::collection::vec(arb_title(), 0..5),
        events in proptest::collection::vec(
            (arb_title(), arb_date(), arb_time()),
            0..4,
        ),
    ) {
        let src = db();

        // Ensure group names are unique (dedup by name)
        let mut seen_names = std::collections::HashSet::new();
        let mut group_ids = vec![];
        for (name, color) in &groups {
            if seen_names.insert(name.clone()) {
                let g = src.create_group(storage::NewGroup {
                    name: name.clone(),
                    color: color.clone(),
                }).unwrap();
                group_ids.push(g.id);
            }
        }

        for title in &tasks {
            src.create_task(NewTask {
                title: title.clone(),
                priority: None,
                deadline: None,
                group_id: None,
            }).unwrap();
        }

        for (title, date, time) in &events {
            src.create_event(NewEvent {
                title: title.clone(),
                start_date: date.clone(),
                start_time: time.clone(),
                duration_minutes: None,
                group_id: None,
            }).unwrap();
        }

        let td = TempDir::new();
        let export_path = td.path().join("export.sqlite3");

        src.export_sqlite(&export_path).unwrap();

        // Import into a fresh DB
        let dst = db();
        dst.import_sqlite(&export_path).unwrap();

        // Compare group names (ids may differ)
        let src_groups = src.list_groups().unwrap();
        let dst_groups = dst.list_groups().unwrap();
        let mut src_names: Vec<_> = src_groups.iter().map(|g| g.name.clone()).collect();
        let mut dst_names: Vec<_> = dst_groups.iter().map(|g| g.name.clone()).collect();
        src_names.sort();
        dst_names.sort();
        prop_assert_eq!(src_names, dst_names, "group names differ after round-trip");

        // Compare task titles
        let src_tasks = src.list_tasks(TaskFilter::default()).unwrap();
        let dst_tasks = dst.list_tasks(TaskFilter::default()).unwrap();
        let mut src_titles: Vec<_> = src_tasks.iter().map(|t| t.title.clone()).collect();
        let mut dst_titles: Vec<_> = dst_tasks.iter().map(|t| t.title.clone()).collect();
        src_titles.sort();
        dst_titles.sort();
        prop_assert_eq!(src_titles, dst_titles, "task titles differ after round-trip");

        // Compare event titles
        let src_events = src.list_events(None, None).unwrap();
        let dst_events = dst.list_events(None, None).unwrap();
        let mut src_etitles: Vec<_> = src_events.iter().map(|e| e.title.clone()).collect();
        let mut dst_etitles: Vec<_> = dst_events.iter().map(|e| e.title.clone()).collect();
        src_etitles.sort();
        dst_etitles.sort();
        prop_assert_eq!(src_etitles, dst_etitles, "event titles differ after round-trip");
    }
}

// ---------------------------------------------------------------------------
// Feature: terminal-day-organizer, Property 3: Cobertura de la exportación a Markdown
// ---------------------------------------------------------------------------
// Valida: Requisitos 8.1

proptest! {
    #![proptest_config(proptest::test_runner::Config::with_cases(50))]

    #[test]
    fn p3_markdown_export_coverage(
        groups in proptest::collection::vec((arb_title(), arb_hex()), 0..3),
        tasks in proptest::collection::vec(arb_title(), 0..5),
        events in proptest::collection::vec(
            (arb_title(), arb_date(), arb_time()),
            0..4,
        ),
    ) {
        let s = db();

        let mut seen_names = std::collections::HashSet::new();
        for (name, color) in &groups {
            if seen_names.insert(name.clone()) {
                s.create_group(storage::NewGroup { name: name.clone(), color: color.clone() }).unwrap();
            }
        }
        for title in &tasks {
            s.create_task(NewTask {
                title: title.clone(),
                priority: None,
                deadline: None,
                group_id: None,
            }).unwrap();
        }
        for (title, date, time) in &events {
            s.create_event(NewEvent {
                title: title.clone(),
                start_date: date.clone(),
                start_time: time.clone(),
                duration_minutes: None,
                group_id: None,
            }).unwrap();
        }

        let td = TempDir::new();
        let md_path = td.path().join("export.md");
        s.export_markdown(&md_path).unwrap();

        let content = std::fs::read_to_string(&md_path).unwrap();

        // Must contain the three required sections
        prop_assert!(content.contains("# Tareas"), "missing # Tareas section");
        prop_assert!(content.contains("# Eventos"), "missing # Eventos section");
        prop_assert!(content.contains("# Grupos"), "missing # Grupos section");

        // Every stored task title must appear in the Markdown
        let stored_tasks = s.list_tasks(TaskFilter::default()).unwrap();
        for t in &stored_tasks {
            prop_assert!(
                content.contains(&t.title),
                "task title '{}' missing from Markdown",
                t.title
            );
        }

        // Every stored event title must appear
        let stored_events = s.list_events(None, None).unwrap();
        for e in &stored_events {
            prop_assert!(
                content.contains(&e.title),
                "event title '{}' missing from Markdown",
                e.title
            );
        }

        // Every stored group name must appear
        let stored_groups = s.list_groups().unwrap();
        for g in &stored_groups {
            prop_assert!(
                content.contains(&g.name),
                "group name '{}' missing from Markdown",
                g.name
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Export error: non-writable path returns IO_NOT_WRITABLE (task 16.8)
// ---------------------------------------------------------------------------

#[test]
fn export_to_nonexistent_parent_returns_error() {
    let s = db();
    let bad_path = std::path::Path::new("/nonexistent_dir/definitely_missing/out.md");
    let result = s.export_markdown(bad_path);
    assert!(
        matches!(result, Err(StorageError::IoNotWritable(_))),
        "expected IoNotWritable, got {result:?}"
    );
}
