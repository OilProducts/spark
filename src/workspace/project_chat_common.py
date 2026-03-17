from __future__ import annotations

import json
import logging
import os
import re
import time
from typing import Any, Optional

from sparkspawn_common.runtime import (
    build_codex_runtime_environment as _build_codex_runtime_environment,
    normalize_project_path,
    resolve_runtime_workspace_path as _resolve_runtime_workspace_path,
)
from workspace.project_chat_models import ConversationTurn


LOGGER = logging.getLogger(__name__)


def iso_now() -> str:
    return time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())


def slugify(value: str) -> str:
    slug = re.sub(r"[^a-z0-9]+", "-", value.lower()).strip("-")
    return slug or "project"


def truncate_text(value: str, limit: int) -> str:
    trimmed = value.strip()
    if len(trimmed) <= limit:
        return trimmed
    return trimmed[: max(0, limit - 1)].rstrip() + "…"


def normalize_project_path_value(value: str) -> str:
    return normalize_project_path(value)


def resolve_runtime_workspace_path(value: str) -> str:
    return _resolve_runtime_workspace_path(value)


def build_codex_runtime_environment() -> dict[str, str]:
    return _build_codex_runtime_environment()


def as_non_empty_string(value: Any) -> Optional[str]:
    if value is None:
        return None
    if isinstance(value, str):
        trimmed = value.strip()
        return trimmed or None
    if isinstance(value, (int, float, bool)):
        return str(value)
    return None


def normalize_spec_edit_proposal_payload(
    arguments: Any,
    *,
    source_name: str,
) -> dict[str, Any]:
    if not isinstance(arguments, dict):
        raise ValueError(f"{source_name} requires an object argument payload.")
    summary = as_non_empty_string(arguments.get("summary"))
    raw_changes = arguments.get("changes")
    if not summary:
        raise ValueError(f"{source_name} requires a non-empty summary.")
    if not isinstance(raw_changes, list):
        raise ValueError(f"{source_name} requires a changes array.")
    changes: list[dict[str, str]] = []
    for raw_change in raw_changes:
        if not isinstance(raw_change, dict):
            continue
        path = as_non_empty_string(raw_change.get("path"))
        before = as_non_empty_string(raw_change.get("before"))
        after = as_non_empty_string(raw_change.get("after"))
        if not path or before is None or after is None:
            continue
        changes.append({"path": path, "before": before, "after": after})
    if not changes:
        raise ValueError(f"{source_name} requires at least one valid change.")
    payload: dict[str, Any] = {
        "summary": summary,
        "changes": changes,
    }
    rationale = as_non_empty_string(arguments.get("rationale"))
    if rationale:
        payload["rationale"] = rationale
    return payload


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
    model = as_non_empty_string(arguments.get("model"))
    if model:
        payload["model"] = model
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


def derive_conversation_title(turns: list[ConversationTurn]) -> str:
    for turn in turns:
        if turn.role != "user":
            continue
        title = as_non_empty_string(turn.content)
        if title:
            return truncate_text(title, 64)
    return "New thread"


def build_conversation_preview(turns: list[ConversationTurn]) -> Optional[str]:
    for turn in reversed(turns):
        if turn.kind != "message":
            continue
        preview = as_non_empty_string(turn.content)
        if preview:
            return truncate_text(preview, 96)
    return None


def is_project_chat_debug_enabled() -> bool:
    value = str(os.environ.get("SPARKSPAWN_DEBUG_PROJECT_CHAT", "")).strip().lower()
    return value in {"1", "true", "yes", "on"}


def summarize_turns_for_debug(turns: list[ConversationTurn]) -> list[dict[str, Any]]:
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
