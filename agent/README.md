# agent

Paquete Python del Agente del Organizador de Día en Terminal. Se implementa
sobre [Strands Agents](https://strandsagents.com/) y se comunica con la TUI
(Rust) por un Canal_IPC JSONL sobre stdio.

## Instalación (desarrollo)

```bash
pip install -e "./agent[dev]"
```

## Estructura

- `ipc.py`: envelope y `StdioClient`.
- `storage_tools.py`: herramientas `@tool` proxy hacia el Almacén.
- `inference.py`: inferencia determinista de Grupo (Jaccard de trigramas).
- `main.py`: bucle de turno del Agente.
- `tests/`: pruebas unitarias y property-based (`pytest`, `hypothesis`).
