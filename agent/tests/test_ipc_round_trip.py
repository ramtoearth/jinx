# Feature: terminal-day-organizer, Property 1: Round-trip de serialización IPC
#
# Property 1: Round-trip de serialización IPC
# Para cualquier mensaje válido del Canal_IPC, serializar el mensaje a JSON,
# enviarlo por el transporte stdio y deserializarlo en el extremo opuesto SHALL
# producir un mensaje equivalente al original bajo el contrato de igualdad
# definido en design.md.
#
# Valida: Requisitos 10.1, 10.2, 10.3
"""Property-based round-trip tests for the Python-side IPC envelope.

Uses ``hypothesis`` to generate valid Envelope dicts and verify that
``decode(encode(env)) == env`` (structural equality, key-order independent).
"""

from __future__ import annotations

import json

import pytest
from hypothesis import given, settings
from hypothesis import strategies as st

from agent.ipc import PROTOCOL_VERSION, decode, encode

# ---------------------------------------------------------------------------
# Primitive strategies
# ---------------------------------------------------------------------------

_KINDS = st.sampled_from(["request", "response", "event"])

_MESSAGE_TYPES = st.sampled_from(
    [
        "user_message",
        "agent_reply",
        "agent_init",
        "agent_init_ack",
        "shutdown",
        "agent_tool_progress",
        "storage.list_tasks",
        "storage.create_task",
        "storage.update_task",
        "storage.complete_task",
        "storage.delete_task",
        "storage.list_events",
        "storage.create_event",
        "storage.update_event",
        "storage.delete_event",
        "storage.list_groups",
        "storage.create_group",
        "storage.rename_group",
        "storage.recolor_group",
        "storage.delete_group",
        "storage.export_markdown",
        "storage.export_sqlite",
    ]
)

_TEXT = st.text(max_size=64)
_ISO_TS = st.from_regex(
    r"2[0-9]{3}-[01][0-9]-[0-3][0-9]T[0-2][0-9]:[0-5][0-9]:[0-5][0-9][+-][01][0-9]:[0-5][0-9]",
    fullmatch=True,
)
_DATE = st.from_regex(r"2[0-9]{3}-[01][0-9]-[0-3][0-9]", fullmatch=True)
_TIME = st.from_regex(r"[0-2][0-9]:[0-5][0-9]", fullmatch=True)
_HEX_COLOR = st.from_regex(r"#[0-9a-fA-F]{6}", fullmatch=True)
_PRIORITY = st.sampled_from(["alta", "media", "baja"])
_STATUS = st.sampled_from(["pendiente", "completada", "cancelada"])
_UUID = st.uuids().map(str)


def _task_dto() -> st.SearchStrategy[dict]:  # type: ignore[type-arg]
    return st.fixed_dictionaries(
        {
            "id": st.integers(),
            "title": _TEXT,
            "priority": _PRIORITY,
            "status": _STATUS,
            "created_at": _ISO_TS,
            "deadline": st.one_of(st.none(), _ISO_TS),
            "group_id": st.one_of(st.none(), st.integers()),
        }
    )


def _event_dto() -> st.SearchStrategy[dict]:  # type: ignore[type-arg]
    return st.fixed_dictionaries(
        {
            "id": st.integers(),
            "title": _TEXT,
            "start_date": _DATE,
            "start_time": _TIME,
            "duration_minutes": st.one_of(st.none(), st.integers(min_value=0, max_value=600)),
            "group_id": st.one_of(st.none(), st.integers()),
        }
    )


def _group_dto() -> st.SearchStrategy[dict]:  # type: ignore[type-arg]
    return st.fixed_dictionaries(
        {"id": st.integers(), "name": _TEXT, "color": _HEX_COLOR}
    )


# ---------------------------------------------------------------------------
# Payload strategies (representative subset)
# ---------------------------------------------------------------------------

