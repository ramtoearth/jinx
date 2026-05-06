"""Agent main loop — wires Strands Agent with storage tools and runs the
turn loop over the Canal_IPC.

Tasks 7.3–7.5 (Requisitos 5.1, 5.2, 5.3, 9.3, 9.4, 12.3).
"""

from __future__ import annotations

import sys
import uuid
from typing import Any

from agent.ipc import (
    PROTOCOL_VERSION,
    Envelope,
    StdioClient,
    StorageError,
)
from agent.storage_tools import set_client

# ---------------------------------------------------------------------------
# System prompt template
# ---------------------------------------------------------------------------

_SYSTEM_PROMPT = """\
Eres el Agente del Organizador de Día en Terminal.
Ayudas al usuario a gestionar sus Tareas, Eventos y Grupos usando herramientas.

Reglas importantes:
- NOW = {now}  ← usa esta fecha/hora como referencia para cualquier expresión temporal relativa.
- Cuando el usuario mencione fechas relativas ("hoy", "mañana", "en dos horas", "el viernes"),
  convierte siempre a una fecha/hora absoluta en ISO 8601 antes de llamar a cualquier herramienta.
- Si no puedes resolver de forma unívoca una referencia temporal, pide aclaración antes de persistir.
- Sólo llamas a herramientas de almacenamiento con valores absolutos, nunca con referencias relativas.
- Si el usuario no menciona un Grupo al crear una Tarea o Evento, usa la herramienta
  ``infer_group_candidate`` para detectar el Grupo más adecuado según las reglas de umbral.
- Responde siempre en el idioma del usuario.
"""

# ---------------------------------------------------------------------------
# Agent setup
# ---------------------------------------------------------------------------

def _build_agent(now_iso: str) -> Any:
    """Construct the Strands Agent with all tools registered."""
    try:
        from strands import Agent
        from strands_tools import current_time, file_read, file_write, editor  # type: ignore[import]
    except ImportError as exc:
        raise RuntimeError(
            f"strands-agents or strands-tools not installed: {exc}"
        ) from exc

    from agent import storage_tools as st
    from agent.inference import infer_group_candidate

    # Expose storage functions as tools by wrapping them
    def _wrap(fn: Any) -> Any:
        """Minimal Strands-compatible tool wrapper."""
        return fn

    tools = [
        current_time,
        file_read,
        file_write,
        editor,
        _wrap(st.list_tasks),
        _wrap(st.create_task),
        _wrap(st.update_task),
        _wrap(st.complete_task),
        _wrap(st.delete_task),
        _wrap(st.list_events),
        _wrap(st.create_event),
        _wrap(st.update_event),
        _wrap(st.delete_event),
        _wrap(st.list_groups),
        _wrap(st.create_group),
        _wrap(st.rename_group),
        _wrap(st.recolor_group),
        _wrap(st.delete_group),
        _wrap(st.export_markdown),
        _wrap(st.export_sqlite),
        infer_group_candidate,
    ]

    agent = Agent(
        system_prompt=_SYSTEM_PROMPT.format(now=now_iso),
        tools=tools,
    )
    return agent


# ---------------------------------------------------------------------------
# Main loop
# ---------------------------------------------------------------------------

def _get_current_time(timezone: str = "UTC") -> str:
    """Get the current time in ISO 8601 format."""
    try:
        from strands_tools import current_time  # type: ignore[import]
        return str(current_time(timezone=timezone))
    except Exception:
        import datetime
        return datetime.datetime.utcnow().strftime("%Y-%m-%dT%H:%M:%S+00:00")


def main(
    stdin: Any = None,
    stdout: Any = None,
) -> None:
    """Entry point for the Agent process.

    Reads ``agent_init`` from the TUI, sends ``agent_init_ack``, then enters
    the turn loop: receives ``user_message``, calls the Strands Agent, emits
    ``agent_reply``.
    """
    client = StdioClient(stdin=stdin or sys.stdin, stdout=stdout or sys.stdout)
    set_client(client)

    # Wait for agent_init
    timezone = "UTC"
    model_provider = "local"
    for env in client.incoming_lines():
        msg_type = env.get("type", "")
        if msg_type == "agent_init":
            payload = env.get("payload") or {}
            timezone = payload.get("timezone", "UTC")
            model_provider = payload.get("model_provider", "local")
            break
        if msg_type == "shutdown":
            return

    # Send agent_init_ack
    provider_notice = None
    if model_provider == "remote":
        provider_notice = (
            "Aviso: el Agente está usando un proveedor de modelo remoto. "
            "Los mensajes del chat se envían a un servicio externo."
        )

    ack: Envelope = {
        "v": PROTOCOL_VERSION,
        "id": str(uuid.uuid4()),
        "kind": "response",
        "type": "agent_init_ack",
        "payload": {"provider_notice": provider_notice},
    }
    client.write_envelope(ack)

    # Build the agent with the current time
    now_iso = _get_current_time(timezone)
    agent = _build_agent(now_iso)

    # Turn loop
    for env in client.incoming_lines():
        msg_type = env.get("type", "")
        req_id = env.get("id")

        if msg_type == "shutdown":
            break

        if msg_type != "user_message":
            continue

        payload = env.get("payload") or {}
        user_text = payload.get("text", "")

        # Refresh "now" for each turn (Requisito 5.1)
        now_iso = _get_current_time(timezone)
        # Update the system prompt with the current time
        agent.system_prompt = _SYSTEM_PROMPT.format(now=now_iso)

        try:
            response = agent(user_text)
            reply_text = str(response)
        except StorageError as e:
            reply_text = f"No he podido completar la operación: {e.message}"
        except Exception as e:
            reply_text = f"Ha ocurrido un error inesperado: {e}"

        reply: Envelope = {
            "v": PROTOCOL_VERSION,
            "id": str(uuid.uuid4()),
            "kind": "response",
            "type": "agent_reply",
            "ref": req_id,
            "payload": {"text": reply_text},
        }
        client.write_envelope(reply)


if __name__ == "__main__":  # pragma: no cover
    main()
