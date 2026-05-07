"""Paquete del agente de jinx.

Implementa el proceso hijo Python que coopera con la TUI Rust a través de un
Canal_IPC JSONL sobre stdio.

- ``agent.ipc``: envelope TypedDict y cliente ``StdioClient``.
- ``agent.storage_tools``: herramientas ``@tool`` proxy hacia el Almacén.
- ``agent.main``: cableado del ``Agent`` de Strands y bucle de turno.
"""

from __future__ import annotations

__all__: list[str] = []
