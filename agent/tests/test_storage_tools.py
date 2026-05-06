"""Unit tests for storage_tools.py using FakeIPC (task 7.6, 7.7)."""

from __future__ import annotations

import pytest

from agent import storage_tools as st
from agent.ipc import StorageError
from agent.tests.fake_ipc import FakeIPC


def setup_fake() -> FakeIPC:
    fake = FakeIPC()
    st.set_client(fake)  # type: ignore[arg-type]
    return fake


# ---------------------------------------------------------------------------
# Task 7.6 — verify strands_tools are importable from the package
# ---------------------------------------------------------------------------

def test_strands_tools_importable() -> None:
    """strands_tools must provide current_time, file_read, and file_write."""
    pytest.importorskip("strands_tools", reason="strands_tools not installed")
    from strands_tools import current_time, file_read, file_write  # type: ignore[import]
    assert callable(current_time)
    assert callable(file_read)
    assert callable(file_write)


# ---------------------------------------------------------------------------
# Storage tool round-trips via FakeIPC
# ---------------------------------------------------------------------------

def test_list_tasks_returns_list() -> None:
    fake = setup_fake()
    fake.program("storage.list_tasks", {"tasks": [{"id": 1, "title": "t"}]})
    result = st.list_tasks()
    assert result == [{"id": 1, "title": "t"}]
    assert fake.calls[-1][0] == "storage.list_tasks"


def test_create_task_sends_correct_payload() -> None:
    fake = setup_fake()
    fake.program(
        "storage.create_task",
        {"task": {"id": 42, "title": "tarea", "priority": "alta", "status": "pendiente",
                  "created_at": "2025-01-01T00:00:00+00:00", "deadline": None, "group_id": None}},
    )
    task = st.create_task("tarea", priority="alta")
    assert task["id"] == 42
    assert fake.calls[-1] == ("storage.create_task", {"title": "tarea", "priority": "alta"})


def test_storage_error_propagates() -> None:
    fake = setup_fake()
    fake.program_error("storage.delete_task", "NOT_FOUND", "Task 99 not found")
    with pytest.raises(StorageError) as exc_info:
        st.delete_task(99)
    assert exc_info.value.code == "NOT_FOUND"
    assert "99" in exc_info.value.message


def test_create_group_roundtrip() -> None:
    fake = setup_fake()
    fake.program(
        "storage.create_group",
        {"group": {"id": 1, "name": "trabajo", "color": "#3465A4"}},
    )
    group = st.create_group("trabajo", "#3465A4")
    assert group["name"] == "trabajo"


def test_complete_task() -> None:
    fake = setup_fake()
    fake.program(
        "storage.complete_task",
        {"task": {"id": 5, "status": "completada", "title": "x",
                  "priority": "media", "created_at": "2025-01-01T00:00:00+00:00",
                  "deadline": None, "group_id": None}},
    )
    task = st.complete_task(5)
    assert task["status"] == "completada"
    assert fake.calls[-1] == ("storage.complete_task", {"id": 5})
