"""Core push logic: jinx events → Google Calendar, jinx tasks → Google Tasks."""

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

PRIORITY_LABEL = {"alta": "High", "media": "Medium", "baja": "Low"}


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


def task_to_google_task(task: PendingTask) -> dict[str, Any]:
    """Convert a jinx task to a Google Tasks body."""
    body: dict[str, Any] = {"title": task.title}

    if task.deadline:
        dl = task.deadline
        if "T" in dl:
            dt = datetime.datetime.fromisoformat(dl)
            body["due"] = dt.strftime("%Y-%m-%dT%H:%M:%S.000Z")
        else:
            body["due"] = f"{dl}T00:00:00.000Z"

    if task.status == "completada":
        body["status"] = "completed"
    else:
        body["status"] = "needsAction"

    priority = PRIORITY_LABEL.get(task.priority, task.priority)
    body["notes"] = f"Priority: {priority}"

    return body


def _push_calendar_event(
    service: Any,
    calendar_id: str,
    body: dict[str, Any],
    google_event_id: str | None,
) -> dict[str, Any] | None:
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
                .update(calendarId=calendar_id, eventId=google_event_id, body=body)
                .execute()
            )
    except Exception:
        return None


def _push_google_task(
    tasks_service: Any,
    body: dict[str, Any],
    google_task_id: str | None,
) -> dict[str, Any] | None:
    try:
        if google_task_id is None:
            return (
                tasks_service.tasks()
                .insert(tasklist="@default", body=body)
                .execute()
            )
        else:
            body["id"] = google_task_id
            return (
                tasks_service.tasks()
                .update(tasklist="@default", task=google_task_id, body=body)
                .execute()
            )
    except Exception:
        return None


def push_pending(
    calendar_service: Any,
    tasks_service: Any,
    conn: Any,
    calendar_id: str,
    timezone: str,
) -> int:
    """Push pending events to Calendar and pending tasks to Google Tasks."""
    pushed = 0

    for event in get_pending_events(conn):
        body = to_google_event(event, timezone)
        result = _push_calendar_event(calendar_service, calendar_id, body, event.google_event_id)
        if result:
            mark_event_synced(conn, event.id, result["id"], result.get("etag", ""))
            pushed += 1

    for task in get_pending_tasks(conn):
        body = task_to_google_task(task)
        result = _push_google_task(tasks_service, body, task.google_event_id)
        if result:
            mark_task_synced(conn, task.id, result["id"], result.get("etag", ""))
            pushed += 1

    return pushed


def delete_event_from_google(
    calendar_service: Any,
    calendar_id: str,
    google_event_id: str,
) -> bool:
    try:
        calendar_service.events().delete(
            calendarId=calendar_id, eventId=google_event_id
        ).execute()
        return True
    except Exception:
        return False


def delete_task_from_google(
    tasks_service: Any,
    google_task_id: str,
) -> bool:
    try:
        tasks_service.tasks().delete(
            tasklist="@default", task=google_task_id
        ).execute()
        return True
    except Exception:
        return False