_PAYLOADS = st.one_of(
    # lifecycle
    st.fixed_dictionaries({"text": _TEXT}),
    st.fixed_dictionaries({"timezone": _TEXT, "model_provider": st.sampled_from(["local", "remote"])}),
    st.fixed_dictionaries({"provider_notice": st.one_of(st.none(), _TEXT)}),
    st.fixed_dictionaries({"tool": _TEXT, "phase": st.sampled_from(["start", "end"])}),
    # tasks
    st.fixed_dictionaries(
        {
            "title": _TEXT,
            "priority": st.one_of(st.none(), _PRIORITY),
            "deadline": st.one_of(st.none(), _ISO_TS),
            "group_id": st.one_of(st.none(), st.integers()),
        }
    ),
    st.fixed_dictionaries({"task": _task_dto()}),
    st.fixed_dictionaries({"tasks": st.lists(_task_dto(), max_size=5)}),
    # events
    st.fixed_dictionaries(
        {
            "title": _TEXT,
            "start_date": _DATE,
            "start_time": _TIME,
            "duration_minutes": st.one_of(st.none(), st.integers(min_value=0, max_value=600)),
            "group_id": st.one_of(st.none(), st.integers()),
        }
    ),
    st.fixed_dictionaries({"event": _event_dto()}),
    # groups
    st.fixed_dictionaries({"name": _TEXT, "color": _HEX_COLOR}),
    st.fixed_dictionaries({"group": _group_dto()}),
    st.fixed_dictionaries({"groups": st.lists(_group_dto(), max_size=5)}),
    # export
    st.fixed_dictionaries({"output_path": _TEXT}),
    st.fixed_dictionaries({"written_path": _TEXT}),
)

_ERROR_CODES = st.sampled_from(
    [
        "NOT_FOUND",
        "GROUP_NAME_NOT_UNIQUE",
        "VALIDATION_FAILED",
        "IO_NOT_WRITABLE",
        "IO_READ_FAILED",
        "SCHEMA_MIGRATION_FAILED",
        "INTERNAL_ERROR",
    ]
)

_OPTIONAL_ERROR = st.one_of(
    st.none(),
    st.fixed_dictionaries({"code": _ERROR_CODES, "message": _TEXT}),
)


def _envelope_strategy() -> st.SearchStrategy[dict]:  # type: ignore[type-arg]
    return st.fixed_dictionaries(
        {
            "v": st.just(PROTOCOL_VERSION),
            "id": _UUID,
            "kind": _KINDS,
            "type": _MESSAGE_TYPES,
            "payload": st.one_of(st.none(), _PAYLOADS),
            "ref": st.one_of(st.none(), _UUID),
            "error": _OPTIONAL_ERROR,
        }
    ).map(
        lambda d: {
            # Wire invariant: error responses carry no payload
            **d,
            "payload": None if d["error"] is not None else d["payload"],
        }
    )


# ---------------------------------------------------------------------------
# Feature: terminal-day-organizer, Property 1: Round-trip de serialización IPC
# ---------------------------------------------------------------------------
# Valida: Requisitos 10.1, 10.2, 10.3

@settings(max_examples=200, deadline=None)
@given(_envelope_strategy())
def test_ipc_round_trip_python(envelope: dict) -> None:  # type: ignore[type-arg]
    """decode(encode(env)) == env for any well-formed Envelope."""
    # Remove None-valued optional keys so the comparison is key-order-independent
    # and matches the encode/decode cycle (which drops None values).
    def drop_nones(d: dict) -> dict:  # type: ignore[type-arg]
        return {k: v for k, v in d.items() if v is not None}

    cleaned = drop_nones(envelope)
    line = encode(cleaned)  # type: ignore[arg-type]
    decoded = decode(line)
    # Compare via JSON round-trip (structural equality, key-order irrelevant)
    assert json.loads(json.dumps(decoded)) == json.loads(json.dumps(cleaned))


def test_decode_rejects_wrong_version() -> None:
    line = json.dumps(
        {
            "v": 99,
            "id": "00000000-0000-4000-8000-000000000000",
            "kind": "request",
            "type": "user_message",
            "payload": {"text": "hola"},
        }
    )
    with pytest.raises(ValueError, match="unsupported IPC protocol version"):
        decode(line)


def test_decode_accepts_missing_v() -> None:
    """Envelopes without a ``v`` field pass (version check only fires if present)."""
    line = json.dumps(
        {
            "id": "00000000-0000-4000-8000-000000000000",
            "kind": "request",
            "type": "shutdown",
        }
    )
    env = decode(line)
    assert env["type"] == "shutdown"
