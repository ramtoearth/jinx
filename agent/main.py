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

class ConfigError(Exception):
    """Raised when a required configuration value is missing."""


_ENV_VARS: dict[str, str] = {
    "openai": "OPENAI_API_KEY",
    "anthropic": "ANTHROPIC_API_KEY",
    "gemini": "GOOGLE_API_KEY",
    "llamaapi": "LLAMA_API_KEY",
}


def _require_env(backend: str) -> str:
    var = _ENV_VARS[backend]
    value = os.environ.get(var)
    if not value:
        raise ConfigError(var)
    return value


def _build_model(backend: str, model_id: str, host: str | None) -> Any:
    match backend:
        case "ollama":
            from strands.models.ollama import OllamaModel  # type: ignore[import]
            return OllamaModel(
                model_id=model_id,
                host=host or "http://localhost:11434",
                max_tokens=4096,
                temperature=0,
                options={"num_ctx": 8192},
            )
        case "bedrock":
            from strands.models import BedrockModel  # type: ignore[import]
            return BedrockModel(model_id=model_id) if model_id else BedrockModel()
        case "openai":
            api_key = _require_env("openai")
            from strands.models.openai import OpenAIModel  # type: ignore[import]
            return OpenAIModel(client_args={"api_key": api_key}, model_id=model_id)
        case "anthropic":
            api_key = _require_env("anthropic")
            from strands.models.anthropic import AnthropicModel  # type: ignore[import]
            return AnthropicModel(client_args={"api_key": api_key}, model_id=model_id)
        case "gemini":
            api_key = _require_env("gemini")
            from strands.models.gemini import GeminiModel  # type: ignore[import]
            return GeminiModel(client_args={"api_key": api_key}, model_id=model_id)
        case "llamaapi":
            api_key = _require_env("llamaapi")
            from strands.models.llamaapi import LlamaAPIModel  # type: ignore[import]
            return LlamaAPIModel(client_args={"api_key": api_key}, model_id=model_id)
        case _:
            raise ConfigError(f"unknown backend: {backend}")


def _build_agent(
    system_prompt: str,
    backend: str = "ollama",
    model_id: str = "llama3.2:3b",
    host: str | None = None,
) -> Any:
    """Construct the Strands Agent with all tools registered."""
    try:
        from strands import Agent
    except ImportError as exc:
        raise RuntimeError(
            f"strands-agents not installed: {exc}"
        ) from exc

    from agent import storage_tools as st

    model = _build_model(backend, model_id, host)

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
        st.sync_google,
        st.list_notes,
        st.search_notes,
        st.create_note,
        st.update_note,
        st.delete_note,
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
    language = "en"
    backend = "ollama"
    raw_model_id = ""
    host: str | None = None

    for env in client.incoming_lines():
        msg_type = env.get("type", "")
        if msg_type == "agent_init":
            payload = env.get("payload") or {}
            language = payload.get("language", language)
            backend = payload.get("backend", backend)
            raw_model_id = payload.get("model_id", "")
            host = payload.get("host")
            break
        if msg_type == "shutdown":
            return

    _fallback_models: dict[str, str] = {
        "ollama": "llama3.2:3b",
        "openai": "gpt-4o",
        "anthropic": "claude-sonnet-4-6",
        "gemini": "gemini-2.5-flash-lite",
        "llamaapi": "Llama-4-Maverick-17B-128E-Instruct-FP8",
    }
    model_id = raw_model_id.strip() or _fallback_models.get(backend, "")

    # Load locale
    loc = load_locale(language)
    agent_loc = loc.get("agent", {})

    # Build agent and send ack (or error notice)
    provider_notice: str | None = None
    agent: Any = None
    try:
        if backend == "ollama":
            template = agent_loc.get("provider_notice_local", "Using {model}")
            provider_notice = template.replace("{model}", model_id)
        else:
            template_key = f"provider_notice_{backend}"
            template = agent_loc.get(template_key, agent_loc.get("provider_notice_remote", "Using {model}"))
            provider_notice = template.replace("{model}", model_id or "default")

        system_prompt = agent_loc.get("system_prompt", "")
        agent = _build_agent(
            system_prompt=system_prompt,
            backend=backend,
            model_id=model_id,
            host=host,
        )
    except ConfigError as e:
        error_template = agent_loc.get("error_missing_api_key", "{env_var} is not set.")
        provider_notice = error_template.replace("{env_var}", str(e))

    ack: Envelope = {
        "v": PROTOCOL_VERSION,
        "id": str(uuid.uuid4()),
        "kind": "response",
        "type": "agent_init_ack",
        "payload": {"provider_notice": provider_notice},
    }
    client.write_envelope(ack)

    if agent is None:
        return

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
