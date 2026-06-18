"""Storage Tools — proxy @tools that round-trip to the TUI via StdioClient.

Each function calls ``StdioClient.send_request(type, payload)`` which writes a
``storage.*`` request envelope to stdout and blocks until the TUI responds.
A ``StorageError`` is raised (and caught by the agent framework) if the TUI
returns an error envelope.
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
    """List tasks, optionally filtered by status, group, and date range.

    status: "pendiente" | "completada" | "cancelada" (or English: pending/completed/cancelled)
    group_id: filter by group ID, or omit for all groups
    from_date: ISO 8601 datetime, e.g. "2026-05-01T00:00:00+00:00"
    to_date: ISO 8601 datetime, e.g. "2026-05-31T23:59:59+00:00"
    Returns a list of task dicts.
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
def search_tasks(query: str) -> List[Dict[str, Any]]:
    """Search tasks by title (case-insensitive). Returns all matching tasks regardless of status.

    query: text to search for in task titles, e.g. "community builder" or "deploy".
           Pass a simple string, not JSON.
    Use this when the user asks to find, locate, or check for tasks by name/topic.
    Returns a list of matching task dicts (may include completed/cancelled tasks).
    """
    import json as _j
    import re as _re
    try:
        parsed = _j.loads(query)
        if isinstance(parsed, dict):
            query = parsed.get("query") or parsed.get("search_term") or query
    except (ValueError, TypeError):
        pass
    m = _re.search(r'["\']?\s*:\s*["\'](.+?)["\']', query)
    if m and ('"' in query or "query" in query or "search_term" in query):
        query = m.group(1)
    result = _send("storage.search_tasks", {"query": query})
    return result.get("tasks", [])



@tool
def create_task(
    title: str,
    priority: Optional[str] = None,
    deadline: Optional[str] = None,
    group_id: Optional[int] = None,
) -> Dict[str, Any]:
    """Create a new task.

    title: task description
    priority: "alta" | "media" | "baja" (or English: high/medium/low). Defaults to media.
    deadline: ISO 8601 absolute datetime, e.g. "2026-05-08T10:00:00+00:00".
              Use T00:00:00 only when no specific time was mentioned by the user.
    group_id: group ID (call find_group_by_name first to resolve a name to ID)
    Returns the created task dict.
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
    """Update a task. Only pass the fields you want to change.

    id: task ID to update
    title: new title text
    priority: "alta" | "media" | "baja" (or English: high/medium/low)
    status: "pendiente" | "completada" | "cancelada" (or English: pending/completed/cancelled)
    deadline: ISO 8601 absolute datetime, e.g. "2026-05-08T10:00:00+00:00"
    group_id: group ID (call find_group_by_name first to resolve a name to ID)
    Returns the updated task dict.
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
    """Mark a task as completed by its ID.

    id: task ID to complete
    Returns the updated task dict with status "completada".
    """
    result = _send("storage.complete_task", {"id": id})
    return result["task"]



@tool
def delete_task(id: int) -> None:
    """Delete a task permanently by its ID. This cannot be undone.

    id: task ID to delete
    """
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
    """List calendar events, optionally filtered by date range.

    from_date: start of range in YYYY-MM-DD format, e.g. "2026-05-01"
    to_date: end of range in YYYY-MM-DD format, e.g. "2026-05-31"
    group_id: filter by group ID, or omit for all groups
    Returns a list of event dicts.
    """
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
    """Create a new calendar event.

    title: event name
    start_date: date in YYYY-MM-DD format, e.g. "2026-05-28"
    start_time: time in HH:MM format (24h), e.g. "14:30"
    duration_minutes: length in minutes, or omit for no duration
    group_id: group ID (call find_group_by_name first to resolve a name to ID)
    Returns the created event dict.
    """
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
    """Update a calendar event. Only pass the fields you want to change.

    id: event ID to update
    title: new event name
    start_date: date in YYYY-MM-DD format, e.g. "2026-05-28"
    start_time: time in HH:MM format (24h), e.g. "14:30"
    duration_minutes: length in minutes
    group_id: group ID (call find_group_by_name first to resolve a name to ID)
    Returns the updated event dict.
    """
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
    """Delete a calendar event permanently by its ID. This cannot be undone.

    id: event ID to delete
    """
    _send("storage.delete_event", {"id": id})


