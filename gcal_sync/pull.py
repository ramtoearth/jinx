"""Pull logic: Google Calendar/Tasks → jinx.

Uses Calendar syncToken for efficient incremental sync.
Uses timestamp comparison for Tasks (no syncToken support).
Conflict resolution: jinx wins (skip items with push_pending=1).
"""

from __future__ import annotations

import datetime
from typing import Any

from gcal_sync.db import (
    delete_event_by_google_id,
    delete_task_by_google_id,
    find_event_by_google_id,
    find_task_by_google_id,
    get_sync_state,
    set_calendar_sync_token,
    set_tasks_last_sync,
    upsert_event_from_google,
    upsert_task_from_google,
)


def _parse_google_event(item: dict[str, Any], timezone: str) -> tuple[str, str, int | None]:
    """Extract start_date, start_time, duration_minutes from a Google Calendar event."""
    start = item.get("start", {})
    end = item.get("end", {})

    if "dateTime" in start:
        start_dt = datetime.datetime.fromisoformat(start["dateTime"])
        start_date = start_dt.strftime("%Y-%m-%d")
        start_time = start_dt.strftime("%H:%M")

        if "dateTime" in end:
            end_dt = datetime.datetime.fromisoformat(end["dateTime"])
            duration = int((end_dt - start_dt).total_seconds() / 60)
        else:
            duration = 60
        return start_date, start_time, duration if duration > 0 else None
    elif "date" in start:
        return start["date"], "00:00", None
    else:
        today = datetime.date.today().isoformat()
        return today, "00:00", None


def pull_calendar(
    calendar_service: Any,
    conn: Any,
    calendar_id: str,
    timezone: str,
) -> int:
    """Pull changes from Google Calendar into jinx. Returns count of items processed."""
    sync_token, _ = get_sync_state(conn)
    pulled = 0

    try:
        kwargs: dict[str, Any] = {"calendarId": calendar_id}
        if sync_token:
            kwargs["syncToken"] = sync_token
        else:
            now = datetime.datetime.now(datetime.timezone.utc)
            kwargs["timeMin"] = (now - datetime.timedelta(days=30)).isoformat()
            kwargs["timeMax"] = (now + datetime.timedelta(days=365)).isoformat()
            kwargs["singleEvents"] = True

        result = calendar_service.events().list(**kwargs).execute()
    except Exception as e:
        error_str = str(e)
        if "410" in error_str or "Gone" in error_str:
            # syncToken expired — do full re-sync
            now = datetime.datetime.now(datetime.timezone.utc)
            result = calendar_service.events().list(
                calendarId=calendar_id,
                singleEvents=True,
                timeMin=(now - datetime.timedelta(days=30)).isoformat(),
                timeMax=(now + datetime.timedelta(days=365)).isoformat(),
            ).execute()
        else:
            raise

    for item in result.get("items", []):
        google_id = item.get("id")
        if not google_id:
            continue

        if item.get("status") == "cancelled":
            delete_event_by_google_id(conn, google_id)
            pulled += 1
            continue

        # Conflict resolution: jinx wins
        local = find_event_by_google_id(conn, google_id)
        if local and local["push_pending"] == 1:
            continue

        title = item.get("summary", "(no title)")
        etag = item.get("etag", "")
        start_date, start_time, duration = _parse_google_event(item, timezone)

        upsert_event_from_google(
            conn, google_id, title, start_date, start_time, duration, etag
        )
        pulled += 1

    new_token = result.get("nextSyncToken")
    if new_token:
        set_calendar_sync_token(conn, new_token)

    return pulled


def _parse_google_task_priority(notes: str | None) -> str:
    """Extract priority from task notes field."""
    if not notes:
        return "media"
    lower = notes.lower()
    if "high" in lower:
        return "alta"
    if "low" in lower:
        return "baja"
    return "media"


def _parse_google_task_deadline(due: str | None) -> str | None:
    """Convert Google Tasks 'due' field to jinx deadline format."""
    if not due:
        return None
    try:
        dt = datetime.datetime.fromisoformat(due.replace("Z", "+00:00"))
        return dt.strftime("%Y-%m-%dT%H:%M:%S+00:00")
    except (ValueError, TypeError):
        return None


def pull_tasks(
    tasks_service: Any,
    conn: Any,
) -> int:
    """Pull changes from Google Tasks into jinx. Returns count of items processed."""
    _, last_sync = get_sync_state(conn)
    pulled = 0

    try:
        result = tasks_service.tasks().list(
            tasklist="@default", showCompleted=True, showHidden=True
        ).execute()
    except Exception:
        raise

    for item in result.get("items", []):
        google_id = item.get("id")
        if not google_id:
            continue

        updated = item.get("updated", "")
        if last_sync and updated <= last_sync:
            continue

        if item.get("deleted"):
            delete_task_by_google_id(conn, google_id)
            pulled += 1
            continue

        # Conflict resolution: jinx wins
        local = find_task_by_google_id(conn, google_id)
        if local and local["push_pending"] == 1:
            continue

        title = item.get("title", "(no title)")
        etag = item.get("etag", "")
        status = "completada" if item.get("status") == "completed" else "pendiente"
        priority = _parse_google_task_priority(item.get("notes"))
        deadline = _parse_google_task_deadline(item.get("due"))

        upsert_task_from_google(
            conn, google_id, title, priority, status, deadline, etag
        )
        pulled += 1

    now = datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%S.000Z")
    set_tasks_last_sync(conn, now)

    return pulled
