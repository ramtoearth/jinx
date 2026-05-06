#!/usr/bin/env python3
"""IPC echo helper for the cross-process round-trip test (Task 2.6).

Reads JSON Lines from stdin, deserializes each line with ``agent.ipc.decode``,
re-serializes it with ``agent.ipc.encode``, and writes the result back to
stdout.  Used by ``tui/tests/cross_ipc.rs`` to verify Python ↔ Rust round-trip
fidelity.
"""

from __future__ import annotations

import json
import sys

sys.path.insert(0, str(__import__("pathlib").Path(__file__).parent.parent))

from agent.ipc import decode, encode  # noqa: E402


def main() -> None:
    for line in sys.stdin:
        line = line.rstrip("\n")
        if not line:
            continue
        try:
            env = decode(line)
        except (ValueError, json.JSONDecodeError):
            continue
        sys.stdout.write(encode(env) + "\n")
        sys.stdout.flush()


if __name__ == "__main__":
    main()
