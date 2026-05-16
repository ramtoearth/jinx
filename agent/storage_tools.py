"""Storage Tools — proxy @tools that round-trip to the TUI via StdioClient.

Each function calls ``StdioClient.send_request(type, payload)`` which writes a
``storage.*`` request envelope to stdout and blocks until the TUI responds.
A ``StorageError`` is raised (and caught by the agent framework) if the TUI
returns an error envelope.

Requisitos 2.1–2.5, 3.1–3.5, 10.1–10.2, 11.3, 13.11, 15.1–15.5.
"""

from __future__ import annotations

from typing import Any, Dict, List, Optional

try:
    from strands import tool  # type: ignore[import]
except ImportError:
    def tool(fn):  # type: ignore[misc]
        return fn

from agent.ipc import StorageError, StdioClient

# The global client instance is set by main.py before the Agent runs.
_client: Optional[StdioClient] = None


def set_client(client: StdioClient) -> None:
    global _client
    _client = client


def _send(msg_type: str, payload: Optional[Dict[str, Any]] = None) -> Dict[str, Any]:
    if _client is None:
        raise RuntimeError("StdioClient not initialised")
    return _client.send_request(msg_type, payload)


# ---------------------------------------------------------------------------
# Value normalizers — map English synonyms to the Spanish values the
# storage layer requires, so models that respond in English still work.
# ---------------------------------------------------------------------------

_PRIORITY_MAP: Dict[str, str] = {
    "alta": "alta", "high": "alta", "alto": "alta",
    "media": "media", "medium": "media", "medio": "media", "normal": "media",
    "baja": "baja", "low": "baja", "bajo": "baja",
}

_STATUS_MAP: Dict[str, str] = {
    "pendiente": "pendiente", "pending": "pendiente",
    "completada": "completada", "completed": "completada", "done": "completada",
    "cancelada": "cancelada", "cancelled": "cancelada", "canceled": "cancelada",
}


def _norm_priority(v: Optional[str]) -> Optional[str]:
    if v is None:
        return None
    normalized = _PRIORITY_MAP.get(v.lower())
    if normalized is None:
        raise StorageError("VALIDATION_FAILED", f"priority must be alta/media/baja, got {v!r}")
    return normalized


def _norm_status(v: Optional[str]) -> Optional[str]:
    if v is None:
        return None
    normalized = _STATUS_MAP.get(v.lower())
    if normalized is None:
        raise StorageError("VALIDATION_FAILED", f"status must be pendiente/completada/cancelada, got {v!r}")
    return normalized


# ---------------------------------------------------------------------------
# Tasks
# ---------------------------------------------------------------------------


@tool
def list_tasks(
    status: Optional[str] = None,
    group_id: Optional[int] = None,
    from_date: Optional[str] = None,
    to_date: Optional[str] = None,
) -> List[Dict[str, Any]]:
    """List tasks, optionally filtered by status, group_id, and date range.

    status: "pendiente" | "completada" | "cancelada"  (or English: pending/completed/cancelled)
    """
    payload: Dict[str, Any] = {}
    if status is not None:
        payload["status"] = _norm_status(status)
    if group_id is not None:
        payload["group_id"] = group_id
    if from_date is not None:
        payload["from_date"] = from_date
    if to_date is not None:
        payload["to_date"] = to_date
    result = _send("storage.list_tasks", payload or None)
    return result.get("tasks", [])



@tool
def create_task(
    title: str,
    priority: Optional[str] = None,
    deadline: Optional[str] = None,
    group_id: Optional[int] = None,
) -> Dict[str, Any]:
    """Create a new task and return it.

    priority: "alta" | "media" | "baja"  (or English: high/medium/low)
    deadline: ISO 8601 absolute datetime, e.g. "2026-05-08T10:00:00+00:00"
    """
    payload: Dict[str, Any] = {"title": title}
    if priority is not None:
        payload["priority"] = _norm_priority(priority)
    if deadline is not None:
        payload["deadline"] = deadline
    if group_id is not None:
        payload["group_id"] = group_id
    result = _send("storage.create_task", payload)
    return result["task"]



@tool
def update_task(
    id: int,
    title: Optional[str] = None,
    priority: Optional[str] = None,
    status: Optional[str] = None,
    deadline: Optional[str] = None,
    group_id: Optional[int] = None,
) -> Dict[str, Any]:
    """Update fields of a task. Only provided fields are changed.

    priority: "alta" | "media" | "baja"  (or English: high/medium/low)
    status:   "pendiente" | "completada" | "cancelada"  (or English: pending/completed/cancelled)
    """
    patch: Dict[str, Any] = {}
    if title is not None:
        patch["title"] = title
    if priority is not None:
        patch["priority"] = _norm_priority(priority)
    if status is not None:
        patch["status"] = _norm_status(status)
    if deadline is not None:
        patch["deadline"] = deadline
    if group_id is not None:
        patch["group_id"] = group_id
    result = _send("storage.update_task", {"id": id, "patch": patch})
    return result["task"]



