from __future__ import annotations

import json
import logging
import os
import re
from typing import TYPE_CHECKING, Any, Optional

from attractor.launch_context import normalize_launch_context
from spark.workspace.conversations.utils import as_non_empty_string

if TYPE_CHECKING:
    from spark.workspace.conversations.models import ConversationTurn


LOGGER = logging.getLogger(__name__)


def normalize_flow_run_request_payload(
    arguments: Any,
    *,
    source_name: str,
) -> dict[str, Any]:
    if not isinstance(arguments, dict):
        raise ValueError(f"{source_name} requires an object argument payload.")
    flow_name = as_non_empty_string(arguments.get("flow_name"))
    summary = as_non_empty_string(arguments.get("summary"))
    if not flow_name:
        raise ValueError(f"{source_name} requires a non-empty flow_name.")
    if not summary:
        raise ValueError(f"{source_name} requires a non-empty summary.")
    payload: dict[str, Any] = {
        "flow_name": flow_name,
        "summary": summary,
    }
    goal = as_non_empty_string(arguments.get("goal"))
    if goal:
        payload["goal"] = goal
    launch_context = normalize_launch_context(
        arguments.get("launch_context"),
        source_name=source_name,
    )
    if launch_context:
        payload["launch_context"] = launch_context
    model = as_non_empty_string(arguments.get("model"))
    if model:
        payload["model"] = model
    llm_provider = as_non_empty_string(arguments.get("llm_provider"))
    if llm_provider:
        payload["llm_provider"] = llm_provider.lower()
    llm_profile = as_non_empty_string(arguments.get("llm_profile"))
    if llm_profile:
        payload["llm_profile"] = llm_profile
    reasoning_effort = as_non_empty_string(arguments.get("reasoning_effort"))
    if reasoning_effort:
        payload["reasoning_effort"] = reasoning_effort.lower()
    execution_container_image = as_non_empty_string(arguments.get("execution_container_image"))
    if execution_container_image:
        payload["execution_container_image"] = execution_container_image
    return payload


def extract_json_object(raw: str) -> dict[str, Any]:
    text = raw.strip()
    if not text:
        raise ValueError("Expected non-empty JSON response from Codex.")
    fenced = re.search(r"```(?:json)?\s*(\{.*\})\s*```", text, flags=re.DOTALL)
    candidate = fenced.group(1) if fenced else text
    if not candidate.lstrip().startswith("{"):
        start = candidate.find("{")
        end = candidate.rfind("}")
        if start >= 0 and end > start:
            candidate = candidate[start : end + 1]
    parsed = json.loads(candidate)
    if not isinstance(parsed, dict):
        raise ValueError("Expected top-level JSON object from Codex.")
    return parsed


def parse_chat_response_payload(raw: str) -> tuple[str, Optional[dict[str, Any]]]:
    text = raw.strip()
    if not text:
        return "", None
    try:
        parsed = extract_json_object(text)
    except Exception:
        return text, None
    assistant_message = as_non_empty_string(parsed.get("assistant_message"))
    if assistant_message:
        return assistant_message, parsed if isinstance(parsed, dict) else None
    fallback_text = text if not text.startswith("{") else ""
    return fallback_text, parsed if isinstance(parsed, dict) else None


def is_project_chat_debug_enabled() -> bool:
    value = str(os.environ.get("SPARK_DEBUG_PROJECT_CHAT", "")).strip().lower()
    return value in {"1", "true", "yes", "on"}


def summarize_turns_for_debug(turns: list["ConversationTurn"]) -> list[dict[str, Any]]:
    return [
        {
            "id": turn.id,
            "role": turn.role,
            "kind": turn.kind,
            "artifact_id": turn.artifact_id,
            "content": turn.content[:160],
        }
        for turn in turns
    ]


def log_project_chat_debug(message: str, **fields: Any) -> None:
    if not is_project_chat_debug_enabled():
        return
    if fields:
        LOGGER.info("[project-chat] %s | %s", message, json.dumps(fields, sort_keys=True, default=str))
        return
    LOGGER.info("[project-chat] %s", message)
