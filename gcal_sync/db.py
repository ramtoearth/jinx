"""Direct SQLite access for the gcal_sync daemon.

Uses Python's built-in sqlite3 module. The jinx database uses WAL mode,
so concurrent reads/writes from the TUI (Rust) and this process are safe.
"""

from __future__ import annotations

import datetime
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


# ---------------------------------------------------------------------------
# Push helpers
# ---------------------------------------------------------------------------


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


# ---------------------------------------------------------------------------
# Sync state
# ---------------------------------------------------------------------------


def get_sync_state(conn: sqlite3.Connection) -> tuple[str | None, str | None]:
    row = conn.execute(
        "SELECT calendar_sync_token, tasks_last_sync FROM sync_state WHERE id = 1"
    ).fetchone()
    if row is None:
        return (None, None)
    return (row["calendar_sync_token"], row["tasks_last_sync"])


def set_calendar_sync_token(conn: sqlite3.Connection, token: str | None) -> None:
    conn.execute(
        "UPDATE sync_state SET calendar_sync_token = ? WHERE id = 1",
        (token,),
    )
    conn.commit()


def set_tasks_last_sync(conn: sqlite3.Connection, timestamp: str) -> None:
    conn.execute(
        "UPDATE sync_state SET tasks_last_sync = ? WHERE id = 1",
        (timestamp,),
    )
    conn.commit()


# ---------------------------------------------------------------------------
# Pull helpers — find/upsert/delete by google ID
# ---------------------------------------------------------------------------


def find_event_by_google_id(conn: sqlite3.Connection, google_event_id: str) -> dict | None:
    row = conn.execute(
        "SELECT id, title, start_date, start_time, duration_minutes, "
        "group_id, google_event_id, google_etag, push_pending "
        "FROM events WHERE google_event_id = ?",
        (google_event_id,),
    ).fetchone()
    return dict(row) if row else None


def find_task_by_google_id(conn: sqlite3.Connection, google_task_id: str) -> dict | None:
    row = conn.execute(
        "SELECT id, title, priority, status, created_at, deadline, "
        "group_id, google_event_id, google_etag, push_pending "
        "FROM tasks WHERE google_event_id = ?",
        (google_task_id,),
    ).fetchone()
    return dict(row) if row else None


def upsert_event_from_google(
    conn: sqlite3.Connection,
    google_event_id: str,
    title: str,
    start_date: str,
    start_time: str,
    duration_minutes: int | None,
    google_etag: str,
) -> None:
    existing = find_event_by_google_id(conn, google_event_id)
    if existing:
        conn.execute(
            "UPDATE events SET title = ?, start_date = ?, start_time = ?, "
            "duration_minutes = ?, google_etag = ?, push_pending = 0 WHERE id = ?",
            (title, start_date, start_time, duration_minutes, google_etag, existing["id"]),
        )
    else:
        conn.execute(
            "INSERT INTO events (title, start_date, start_time, duration_minutes, "
            "group_id, google_event_id, google_etag, push_pending) "
            "VALUES (?, ?, ?, ?, NULL, ?, ?, 0)",
            (title, start_date, start_time, duration_minutes, google_event_id, google_etag),
        )
    conn.commit()


def upsert_task_from_google(
    conn: sqlite3.Connection,
    google_task_id: str,
    title: str,
    priority: str,
    status: str,
    deadline: str | None,
    google_etag: str,
) -> None:
    existing = find_task_by_google_id(conn, google_task_id)
    if existing:
        conn.execute(
            "UPDATE tasks SET title = ?, priority = ?, status = ?, deadline = ?, "
            "google_etag = ?, push_pending = 0 WHERE id = ?",
            (title, priority, status, deadline, google_etag, existing["id"]),
        )
    else:
        now = datetime.datetime.now(datetime.timezone.utc).strftime(
            "%Y-%m-%dT%H:%M:%S+00:00"
        )
        conn.execute(
            "INSERT INTO tasks (title, priority, status, created_at, deadline, "
            "group_id, google_event_id, google_etag, push_pending) "
            "VALUES (?, ?, ?, ?, ?, NULL, ?, ?, 0)",
            (title, priority, status, now, deadline, google_task_id, google_etag),
        )
    conn.commit()


def delete_event_by_google_id(conn: sqlite3.Connection, google_event_id: str) -> None:
    conn.execute("DELETE FROM events WHERE google_event_id = ?", (google_event_id,))
    conn.commit()


def delete_task_by_google_id(conn: sqlite3.Connection, google_task_id: str) -> None:
    conn.execute("DELETE FROM tasks WHERE google_event_id = ?", (google_task_id,))
    conn.commit()
