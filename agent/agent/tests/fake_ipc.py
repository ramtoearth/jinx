"""FakeIPC — in-process replacement for StdioClient used in agent unit tests.

Supports programmed responses and error injection.
"""

from __future__ import annotations

from typing import Any, Callable, Dict, Iterator, List, Optional, Tuple

from agent.ipc import Envelope, StorageError


class FakeIPC:
    """Synchronous fake that replaces StdioClient in tests.

    Usage::

        fake = FakeIPC()
        fake.program("storage.create_task", {"task": {...}})
        result = fake.send_request("storage.create_task", {"title": "..."})
    """

    def __init__(self) -> None:
        self._programs: List[Tuple[str, Any]] = []
        self._calls: List[Tuple[str, Optional[Dict[str, Any]]]] = []
        self._written: List[Envelope] = []

    def program(self, msg_type: str, response: Any) -> None:
        """Queue a response for the next call matching ``msg_type``."""
        self._programs.append((msg_type, response))

    def program_error(self, msg_type: str, code: str, message: str) -> None:
        """Queue an error for the next call matching ``msg_type``."""
        self._programs.append((msg_type, StorageError(code, message)))

    def send_request(
        self, msg_type: str, payload: Optional[Dict[str, Any]]
    ) -> Dict[str, Any]:
        self._calls.append((msg_type, payload))
        for i, (expected_type, result) in enumerate(self._programs):
            if expected_type == msg_type:
                self._programs.pop(i)
                if isinstance(result, StorageError):
                    raise result
                return result if isinstance(result, dict) else {}
        raise RuntimeError(f"FakeIPC: no response programmed for {msg_type!r}")

    def write_envelope(self, env: Envelope) -> None:
        self._written.append(env)

    def incoming_lines(self) -> Iterator[Envelope]:
        return iter([])

    @property
    def calls(self) -> List[Tuple[str, Optional[Dict[str, Any]]]]:
        """All (type, payload) tuples sent via send_request."""
        return list(self._calls)

    @property
    def written(self) -> List[Envelope]:
        """All envelopes written via write_envelope."""
        return list(self._written)

    def reset(self) -> None:
        self._programs.clear()
        self._calls.clear()
        self._written.clear()
