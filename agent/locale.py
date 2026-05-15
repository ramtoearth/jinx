"""Locale loader for the jinx agent.

Reads a TOML locale file from the locales/ directory next to this module.
Falls back to English if the requested language file is not found.
"""

from __future__ import annotations

import tomllib
from pathlib import Path
from typing import Any, Dict


def load(lang: str) -> Dict[str, Any]:
    locale_dir = Path(__file__).parent / "locales"
    path = locale_dir / f"{lang}.toml"
    if not path.exists():
        path = locale_dir / "en.toml"
    with open(path, "rb") as f:
        return tomllib.load(f)
