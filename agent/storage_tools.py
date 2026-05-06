"""Storage Tools — proxy @tools that round-trip to the TUI via StdioClient.

Each function calls ``StdioClient.send_request(type, payload)`` which writes a
``storage.*`` request envelope to stdout and blocks until the TUI responds.
A ``StorageError`` is raised (and caught by the agent framework) if the TUI
returns an error envelope.

Requisitos 2.1–2.5, 3.1–3.5, 10.1–10.2, 11.3, 13.11, 15.1–15.5.
"""

from __future__ import annotations

from typing import Any, Dict, List, Optional

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
# Tasks
# ---------------------------------------------------------------------------

def list_tasks(
    status: Optional[str] = None,
    group_id: Optional[int] = None,
    from_date: Optional[str] = None,
    to_date: Optional[str] = None,
) -> List[Dict[str, Any]]:
    """List tasks, optionally filtered by status, group_id, and date range."""
    payload: Dict[str, Any] = {}
    if status is not None:
        payload["status"] = status
    if group_id is not None:
        payload["group_id"] = group_id
    if from_date is not None:
        payload["from_date"] = from_date
    if to_date is not None:
        payload["to_date"] = to_date
    result = _send("storage.list_tasks", payload or None)
    return result.get("tasks", [])


def create_task(
    title: str,
    priority: Optional[str] = None,
    deadline: Optional[str] = None,
    group_id: Optional[int] = None,
) -> Dict[str, Any]:
    """Create a new task and return it."""
    payload: Dict[str, Any] = {"title": title}
    if priority is not None:
        payload["priority"] = priority
    if deadline is not None:
        payload["deadline"] = deadline
    if group_id is not None:
        payload["group_id"] = group_id
    result = _send("storage.create_task", payload)
    return result["task"]


def update_task(
    id: int,
    title: Optional[str] = None,
    priority: Optional[str] = None,
    status: Optional[str] = None,
    deadline: Optional[str] = None,
    group_id: Optional[int] = None,
) -> Dict[str, Any]:
    """Update fields of a task."""
    patch: Dict[str, Any] = {}
    if title is not None:
        patch["title"] = title
    if priority is not None:
        patch["priority"] = priority
    if status is not None:
        patch["status"] = status
    if deadline is not None:
        patch["deadline"] = deadline
    if group_id is not None:
        patch["group_id"] = group_id
    result = _send("storage.update_task", {"id": id, "patch": patch})
    return result["task"]


def complete_task(id: int) -> Dict[str, Any]:
    """Mark a task as completed."""
    result = _send("storage.complete_task", {"id": id})
    return result["task"]


def delete_task(id: int) -> None:
    """Delete a task permanently."""
    _send("storage.delete_task", {"id": id})


# ---------------------------------------------------------------------------
# Events
# ---------------------------------------------------------------------------

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


def delete_event(id: int) -> None:
    """Delete a calendar event."""
    _send("storage.delete_event", {"id": id})


# ---------------------------------------------------------------------------
# Groups
# ---------------------------------------------------------------------------

def list_groups() -> List[Dict[str, Any]]:
    """List all groups."""
    result = _send("storage.list_groups")
    return result.get("groups", [])


def create_group(name: str, color: str) -> Dict[str, Any]:
    """Create a new group (color as '#RRGGBB')."""
    result = _send("storage.create_group", {"name": name, "color": color})
    return result["group"]


def rename_group(id: int, name: str) -> Dict[str, Any]:
    """Rename a group."""
    result = _send("storage.rename_group", {"id": id, "name": name})
    return result["group"]


def recolor_group(id: int, color: str) -> Dict[str, Any]:
    """Change a group's colour."""
    result = _send("storage.recolor_group", {"id": id, "color": color})
    return result["group"]


def delete_group(id: int) -> None:
    """Delete a group (tasks/events lose their group_id)."""
    _send("storage.delete_group", {"id": id})


# ---------------------------------------------------------------------------
# Export
# ---------------------------------------------------------------------------

def export_markdown(output_path: str) -> str:
    """Export all data to a Markdown file and return the written path."""
    result = _send("storage.export_markdown", {"output_path": output_path})
    return result["written_path"]


def export_sqlite(output_path: str) -> str:
    """Export all data to a SQLite file and return the written path."""
    result = _send("storage.export_sqlite", {"output_path": output_path})
    return result["written_path"]
