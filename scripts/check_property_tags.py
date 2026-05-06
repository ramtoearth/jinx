#!/usr/bin/env python3
"""Validate that every property-based test carries the required header annotation.

For Rust tests (proptest):
  // Feature: terminal-day-organizer, Property <N>: <text>

For Python tests (hypothesis):
  # Feature: terminal-day-organizer, Property <N>: <text>

Also checks that every referenced Property N exists in design.md.

Exit code: 0 if all checks pass, 1 otherwise.
"""

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
DESIGN_MD = ROOT / ".kiro" / "specs" / "terminal-day-organizer" / "design.md"

HEADER_RE = re.compile(
    r"Feature:\s*terminal-day-organizer,\s*Property\s+(\d+):\s*(.+)"
)

# Patterns that indicate a property test block is coming.
PROPTEST_RUST_RE = re.compile(r"proptest!\s*\{")
HYPOTHESIS_PY_RE = re.compile(r"@(given|settings)")


def load_design_properties(path: Path) -> dict[int, str]:
    """Return {N: description} for each '### Property N: description' in design.md."""
    props: dict[int, str] = {}
    for line in path.read_text(encoding="utf-8").splitlines():
        m = re.match(r"###\s+Property\s+(\d+):\s*(.+)", line)
        if m:
            props[int(m.group(1))] = m.group(2).strip()
    return props


def check_rust_file(path: Path, design_props: dict[int, str]) -> list[str]:
    errors: list[str] = []
    text = path.read_text(encoding="utf-8")
    lines = text.splitlines()

    # Find every proptest! { block and look for the header annotation above it.
    for i, line in enumerate(lines):
        if PROPTEST_RUST_RE.search(line):
            # Search up to 20 lines above for the Feature header
            header_line = None
            for j in range(max(0, i - 20), i):
                m = HEADER_RE.search(lines[j])
                if m:
                    header_line = (j + 1, int(m.group(1)), m.group(2).strip())
                    break
            if header_line is None:
                errors.append(
                    f"{path}:{i+1}: proptest! block missing "
                    f"'Feature: terminal-day-organizer, Property N:' header"
                )
            else:
                _, prop_n, _ = header_line
                if prop_n not in design_props:
                    errors.append(
                        f"{path}:{i+1}: Property {prop_n} referenced in header "
                        f"but not found in design.md"
                    )
    return errors


def check_python_file(path: Path, design_props: dict[int, str]) -> list[str]:
    errors: list[str] = []
    text = path.read_text(encoding="utf-8")
    lines = text.splitlines()

    for i, line in enumerate(lines):
        if HYPOTHESIS_PY_RE.search(line):
            # Search up to 20 lines above for the Feature header (in a comment)
            header_line = None
            for j in range(max(0, i - 20), i):
                m = HEADER_RE.search(lines[j])
                if m:
                    header_line = (j + 1, int(m.group(1)), m.group(2).strip())
                    break
            if header_line is None:
                errors.append(
                    f"{path}:{i+1}: @given/@settings decorator missing "
                    f"'# Feature: terminal-day-organizer, Property N:' header"
                )
            else:
                _, prop_n, _ = header_line
                if prop_n not in design_props:
                    errors.append(
                        f"{path}:{i+1}: Property {prop_n} referenced in header "
                        f"but not found in design.md"
                    )
    return errors


def main() -> int:
    if not DESIGN_MD.exists():
        print(f"ERROR: design.md not found at {DESIGN_MD}", file=sys.stderr)
        return 1

    design_props = load_design_properties(DESIGN_MD)
    if not design_props:
        print("ERROR: no properties found in design.md", file=sys.stderr)
        return 1

    print(f"Loaded {len(design_props)} properties from design.md")

    all_errors: list[str] = []

    # Check Rust test files
    for rs_file in ROOT.rglob("*.rs"):
        if "target" in rs_file.parts:
            continue
        if "tests" in rs_file.parts or rs_file.name.endswith("_test.rs"):
            errors = check_rust_file(rs_file, design_props)
            all_errors.extend(errors)

    # Also check src/ files that contain inline #[cfg(test)] with proptest
    for rs_file in ROOT.rglob("*.rs"):
        if "target" in rs_file.parts:
            continue
        if "tests" not in rs_file.parts:
            text = rs_file.read_text(encoding="utf-8")
            if "proptest!" in text:
                errors = check_rust_file(rs_file, design_props)
                all_errors.extend(errors)

    # Check Python test files
    for py_file in ROOT.rglob("test_*.py"):
        if "__pycache__" in py_file.parts:
            continue
        errors = check_python_file(py_file, design_props)
        all_errors.extend(errors)

    if all_errors:
        print(f"\nFound {len(all_errors)} error(s):")
        for err in all_errors:
            print(f"  {err}")
        return 1

    print("All property-based tests carry valid Feature headers.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
