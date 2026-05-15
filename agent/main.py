"""Agent main loop — wires Strands Agent with storage tools and runs the
turn loop over the Canal_IPC.
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
from agent.locale import load as load_locale
from agent.storage_tools import set_client
from strands_tools import current_time, editor, file_read, file_write, think  # type: ignore[import]

_BUILTIN_TOOLS = [current_time, file_read, file_write, editor, think]

# ---------------------------------------------------------------------------
# Agent setup
# ---------------------------------------------------------------------------

def _build_agent(
    system_prompt: str,
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
            temperature=0,
            options={
                "num_ctx": 8192,
            },
        )
    else:
        if bedrock_model_id:
            from strands.models import BedrockModel  # type: ignore[import]
            model = BedrockModel(model_id=bedrock_model_id)
        else:
            model = None

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
        st.find_group_by_name,
        st.create_group,
        st.rename_group,
        st.recolor_group,
        st.delete_group,
        st.export_markdown,
        st.export_sqlite,
    ]

    import json as _json

    def _stderr_callback(**kwargs: Any) -> None:
        chunk = kwargs.get("data", "")
        if chunk:
            sys.stderr.write(chunk)
            sys.stderr.flush()
        tool = kwargs.get("current_tool_use")
        if tool:
            name = tool.get("name", "?")
            inp = _json.dumps(tool.get("input", {}), ensure_ascii=False)
            sys.stderr.write(f"\n[tool→] {name}({inp})\n")
            sys.stderr.flush()

    agent = Agent(
        system_prompt=system_prompt,
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
    language = "en"
    ollama_model = os.environ.get("OLLAMA_MODEL", "llama3.2:3b")
    ollama_host = os.environ.get("OLLAMA_HOST", "http://localhost:11434")
    bedrock_model_id: str | None = os.environ.get("BEDROCK_MODEL_ID")

    for env in client.incoming_lines():
        msg_type = env.get("type", "")
        if msg_type == "agent_init":
            payload = env.get("payload") or {}
            model_provider = payload.get("model_provider", model_provider)
            language = payload.get("language", language)
            ollama_model = payload.get("ollama_model") or ollama_model
            ollama_host = payload.get("ollama_host") or ollama_host
            bedrock_model_id = payload.get("bedrock_model_id") or bedrock_model_id
            break
        if msg_type == "shutdown":
            return

    # Load locale
    loc = load_locale(language)
    agent_loc = loc.get("agent", {})

    # Send agent_init_ack
    provider_notice: str | None = None
    if model_provider == "local":
        template = agent_loc.get("provider_notice_local", "")
        provider_notice = template.replace("{model}", ollama_model)
    elif model_provider == "remote":
        bedrock_name = bedrock_model_id or "default"
        template = agent_loc.get("provider_notice_remote", "")
        provider_notice = template.replace("{model}", bedrock_name)

    ack: Envelope = {
        "v": PROTOCOL_VERSION,
        "id": str(uuid.uuid4()),
        "kind": "response",
        "type": "agent_init_ack",
        "payload": {"provider_notice": provider_notice},
    }
    client.write_envelope(ack)

    system_prompt = agent_loc.get("system_prompt", "")
    agent = _build_agent(
        system_prompt=system_prompt,
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
            reply_text = str(response)
        except StorageError as e:
            template = agent_loc.get("error_operation_failed", "Error: {error}")
            reply_text = template.replace("{error}", e.message)
        except Exception as e:
            template = agent_loc.get("error_unexpected", "Error: {error}")
            reply_text = template.replace("{error}", str(e))

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
