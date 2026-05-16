"""Sync daemon entry point.

Spawned by the TUI as a subprocess. Reads commands from stdin (JSON lines),
pushes pending events to Google Calendar, writes status to stdout.

Commands:
  {"command": "sync"}                  — trigger immediate push
  {"command": "delete", "google_event_id": "..."}  — delete from Google
  {"command": "stop"}                  — graceful shutdown
"""

from __future__ import annotations

import argparse
import json
import sys
import threading
import time
from pathlib import Path
from typing import Any


def _load_credentials(token_path: Path) -> Any:
    from google.oauth2.credentials import Credentials  # type: ignore[import]

    creds = Credentials.from_authorized_user_file(
        str(token_path),
        scopes=["https://www.googleapis.com/auth/calendar"],
    )
    if creds.expired and creds.refresh_token:
        from google.auth.transport.requests import Request  # type: ignore[import]

        creds.refresh(Request())
        token_path.write_text(creds.to_json())

    return creds


def _build_service(creds: Any) -> Any:
    from googleapiclient.discovery import build  # type: ignore[import]

    return build("calendar", "v3", credentials=creds)


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
        service = _build_service(creds)
    except Exception as e:
        _write_status("error", message=str(e))
        return

    from gcal_sync.db import connect
    from gcal_sync.sync import delete_from_google, push_pending

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
                do_sync(service, conn, args.calendar_id, args.timezone)
            elif command == "delete":
                gid = cmd.get("google_event_id")
                if gid:
                    delete_from_google(service, args.calendar_id, gid)
        stop_event.set()

    def do_sync(
        svc: Any, db_conn: Any, calendar_id: str, timezone: str
    ) -> None:
        _write_status("syncing")
        try:
            pushed = push_pending(svc, db_conn, calendar_id, timezone)
            _write_status("done", pushed=pushed)
        except Exception as e:
            _write_status("error", message=str(e))

    reader_thread = threading.Thread(target=stdin_reader, daemon=True)
    reader_thread.start()

    # Initial sync
    do_sync(service, conn, args.calendar_id, args.timezone)

    stop_event.wait()
    conn.close()


if __name__ == "__main__":
    main()
