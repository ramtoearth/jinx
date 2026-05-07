"""Paquete del Agente del Organizador de Día en Terminal.

Este paquete implementa el proceso hijo Python que cooperará con la TUI en
Rust a través de un Canal_IPC JSONL sobre stdio. Los submódulos se cablean en
tareas posteriores del plan de implementación:

- ``agent.ipc``: envelope TypedDict y cliente ``StdioClient``.
- ``agent.storage_tools``: herramientas ``@tool`` proxy hacia el Almacén.
- ``agent.main``: cableado del ``Agent`` de Strands y bucle de turno.
"""

from __future__ import annotations

__all__: list[str] = []
