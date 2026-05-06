"""Canal_IPC JSONL sobre stdio del Agente.

Defines the Python-side TypedDicts matching the Rust ``Envelope`` and all
payload structs in ``tui/src/ipc.rs``.  Also provides ``encode``/``decode``
for the JSON Lines wire format and the ``StdioClient`` that the Storage Tools
use to make synchronous round-trips to the TUI.
"""

from __future__ import annotations

import json
import queue
import sys
import threading
import uuid
from typing import Any, Dict, Iterator, Literal, Optional, TypedDict


# ---------------------------------------------------------------------------
# Protocol constant
# ---------------------------------------------------------------------------

PROTOCOL_VERSION: int = 1

# ---------------------------------------------------------------------------
# Envelope TypedDicts
# ---------------------------------------------------------------------------

Kind = Literal["request", "response", "event"]


class StorageErrorPayload(TypedDict, total=False):
    code: str
    message: str


class Envelope(TypedDict, total=False):
    v: int
    id: str
    kind: Kind
    type: str
    payload: Optional[Dict[str, Any]]
    ref: Optional[str]
    error: Optional[StorageErrorPayload]


# ---------------------------------------------------------------------------
# Encode / decode helpers (task 2.3)
# ---------------------------------------------------------------------------

def encode(env: Envelope) -> str:
    """Serialize an ``Envelope`` to a single JSON Line (no trailing newline)."""
    return json.dumps(env, ensure_ascii=False)


def decode(line: str) -> Envelope:
    """Deserialize a single JSON Line into an ``Envelope``.

    Raises ``ValueError`` if ``v`` is present and not equal to
    ``PROTOCOL_VERSION``.
    """
    env: Envelope = json.loads(line)
    v = env.get("v")  # type: ignore[attr-defined]
    if v is not None and v != PROTOCOL_VERSION:
        raise ValueError(
            f"unsupported IPC protocol version: expected {PROTOCOL_VERSION}, got {v}"
        )
    return env


# ---------------------------------------------------------------------------
# StorageError exception
# ---------------------------------------------------------------------------

class StorageError(Exception):
    """Raised when the TUI returns an error response to a storage request."""

    def __init__(self, code: str, message: str) -> None:
        super().__init__(message)
        self.code = code
        self.message = message


# ---------------------------------------------------------------------------
# StdioClient — synchronous request/response over JSONL stdio
# ---------------------------------------------------------------------------

_SENTINEL = object()


class StdioClient:
    """Correlates ``request``/``response`` pairs by ``ref`` over stdio.

    The Agent process reads from ``stdin`` and writes to ``stdout``.  Every
    call to :meth:`send_request` writes a request envelope to ``stdout`` and
    then blocks until the TUI sends back a response envelope whose ``ref``
    matches the request ``id``.

    A background reader thread feeds incoming lines into the appropriate
    destination:

    - Storage responses (``kind == "response"`` with a matching ``ref``) are
      dispatched directly to the waiting :meth:`send_request` caller.
    - Everything else (``user_message``, ``agent_init``, ``shutdown``, …) is
      placed on ``_inbox`` so :meth:`incoming_lines` can yield it.
    """

    def __init__(
        self,
        stdin: Any = None,
        stdout: Any = None,
    ) -> None:
        self._stdin = stdin or sys.stdin
        self._stdout = stdout or sys.stdout
        self._write_lock = threading.Lock()
        self._pending_lock = threading.Lock()
        # Maps request id → (response_or_None, Event)
        self._pending: Dict[str, list[Any]] = {}
        # Queue for non-storage envelopes destined to the main loop
        self._inbox: queue.Queue[Any] = queue.Queue()
        self._reader = threading.Thread(target=self._read_loop, daemon=True)
        self._reader.start()

    # ------------------------------------------------------------------
    # Public API
    # ------------------------------------------------------------------

    def send_request(
        self, message_type: str, payload: Optional[Dict[str, Any]]
    ) -> Dict[str, Any]:
        """Send a storage request and block until the response arrives.

        Returns the response ``payload`` on success.  Raises
        :class:`StorageError` if the response carries an ``error``.
        """
        req_id = str(uuid.uuid4())
        event = threading.Event()
        slot: list[Any] = [None, event]
        with self._pending_lock:
            self._pending[req_id] = slot

        env: Envelope = {
            "v": PROTOCOL_VERSION,
            "id": req_id,
            "kind": "request",
            "type": message_type,
        }
        if payload is not None:
            env["payload"] = payload

        self._write(env)
        event.wait()

        with self._pending_lock:
            del self._pending[req_id]

        response: Optional[Envelope] = slot[0]
        if response is None:
            raise StorageError("INTERNAL_ERROR", "no response received")

        err = response.get("error")  # type: ignore[attr-defined]
        if err:
            raise StorageError(err["code"], err["message"])

        return response.get("payload") or {}  # type: ignore[attr-defined]

    def write_envelope(self, env: Envelope) -> None:
        """Write any envelope to stdout (e.g. agent_reply, events)."""
        self._write(env)

    def incoming_lines(self) -> Iterator[Envelope]:
        """Iterate over non-storage-response envelopes from the TUI.

        Storage responses are consumed internally by :meth:`send_request`;
        everything else (``user_message``, ``agent_init``, ``shutdown``, …)
        is yielded here so the main loop can act on them.  Iteration ends
        when the reader thread signals EOF.
        """
        while True:
            item = self._inbox.get()
            if item is _SENTINEL:
                return
            yield item

    # ------------------------------------------------------------------
    # Internal helpers
    # ------------------------------------------------------------------

    def _write(self, env: Envelope) -> None:
        line = encode(env) + "\n"
        with self._write_lock:
            self._stdout.write(line)
            self._stdout.flush()

    def _read_loop(self) -> None:
        try:
            for raw_line in self._stdin:
                line = raw_line.rstrip("\n")
                if not line:
                    continue
                try:
                    env = decode(line)
                except (ValueError, json.JSONDecodeError):
                    continue
                self._dispatch(env)
        finally:
            # Signal incoming_lines() that the stream is closed
            self._inbox.put(_SENTINEL)
            # Wake any waiting send_request callers
            with self._pending_lock:
                for slot in self._pending.values():
                    if slot[1] is not None:
                        slot[1].set()  # type: ignore[union-attr]

    def _dispatch(self, env: Envelope) -> None:
        ref = env.get("ref")  # type: ignore[attr-defined]
        kind = env.get("kind")  # type: ignore[attr-defined]
        if kind == "response" and ref:
            with self._pending_lock:
                slot = self._pending.get(ref)
            if slot is not None:
                slot[0] = env
                slot[1].set()  # type: ignore[union-attr]
                return
        self._inbox.put(env)
