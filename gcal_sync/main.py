"""Sync daemon entry point.

Spawned by the TUI as a subprocess. Reads commands from stdin (JSON lines),
pushes pending events to Google Calendar and tasks to Google Tasks,
writes status to stdout.

Commands:
  {"command": "sync"}                                          — trigger immediate push
  {"command": "delete", "google_event_id": "...", "kind": "event"}  — delete event from Calendar
  {"command": "delete", "google_event_id": "...", "kind": "task"}   — delete task from Google Tasks
  {"command": "stop"}                                          — graceful shutdown
"""

from __future__ import annotations

import argparse
import json
import sys
import threading
from pathlib import Path
from typing import Any

SCOPES = [
    "https://www.googleapis.com/auth/calendar",
    "https://www.googleapis.com/auth/tasks",
]


def _load_credentials(token_path: Path) -> Any:
    from google.oauth2.credentials import Credentials  # type: ignore[import]

    creds = Credentials.from_authorized_user_file(str(token_path), scopes=SCOPES)
    if creds.expired and creds.refresh_token:
        from google.auth.transport.requests import Request  # type: ignore[import]

        creds.refresh(Request())
        token_path.write_text(creds.to_json())

    return creds


def _build_services(creds: Any) -> tuple[Any, Any]:
    from googleapiclient.discovery import build  # type: ignore[import]

    calendar_service = build("calendar", "v3", credentials=creds)
    tasks_service = build("tasks", "v1", credentials=creds)
    return calendar_service, tasks_service


def _write_status(state: str, **extra: Any) -> None:
    msg: dict[str, Any] = {"type": "sync_status", "state": state}
    msg.update(extra)
    try:
        sys.stdout.write(json.dumps(msg) + "\n")
        sys.stdout.flush()
    except BrokenPipeError:
        pass


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--db", required=True)
    parser.add_argument("--calendar-id", default="primary")
    parser.add_argument("--token-path", required=True)
    parser.add_argument("--timezone", default="UTC")
    args = parser.parse_args()

    token_path = Path(args.token_path)
    if not token_path.exists():
        _write_status("error", message="Token file not found")
        return

    try:
        creds = _load_credentials(token_path)
        calendar_service, tasks_service = _build_services(creds)
    except Exception as e:
        _write_status("error", message=str(e))
        return

    from gcal_sync.db import connect
    from gcal_sync.sync import delete_event_from_google, delete_task_from_google, push_pending

    conn = connect(args.db)
    _write_status("idle")

    stop_event = threading.Event()

    def stdin_reader() -> None:
        for line in sys.stdin:
            line = line.strip()
            if not line:
                continue
            try:
                cmd = json.loads(line)
            except json.JSONDecodeError:
                continue

            command = cmd.get("command", "")
            if command == "stop":
                stop_event.set()
                return
            elif command == "sync":
                do_sync()
            elif command == "delete":
                gid = cmd.get("google_event_id")
                kind = cmd.get("kind", "event")
                if gid:
                    if kind == "task":
                        delete_task_from_google(tasks_service, gid)
                    else:
                        delete_event_from_google(calendar_service, args.calendar_id, gid)
        stop_event.set()

    def do_sync() -> None:
        _write_status("syncing")
        try:
            pushed = push_pending(
                calendar_service, tasks_service, conn, args.calendar_id, args.timezone
            )
            _write_status("done", pushed=pushed)
        except Exception as e:
            _write_status("error", message=str(e))

    reader_thread = threading.Thread(target=stdin_reader, daemon=True)
    reader_thread.start()

    do_sync()

    stop_event.wait()
    conn.close()


if __name__ == "__main__":
    main()
