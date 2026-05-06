"""Herramientas Storage del Agente (proxies IPC).

Este módulo expondrá funciones decoradas como herramientas de Strands
(``@tool``) que delegan cada operación del Almacén a la TUI vía
``StdioClient``. El esqueleto se cablea en las tareas 9.1–9.5 del plan.
"""

from __future__ import annotations
