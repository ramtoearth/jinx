"""Agent main loop — wires Strands Agent with storage tools and runs the
turn loop over the Canal_IPC.

Tasks 7.3–7.5 (Requisitos 5.1, 5.2, 5.3, 9.3, 9.4, 12.3).
"""

from __future__ import annotations

import os
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
from strands_tools import current_time, editor, file_read, file_write, think  # type: ignore[import]

_BUILTIN_TOOLS = [current_time, file_read, file_write, editor, think]

# ---------------------------------------------------------------------------
# System prompt template
# ---------------------------------------------------------------------------

_SYSTEM_PROMPT = """\
Eres el Agente del Organizador de Día en Terminal.
Ayudas al usuario a gestionar sus Tareas, Eventos y Grupos usando herramientas.

Reglas importantes:
- NOW = la fecha/hora ISO 8601 indicada al inicio del mensaje del usuario (línea "[NOW: ...]").
  Usa ese valor como referencia para cualquier expresión temporal relativa.
- Cuando el usuario mencione fechas relativas ("hoy", "mañana", "en dos horas", "el viernes"),
  convierte siempre a una fecha/hora absoluta en ISO 8601 antes de llamar a cualquier herramienta.
- Si no puedes resolver de forma unívoca una referencia temporal, pide aclaración antes de persistir.
- Sólo llamas a herramientas de almacenamiento con valores absolutos, nunca con referencias relativas.
- Grupos al crear Tarea o Evento:
    * Llama a ``list_groups`` para ver los grupos disponibles y elige el más apropiado por contexto.
    * Si el usuario menciona un nombre de grupo, busca su id en la lista y úsalo.
    * Si ningún grupo encaja claramente, usa group_id null.
    * NUNCA rechaces crear una Tarea o Evento por no poder determinar el Grupo.
- Responde siempre en el idioma del usuario.
"""

# ---------------------------------------------------------------------------
# Agent setup
# ---------------------------------------------------------------------------

def _build_agent(
    model_provider: str = "local",
    ollama_model: str = "llama3.2:3b",
    ollama_host: str = "http://localhost:11434",
    bedrock_model_id: str | None = None,
) -> Any:
    """Construct the Strands Agent with all tools registered."""
    try:
        from strands import Agent
    except ImportError as exc:
        raise RuntimeError(
            f"strands-agents not installed: {exc}"
        ) from exc

    from strands.models.ollama import OllamaModel  # type: ignore[import]
    from agent import storage_tools as st

    if model_provider == "local":
        model: Any = OllamaModel(
            model_id=ollama_model,
            host=ollama_host,
            max_tokens=4096,
            temperature=0,        # deterministic — required for reliable tool calling
            options={
                "num_ctx": 8192,  # enough context for 22 tool schemas + conversation
            },
        )
    else:
        if bedrock_model_id:
            from strands.models import BedrockModel  # type: ignore[import]
            model = BedrockModel(model_id=bedrock_model_id)
        else:
            model = None  # Strands usa el modelo Bedrock por defecto

    tools = _BUILTIN_TOOLS + [
        st.list_tasks,
        st.create_task,
        st.update_task,
        st.complete_task,
        st.delete_task,
        st.list_events,
        st.create_event,
        st.update_event,
        st.delete_event,
        st.list_groups,
        st.create_group,
        st.rename_group,
        st.recolor_group,
        st.delete_group,
        st.export_markdown,
        st.export_sqlite,
    ]

    import json as _json

    def _stderr_callback(**kwargs: Any) -> None:
        # Streaming text → stderr (visible in /tmp/tui_agent.log)
        chunk = kwargs.get("data", "")
        if chunk:
            sys.stderr.write(chunk)
            sys.stderr.flush()
        # Tool call start → log name and arguments for debugging
        tool = kwargs.get("current_tool_use")
        if tool:
            name = tool.get("name", "?")
            inp = _json.dumps(tool.get("input", {}), ensure_ascii=False)
            sys.stderr.write(f"\n[tool→] {name}({inp})\n")
            sys.stderr.flush()

    agent = Agent(
        system_prompt=_SYSTEM_PROMPT,
        model=model,
        tools=tools,
        callback_handler=_stderr_callback,
    )
    return agent


# ---------------------------------------------------------------------------
# Main loop
# ---------------------------------------------------------------------------

def _get_current_time() -> str:
    """Get the current time in ISO 8601 format (UTC)."""
    import datetime
    return datetime.datetime.now(datetime.timezone.utc).strftime(
        "%Y-%m-%dT%H:%M:%S+00:00"
    )


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
    model_provider = "local"
    ollama_model = os.environ.get("OLLAMA_MODEL", "llama3.2:3b")
    ollama_host = os.environ.get("OLLAMA_HOST", "http://localhost:11434")
    bedrock_model_id: str | None = os.environ.get("BEDROCK_MODEL_ID")

    for env in client.incoming_lines():
        msg_type = env.get("type", "")
        if msg_type == "agent_init":
            payload = env.get("payload") or {}
            model_provider = payload.get("model_provider", model_provider)
            # Config-file values from TUI take precedence; env vars are fallback for
            # power users / CI running the agent directly without the TUI.
            ollama_model = payload.get("ollama_model") or ollama_model
            ollama_host = payload.get("ollama_host") or ollama_host
            bedrock_model_id = payload.get("bedrock_model_id") or bedrock_model_id
            break
        if msg_type == "shutdown":
            return

    # Send agent_init_ack
    provider_notice: str | None = None
    if model_provider == "local":
        provider_notice = f"Agente usando Ollama local ({ollama_model}). Sin envío de datos a la nube."
    elif model_provider == "remote":
        bedrock_name = bedrock_model_id or "default"
        provider_notice = (
            f"Agente usando Amazon Bedrock ({bedrock_name}). "
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

    agent = _build_agent(
        model_provider=model_provider,
        ollama_model=ollama_model,
        ollama_host=ollama_host,
        bedrock_model_id=bedrock_model_id,
    )

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

        # Prepend current time so the agent can resolve relative dates (Requisito 5.1).
        # Injecting NOW in the user message avoids mutating agent.system_prompt per-turn,
        # which is fragile and not recommended by the Strands API.
        now_iso = _get_current_time()
        contextualized = f"[NOW: {now_iso}]\n{user_text}"

        try:
            response = agent(contextualized)
            reply_text = str(response)  # AgentResult.__str__ returns accumulated assistant text
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
