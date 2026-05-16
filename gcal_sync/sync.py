"""Core push logic: jinx events and tasks → Google Calendar."""

from __future__ import annotations

import datetime
from typing import Any

from gcal_sync.db import (
    PendingEvent,
    PendingTask,
    get_pending_events,
    get_pending_tasks,
    mark_event_synced,
    mark_task_synced,
)

PRIORITY_EMOJI = {"alta": "🔴", "media": "🟡", "baja": "🟢"}
STATUS_LABEL = {"pendiente": "pending", "completada": "done", "cancelada": "cancelled"}


def to_google_event(event: PendingEvent, timezone: str) -> dict[str, Any]:
    """Convert a jinx event to a Google Calendar event body."""
    time_str = event.start_time
    if time_str.count(":") == 2:
        fmt = "%Y-%m-%dT%H:%M:%S"
    else:
        fmt = "%Y-%m-%dT%H:%M"
    start_dt = datetime.datetime.strptime(
        f"{event.start_date}T{time_str}", fmt
    )

    duration = event.duration_minutes or 60
    end_dt = start_dt + datetime.timedelta(minutes=duration)

    return {
        "summary": event.title,
        "start": {"dateTime": start_dt.isoformat(), "timeZone": timezone},
        "end": {"dateTime": end_dt.isoformat(), "timeZone": timezone},
    }


def task_to_google_event(task: PendingTask, timezone: str) -> dict[str, Any]:
    """Convert a jinx task to a Google Calendar event body."""
    priority = PRIORITY_EMOJI.get(task.priority, "")
    status = STATUS_LABEL.get(task.status, task.status)
    summary = f"[Task{' ' + priority if priority else ''}] {task.title}"

    if task.deadline:
        # Parse deadline — ISO 8601 with timezone offset
        dl = task.deadline
        try:
            if "T" in dl:
                # Has time component: "2026-05-16T14:00:00+00:00"
                dt = datetime.datetime.fromisoformat(dl)
                end_dt = dt + datetime.timedelta(hours=1)
                return {
                    "summary": summary,
                    "description": f"Status: {status}",
                    "start": {"dateTime": dt.isoformat(), "timeZone": timezone},
                    "end": {"dateTime": end_dt.isoformat(), "timeZone": timezone},
                }
            else:
                # Date only: "2026-05-16"
                return {
                    "summary": summary,
                    "description": f"Status: {status}",
                    "start": {"date": dl},
                    "end": {"date": dl},
                }
        except (ValueError, TypeError):
            pass

    # No deadline — use created_at date as all-day event
    created_date = task.created_at[:10] if len(task.created_at) >= 10 else None
    if created_date:
        return {
            "summary": summary,
            "description": f"Status: {status}",
            "start": {"date": created_date},
            "end": {"date": created_date},
        }

    # Fallback: today
    today = datetime.date.today().isoformat()
    return {
        "summary": summary,
        "description": f"Status: {status}",
        "start": {"date": today},
        "end": {"date": today},
    }


def _push_item(
    service: Any,
    calendar_id: str,
    body: dict[str, Any],
    google_event_id: str | None,
) -> dict[str, Any] | None:
    """Insert or update a single item in Google Calendar. Returns the result."""
    try:
        if google_event_id is None:
            return (
                service.events()
                .insert(calendarId=calendar_id, body=body)
                .execute()
            )
        else:
            return (
                service.events()
                .update(
                    calendarId=calendar_id,
                    eventId=google_event_id,
                    body=body,
                )
                .execute()
            )
    except Exception:
        return None


def push_pending(
    service: Any,
    conn: Any,
    calendar_id: str,
    timezone: str,
) -> int:
    """Push all pending events and tasks to Google Calendar. Returns count pushed."""
    pushed = 0

    for event in get_pending_events(conn):
        body = to_google_event(event, timezone)
        result = _push_item(service, calendar_id, body, event.google_event_id)
        if result:
            mark_event_synced(conn, event.id, result["id"], result.get("etag", ""))
            pushed += 1

    for task in get_pending_tasks(conn):
        body = task_to_google_event(task, timezone)
        result = _push_item(service, calendar_id, body, task.google_event_id)
        if result:
            mark_task_synced(conn, task.id, result["id"], result.get("etag", ""))
            pushed += 1

    return pushed


def delete_from_google(
    service: Any,
    calendar_id: str,
    google_event_id: str,
) -> bool:
    """Delete an event from Google Calendar. Returns True on success."""
    try:
        service.events().delete(
            calendarId=calendar_id, eventId=google_event_id
        ).execute()
        return True
    except Exception:
        return False