# ---------------------------------------------------------------------------
# Groups
# ---------------------------------------------------------------------------


@tool
def list_groups() -> List[Dict[str, Any]]:
    """List all groups. Returns a list of group dicts with id, name, and color."""
    result = _send("storage.list_groups")
    return result.get("groups", [])



@tool
def find_group_by_name(name: str) -> Optional[Dict[str, Any]]:
    """Find a group by name (case-insensitive). Returns the group dict or None.

    name: group name to search for
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
    """Create a new group.

    name: group name (must be unique, case-insensitive)
    color: hex color in #RRGGBB format, e.g. "#FF5733"
    Returns the created group dict.
    """
    result = _send("storage.create_group", {"name": name, "color": color})
    return result["group"]



@tool
def rename_group(id: int, name: str) -> Dict[str, Any]:
    """Rename a group.

    id: group ID (call find_group_by_name to resolve a name to ID)
    name: new group name
    Returns the updated group dict.
    """
    result = _send("storage.rename_group", {"id": id, "name": name})
    return result["group"]



@tool
def recolor_group(id: int, color: str) -> Dict[str, Any]:
    """Change a group's color.

    id: group ID (call find_group_by_name to resolve a name to ID)
    color: hex color in #RRGGBB format, e.g. "#FF5733"
    Returns the updated group dict.
    """
    result = _send("storage.recolor_group", {"id": id, "color": color})
    return result["group"]



@tool
def delete_group(id: int) -> None:
    """Delete a group permanently. Tasks and events in this group will have their group removed.

    id: group ID to delete
    """
    _send("storage.delete_group", {"id": id})


# ---------------------------------------------------------------------------
# Export
# ---------------------------------------------------------------------------


@tool
def export_markdown(output_path: str) -> str:
    """Export all data to a Markdown file.

    output_path: absolute file path to write, e.g. "/tmp/export.md"
    Returns the written file path.
    """
    result = _send("storage.export_markdown", {"output_path": output_path})
    return result["written_path"]



@tool
def export_sqlite(output_path: str) -> str:
    """Export all data to a SQLite file.

    output_path: absolute file path to write, e.g. "/tmp/export.sqlite3"
    Returns the written file path.
    """
    result = _send("storage.export_sqlite", {"output_path": output_path})
    return result["written_path"]


# ---------------------------------------------------------------------------
# Notes
# ---------------------------------------------------------------------------


def _snippet(body: str, term: str, context_lines: int = 5) -> str:
    """Extract a window of lines around the first match of term in body."""
    lines = body.split("\n")
    term_lower = term.lower()
    match_idx = None
    for i, line in enumerate(lines):
        if term_lower in line.lower():
            match_idx = i
            break
    if match_idx is None:
        return "\n".join(lines[:context_lines * 2])
    start = max(0, match_idx - context_lines)
    end = min(len(lines), match_idx + context_lines + 1)
    snippet = "\n".join(lines[start:end])
    if start > 0:
        snippet = "...\n" + snippet
    if end < len(lines):
        snippet = snippet + "\n..."
    return snippet


@tool
def list_notes() -> List[Dict[str, Any]]:
    """List all notes, ordered by most recently updated first.

    Returns a list of note dicts with id, title, body (first 10 lines), created_at, updated_at.
    """
    result = _send("storage.list_notes")
    notes = result.get("notes", [])
    for note in notes:
        if note.get("body"):
            lines = note["body"].split("\n")
            if len(lines) > 10:
                note["body"] = "\n".join(lines[:10]) + "\n..."
    return notes



@tool
def search_notes(search_term: str) -> List[Dict[str, Any]]:
    """Search notes by title or body content (case-insensitive).

    search_term: plain text to search for, e.g. "David" or "despensa".
                 Pass a simple string, not JSON.
    Returns notes where title or body contains the search_term (body is a snippet around the match).
    """
    import json as _j
    import re as _re
    try:
        parsed = _j.loads(search_term)
        if isinstance(parsed, dict):
            search_term = parsed.get("query") or parsed.get("search_term") or search_term
    except (ValueError, TypeError):
        pass
    m = _re.search(r'["\']?\s*:\s*["\'](.+?)["\']', search_term)
    if m and ('"' in search_term or "query" in search_term or "search_term" in search_term):
        search_term = m.group(1)
    result = _send("storage.search_notes", {"query": search_term})
    notes = result.get("notes", [])
    for note in notes:
        if note.get("body"):
            note["body"] = _snippet(note["body"], search_term)
    return notes



@tool
def create_note(title: str, body: str = "") -> Dict[str, Any]:
    """Create a new note.

    title: note title
    body: note content, supports markdown (headers, lists, bold, code blocks)
    Returns the created note dict.
    """
    result = _send("storage.create_note", {"title": title, "body": body})
    return result["note"]



@tool
def update_note(id: int, title: Optional[str] = None, body: Optional[str] = None) -> Dict[str, Any]:
    """Update a note. Only pass the fields you want to change.

    id: note ID to update
    title: new title text
    body: new body content (markdown supported)
    Returns the updated note dict.
    """
    patch: Dict[str, Any] = {}
    if title is not None:
        patch["title"] = title
    if body is not None:
        patch["body"] = body
    result = _send("storage.update_note", {"id": id, "patch": patch})
    return result["note"]



@tool
def delete_note(id: int) -> None:
    """Delete a note permanently by its ID. This cannot be undone.

    id: note ID to delete
    """
    _send("storage.delete_note", {"id": id})


@tool
def export_note(id: int, output_path: str) -> str:
    """Export a single note to a Markdown file.

    id: note ID to export (call list_notes or search_notes first to find it)
    output_path: absolute file path to write, e.g. "/tmp/my-note.md"
    Do NOT use export_markdown for individual notes — that exports all tasks/events.
    Returns the written file path.
    """
    result = _send("storage.export_note", {"id": id, "output_path": output_path})
    return result["written_path"]


# ---------------------------------------------------------------------------
# Finance — Transactions
# ---------------------------------------------------------------------------


@tool
def register_transaction(
    amount: float,
    tx_type: str,
    category: str,
    date: str,
    description: str = "",
    group_id: Optional[int] = None,
) -> Dict[str, Any]:
    """Register a financial transaction (income or expense).

    amount: the amount in currency units (e.g. 150.50 = $150.50). Stored as cents internally.
    tx_type: "ingreso" or "gasto" (income/expense)
    category: spending category, e.g. "comida", "transporte", "servicios", "salario"
    date: ISO date, e.g. "2026-06-18"
    description: optional note about the transaction
    group_id: optional group ID for color coding
    Returns the created transaction dict.
    """
    tx_type_norm = tx_type.lower()
    if tx_type_norm in ("income", "ingreso"):
        tx_type_norm = "ingreso"
    elif tx_type_norm in ("expense", "gasto"):
        tx_type_norm = "gasto"
    else:
        raise StorageError("VALIDATION_FAILED", f"tx_type must be ingreso/gasto, got {tx_type!r}")
    cents = int(round(amount * 100))
    payload: Dict[str, Any] = {
        "amount": cents, "tx_type": tx_type_norm, "category": category,
        "description": description, "date": date,
    }
    if group_id is not None:
        payload["group_id"] = group_id
    result = _send("storage.finance.create_transaction", payload)
    return result["transaction"]


@tool
def list_transactions(
    tx_type: Optional[str] = None,
    category: Optional[str] = None,
    from_date: Optional[str] = None,
    to_date: Optional[str] = None,
) -> List[Dict[str, Any]]:
    """List financial transactions, optionally filtered.

    tx_type: "ingreso" or "gasto" to filter by type
    category: filter by category name
    from_date: ISO date, e.g. "2026-06-01"
    to_date: ISO date, e.g. "2026-06-30"
    Returns a list of transaction dicts with amounts in cents.
    """
    payload: Dict[str, Any] = {}
    if tx_type is not None:
        payload["tx_type"] = tx_type.lower()
    if category is not None:
        payload["category"] = category
    if from_date is not None:
        payload["from_date"] = from_date
    if to_date is not None:
        payload["to_date"] = to_date
    result = _send("storage.finance.list_transactions", payload or None)
    return result.get("transactions", [])


@tool
def get_monthly_summary(month: str) -> Dict[str, Any]:
    """Get a financial summary for a given month.

    month: YYYY-MM format, e.g. "2026-06"
    Returns dict with total_income, total_expenses, balance (all in cents), and savings_rate (%).
    """
    result = _send("storage.finance.monthly_summary", {"month": month})
    return result


@tool
def get_budget_status(month: str) -> List[Dict[str, Any]]:
    """Get budget vs actual spending for a month.

    month: YYYY-MM format, e.g. "2026-06"
    Returns list of {category, monthly_limit, spent} (amounts in cents).
    """
    result = _send("storage.finance.budget_status", {"month": month})
    return result.get("items", [])


# ---------------------------------------------------------------------------
# Finance — Recurring Rules
# ---------------------------------------------------------------------------


@tool
def create_recurring_rule(
    amount: float,
    tx_type: str,
    category: str,
    period: str,
    next_due: str,
    description: str = "",
    day_of_month: Optional[int] = None,
    group_id: Optional[int] = None,
) -> Dict[str, Any]:
    """Create a recurring transaction rule (auto-generates transactions each period).

    amount: amount in currency units (e.g. 5000.00)
    tx_type: "ingreso" or "gasto"
    category: e.g. "salario", "renta", "netflix"
    period: "weekly", "biweekly", or "monthly"
    next_due: next date to generate, e.g. "2026-07-01"
    description: optional note
    day_of_month: for monthly, which day (1-31)
    Returns the created recurring rule dict.
    """
    tx_type_norm = "ingreso" if tx_type.lower() in ("income", "ingreso") else "gasto"
    cents = int(round(amount * 100))
    payload: Dict[str, Any] = {
        "amount": cents, "tx_type": tx_type_norm, "category": category,
        "description": description, "period": period.lower(), "next_due": next_due,
    }
    if day_of_month is not None:
        payload["day_of_month"] = day_of_month
    if group_id is not None:
        payload["group_id"] = group_id
    result = _send("storage.finance.create_recurring_rule", payload)
    return result["rule"]


@tool
def list_recurring_rules() -> List[Dict[str, Any]]:
    """List all active recurring transaction rules.

    Returns list of recurring rule dicts.
    """
    result = _send("storage.finance.list_recurring_rules")
    return result.get("rules", [])


@tool
def delete_recurring_rule(id: int) -> None:
    """Deactivate a recurring rule (stops generating future transactions).

    id: recurring rule ID
    """
    _send("storage.finance.delete_recurring_rule", {"id": id})


# ---------------------------------------------------------------------------
# Finance — Debts
# ---------------------------------------------------------------------------


@tool
def register_debt(
    creditor: str,
    total_amount: float,
    monthly_payment: float,
    interest_rate: Optional[float] = None,
    due_day: Optional[int] = None,
) -> Dict[str, Any]:
    """Register a debt to track.

    creditor: who you owe, e.g. "Tarjeta BBVA", "Préstamo auto"
    total_amount: original/current total debt in currency units
    monthly_payment: how much you pay per month
    interest_rate: annual percentage rate, e.g. 18.5 for 18.5%
    due_day: day of month payment is due (1-31)
    Returns the created debt dict (amounts in cents).
    """
    from datetime import date
    payload: Dict[str, Any] = {
        "creditor": creditor,
        "total_amount": int(round(total_amount * 100)),
        "remaining_amount": int(round(total_amount * 100)),
        "monthly_payment": int(round(monthly_payment * 100)),
        "start_date": date.today().isoformat(),
    }
    if interest_rate is not None:
        payload["interest_rate"] = interest_rate
    if due_day is not None:
        payload["due_day"] = due_day
    result = _send("storage.finance.create_debt", payload)
    return result["debt"]


@tool
def update_debt_payment(id: int, remaining_amount: Optional[float] = None, monthly_payment: Optional[float] = None) -> Dict[str, Any]:
    """Update a debt (e.g. after making a payment).

    id: debt ID
    remaining_amount: new remaining balance in currency units
    monthly_payment: new monthly payment amount
    Returns the updated debt dict.
    """
    payload: Dict[str, Any] = {"id": id}
    if remaining_amount is not None:
        payload["remaining_amount"] = int(round(remaining_amount * 100))
    if monthly_payment is not None:
        payload["monthly_payment"] = int(round(monthly_payment * 100))
    result = _send("storage.finance.update_debt", payload)
    return result["debt"]


@tool
def list_debts() -> List[Dict[str, Any]]:
    """List all active debts (remaining > 0).

    Returns list of debt dicts with amounts in cents.
    """
    result = _send("storage.finance.list_debts")
    return result.get("debts", [])


# ---------------------------------------------------------------------------
# Finance — Goals
# ---------------------------------------------------------------------------


@tool
def register_goal(
    name: str,
    target_amount: float,
    horizon: str,
    current_amount: float = 0,
    deadline: Optional[str] = None,
) -> Dict[str, Any]:
    """Create a financial goal.

    name: e.g. "Fondo de emergencia", "Viaje Europa", "Enganche casa"
    target_amount: goal amount in currency units
    horizon: "corto" (< 1 year), "mediano" (1-5 years), or "largo" (5+ years)
    current_amount: how much saved so far (default 0)
    deadline: optional target date, e.g. "2027-12-31"
    Returns the created goal dict.
    """
    horizon_norm = horizon.lower()
    if horizon_norm in ("short", "corto"):
        horizon_norm = "corto"
    elif horizon_norm in ("medium", "mediano"):
        horizon_norm = "mediano"
    elif horizon_norm in ("long", "largo"):
        horizon_norm = "largo"
    else:
        raise StorageError("VALIDATION_FAILED", f"horizon must be corto/mediano/largo, got {horizon!r}")
    payload: Dict[str, Any] = {
        "name": name,
        "target_amount": int(round(target_amount * 100)),
        "current_amount": int(round(current_amount * 100)),
        "horizon": horizon_norm,
    }
    if deadline is not None:
        payload["deadline"] = deadline
    result = _send("storage.finance.create_goal", payload)
    return result["goal"]


@tool
def update_goal_progress(id: int, current_amount: Optional[float] = None, target_amount: Optional[float] = None) -> Dict[str, Any]:
    """Update progress on a financial goal.

    id: goal ID
    current_amount: new saved amount in currency units
    target_amount: update the target if needed
    Returns the updated goal dict.
    """
    payload: Dict[str, Any] = {"id": id}
    if current_amount is not None:
        payload["current_amount"] = int(round(current_amount * 100))
    if target_amount is not None:
        payload["target_amount"] = int(round(target_amount * 100))
    result = _send("storage.finance.update_goal", payload)
    return result["goal"]


@tool
def list_goals() -> List[Dict[str, Any]]:
    """List all financial goals with progress.

    Returns list of goal dicts with amounts in cents.
    """
    result = _send("storage.finance.list_goals")
    return result.get("goals", [])