@tool
def complete_task(id: int) -> Dict[str, Any]:
    """Mark a task as completed."""
    result = _send("storage.complete_task", {"id": id})
    return result["task"]



@tool
def delete_task(id: int) -> None:
    """Delete a task permanently."""
    _send("storage.delete_task", {"id": id})


# ---------------------------------------------------------------------------
# Events
# ---------------------------------------------------------------------------


@tool
def list_events(
    from_date: Optional[str] = None,
    to_date: Optional[str] = None,
    group_id: Optional[int] = None,
) -> List[Dict[str, Any]]:
    """List events, optionally filtered by date range and group."""
    payload: Dict[str, Any] = {}
    if from_date is not None:
        payload["from_date"] = from_date
    if to_date is not None:
        payload["to_date"] = to_date
    if group_id is not None:
        payload["group_id"] = group_id
    result = _send("storage.list_events", payload or None)
    return result.get("events", [])



@tool
def create_event(
    title: str,
    start_date: str,
    start_time: str,
    duration_minutes: Optional[int] = None,
    group_id: Optional[int] = None,
) -> Dict[str, Any]:
    """Create a new calendar event and return it."""
    payload: Dict[str, Any] = {
        "title": title,
        "start_date": start_date,
        "start_time": start_time,
    }
    if duration_minutes is not None:
        payload["duration_minutes"] = duration_minutes
    if group_id is not None:
        payload["group_id"] = group_id
    result = _send("storage.create_event", payload)
    return result["event"]



@tool
def update_event(
    id: int,
    title: Optional[str] = None,
    start_date: Optional[str] = None,
    start_time: Optional[str] = None,
    duration_minutes: Optional[int] = None,
    group_id: Optional[int] = None,
) -> Dict[str, Any]:
    """Update fields of a calendar event."""
    patch: Dict[str, Any] = {}
    if title is not None:
        patch["title"] = title
    if start_date is not None:
        patch["start_date"] = start_date
    if start_time is not None:
        patch["start_time"] = start_time
    if duration_minutes is not None:
        patch["duration_minutes"] = duration_minutes
    if group_id is not None:
        patch["group_id"] = group_id
    result = _send("storage.update_event", {"id": id, "patch": patch})
    return result["event"]



@tool
def delete_event(id: int) -> None:
    """Delete a calendar event."""
    _send("storage.delete_event", {"id": id})


# ---------------------------------------------------------------------------
# Groups
# ---------------------------------------------------------------------------


@tool
def list_groups() -> List[Dict[str, Any]]:
    """List all groups."""
    result = _send("storage.list_groups")
    return result.get("groups", [])



@tool
def find_group_by_name(name: str) -> Optional[Dict[str, Any]]:
    """Find a group by name (case-insensitive). Returns the group dict or None.

    ALWAYS call this before create_task/create_event when the user mentions a group name,
    to resolve the group name to its ID. Never guess a group_id.
    """
    result = _send("storage.list_groups")
    groups = result.get("groups", [])
    name_lower = name.lower()
    for g in groups:
        if g.get("name", "").lower() == name_lower:
            return g
    return None


@tool
def create_group(name: str, color: str) -> Dict[str, Any]:
    """Create a new group (color as '#RRGGBB')."""
    result = _send("storage.create_group", {"name": name, "color": color})
    return result["group"]



@tool
def rename_group(id: int, name: str) -> Dict[str, Any]:
    """Rename a group."""
    result = _send("storage.rename_group", {"id": id, "name": name})
    return result["group"]



@tool
def recolor_group(id: int, color: str) -> Dict[str, Any]:
    """Change a group's colour."""
    result = _send("storage.recolor_group", {"id": id, "color": color})
    return result["group"]



@tool
def delete_group(id: int) -> None:
    """Delete a group (tasks/events lose their group_id)."""
    _send("storage.delete_group", {"id": id})


# ---------------------------------------------------------------------------
# Export
# ---------------------------------------------------------------------------


@tool
def export_markdown(output_path: str) -> str:
    """Export all data to a Markdown file and return the written path."""
    result = _send("storage.export_markdown", {"output_path": output_path})
    return result["written_path"]



@tool
def export_sqlite(output_path: str) -> str:
    """Export all data to a SQLite file and return the written path."""
    result = _send("storage.export_sqlite", {"output_path": output_path})
    return result["written_path"]


@tool
def sync_google() -> str:
    """Synchronize with Google Calendar and Google Tasks.

    Pushes local changes to Google and pulls remote changes into jinx.
    Returns a status message indicating what was synced.
    """
    result = _send("storage.sync_google")
    return result.get("status", "Sync completed.")
