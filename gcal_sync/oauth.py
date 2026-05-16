"""One-shot OAuth flow for Google Calendar.

Spawned as a subprocess by the TUI. Opens the browser for authorization,
receives the callback on a local HTTP server, stores tokens, then exits.

Prints a single JSON line to stdout: {"status": "ok"} or {"status": "error", "message": "..."}.
"""

from __future__ import annotations

import json
import os
import stat
import sys
from pathlib import Path


def _config_dir() -> Path:
    """Resolve the jinx config directory (same logic as Rust side)."""
    if sys.platform == "darwin":
        return Path.home() / "Library" / "Application Support" / "jinx"
    elif sys.platform == "win32":
        appdata = os.environ.get("APPDATA", Path.home() / "AppData" / "Roaming")
        return Path(appdata) / "jinx"
    else:
        xdg = os.environ.get("XDG_CONFIG_HOME", Path.home() / ".config")
        return Path(xdg) / "jinx"


def _credentials_path() -> Path:
    """Path to the shipped OAuth client credentials."""
    return Path(__file__).parent / "credentials.json"


def run(token_path: str | None = None) -> None:
    try:
        from google_auth_oauthlib.flow import InstalledAppFlow  # type: ignore[import]
    except ImportError:
        _reply_error("google-auth-oauthlib not installed. Run: uv pip install google-auth-oauthlib")
        return

    creds_file = _credentials_path()
    if not creds_file.exists():
        _reply_error(f"OAuth credentials not found at {creds_file}")
        return

    scopes = [
        "https://www.googleapis.com/auth/calendar",
        "https://www.googleapis.com/auth/tasks",
    ]

    try:
        flow = InstalledAppFlow.from_client_secrets_file(str(creds_file), scopes)
        creds = flow.run_local_server(port=0, open_browser=True)
    except Exception as e:
        _reply_error(str(e))
        return

    out = Path(token_path) if token_path else _config_dir() / "google_token.json"
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(creds.to_json())

    try:
        os.chmod(out, stat.S_IRUSR | stat.S_IWUSR)
    except OSError:
        pass

    _reply_ok()


def _reply_ok() -> None:
    print(json.dumps({"status": "ok"}), flush=True)


def _reply_error(message: str) -> None:
    print(json.dumps({"status": "error", "message": message}), flush=True)


if __name__ == "__main__":
    import argparse

    parser = argparse.ArgumentParser()
    parser.add_argument("--token-path", default=None)
    args = parser.parse_args()
    run(token_path=args.token_path)
