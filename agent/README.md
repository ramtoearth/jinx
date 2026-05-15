# agent

Python agent package. Built on [Strands Agents](https://strandsagents.com) and
[strands-agents-tools](https://github.com/strands-agents/tools). Communicates with the Rust TUI
over a JSON Lines IPC channel on stdio.

## Development setup

```bash
pip install -e "./agent[dev]"
pytest agent/tests/
```

## Modules

- `ipc.py` — envelope `TypedDict` and `StdioClient`
- `storage_tools.py` — `@tool` functions that proxy to the SQLite storage
- `locale.py` — locale loader (TOML-based i18n)
- `main.py` — Strands Agent construction and turn loop
