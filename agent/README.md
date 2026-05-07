# agent

Paquete Python del agente. Construido sobre [Strands Agents](https://strandsagents.com) y
[strands-agents-tools](https://github.com/strands-agents/tools). Se comunica con la TUI Rust
por un canal IPC de líneas JSON sobre stdio.

## Instalación de desarrollo

```bash
pip install -e "./agent[dev]"
pytest agent/tests/
```

## Módulos

- `ipc.py` — envelope `TypedDict` y `StdioClient`
- `storage_tools.py` — herramientas `@tool` que proxean al almacén SQLite
- `main.py` — construcción del agente Strands y bucle de turno
