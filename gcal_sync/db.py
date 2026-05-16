"""Direct SQLite access for the gcal_sync daemon.

Uses Python's built-in sqlite3 module. The jinx database uses WAL mode,
so concurrent reads/writes from the TUI (Rust) and this process are safe.
"""

from __future__ import annotations

import sqlite3
from dataclasses import dataclass
from pathlib import Path


@dataclass
class PendingEvent:
    id: int
    title: str
    start_date: str
    start_time: str
    duration_minutes: int | None
    google_event_id: str | None
    google_etag: str | None


@dataclass
class PendingTask:
    id: int
    title: str
    priority: str
    status: str
    created_at: str
    deadline: str | None
    google_event_id: str | None
    google_etag: str | None


def connect(db_path: str | Path) -> sqlite3.Connection:
    conn = sqlite3.connect(str(db_path), timeout=5)
    conn.execute("PRAGMA journal_mode = WAL")
    conn.execute("PRAGMA foreign_keys = ON")
    conn.row_factory = sqlite3.Row
    return conn


def get_pending_events(conn: sqlite3.Connection) -> list[PendingEvent]:
    rows = conn.execute(
        "SELECT id, title, start_date, start_time, duration_minutes, "
        "google_event_id, google_etag "
        "FROM events WHERE push_pending = 1"
    ).fetchall()
    return [
        PendingEvent(
            id=r["id"],
            title=r["title"],
            start_date=r["start_date"],
            start_time=r["start_time"],
            duration_minutes=r["duration_minutes"],
            google_event_id=r["google_event_id"],
            google_etag=r["google_etag"],
        )
        for r in rows
    ]


def get_pending_tasks(conn: sqlite3.Connection) -> list[PendingTask]:
    rows = conn.execute(
        "SELECT id, title, priority, status, created_at, deadline, "
        "google_event_id, google_etag "
        "FROM tasks WHERE push_pending = 1"
    ).fetchall()
    return [
        PendingTask(
            id=r["id"],
            title=r["title"],
            priority=r["priority"],
            status=r["status"],
            created_at=r["created_at"],
            deadline=r["deadline"],
            google_event_id=r["google_event_id"],
            google_etag=r["google_etag"],
        )
        for r in rows
    ]


def mark_event_synced(
    conn: sqlite3.Connection,
    event_id: int,
    google_event_id: str,
    google_etag: str,
) -> None:
    conn.execute(
        "UPDATE events SET google_event_id = ?, google_etag = ?, push_pending = 0 "
        "WHERE id = ?",
        (google_event_id, google_etag, event_id),
    )
    conn.commit()


def mark_task_synced(
    conn: sqlite3.Connection,
    task_id: int,
    google_event_id: str,
    google_etag: str,
) -> None:
    conn.execute(
        "UPDATE tasks SET google_event_id = ?, google_etag = ?, push_pending = 0 "
        "WHERE id = ?",
        (google_event_id, google_etag, task_id),
    )
    conn.commit()
