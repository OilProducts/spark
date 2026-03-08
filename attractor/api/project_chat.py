from __future__ import annotations

import asyncio
import json
import logging
import os
import re
import selectors
import shutil
import subprocess
import tempfile
import threading
import time
import uuid
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any, Callable, Optional


CHAT_RUNTIME_THREAD_KEY = "_attractor.runtime.thread_id"
RUNTIME_REPO_ROOT = Path(__file__).resolve().parents[2]
FINAL_ANSWER_IDLE_GRACE_SECONDS = 1.0
LIVE_ASSISTANT_IDLE_COMPLETION_GRACE_SECONDS = 2.5
CHAT_TURN_IDLE_TIMEOUT_SECONDS = 15.0
APP_SERVER_REQUEST_TIMEOUT_SECONDS = 15.0
LOGGER = logging.getLogger(__name__)


def _iso_now() -> str:
    return time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())


def _slugify(value: str) -> str:
    slug = re.sub(r"[^a-z0-9]+", "-", value.lower()).strip("-")
    return slug or "project"


def _truncate_text(value: str, limit: int) -> str:
    trimmed = value.strip()
    if len(trimmed) <= limit:
        return trimmed
    return trimmed[: max(0, limit - 1)].rstrip() + "…"


def _normalize_project_path(value: str) -> str:
    trimmed = value.strip()
    if not trimmed:
        return ""
    return str(Path(trimmed).expanduser().resolve(strict=False))


def _extract_json_object(raw: str) -> dict[str, Any]:
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


def _extract_live_assistant_message(raw: str) -> str:
    text = raw.strip()
    if not text:
        return ""
    try:
        parsed = _extract_json_object(text)
    except Exception:
        parsed = None
    if isinstance(parsed, dict):
        assistant_message = _as_non_empty_string(parsed.get("assistant_message"))
        return assistant_message or ""
    if not text.startswith("{"):
        return text

    key_match = re.search(r'"assistant_message"\s*:\s*"', text)
    if key_match is None:
        return ""

    chars: list[str] = []
    escape = False
    index = key_match.end()
    while index < len(text):
        char = text[index]
        if escape:
            if char == "u" and index + 4 < len(text):
                hex_digits = text[index + 1 : index + 5]
                if re.fullmatch(r"[0-9a-fA-F]{4}", hex_digits):
                    chars.append(chr(int(hex_digits, 16)))
                    index += 5
                    escape = False
                    continue
            replacements = {
                '"': '"',
                "\\": "\\",
                "/": "/",
                "b": "\b",
                "f": "\f",
                "n": "\n",
                "r": "\r",
                "t": "\t",
            }
            chars.append(replacements.get(char, char))
            escape = False
            index += 1
            continue
        if char == "\\":
            escape = True
            index += 1
            continue
        if char == '"':
            break
        chars.append(char)
        index += 1
    return "".join(chars).strip()


def _parse_chat_response_payload(raw: str) -> tuple[str, Optional[dict[str, Any]]]:
    text = raw.strip()
    if not text:
        return "", None
    try:
        parsed = _extract_json_object(text)
    except Exception:
        return text, None
    assistant_message = _as_non_empty_string(parsed.get("assistant_message"))
    if assistant_message:
        return assistant_message, parsed if isinstance(parsed, dict) else None
    fallback_text = text if not text.startswith("{") else ""
    return fallback_text, parsed if isinstance(parsed, dict) else None


def _should_draft_spec_proposal(message: str) -> bool:
    normalized = message.strip().lower()
    if not normalized:
        return False
    informational_starts = (
        "what is",
        "what's",
        "whats",
        "explain",
        "summarize",
        "list",
        "show",
        "tell me",
        "why is",
        "how does",
        "how do",
    )
    if normalized.startswith(informational_starts):
        return False
    proposal_signals = (
        "change",
        "update",
        "remove",
        "replace",
        "rename",
        "rework",
        "refactor",
        "clean up",
        "cleanup",
        "redesign",
        "adjust",
        "should",
        "need to",
        "let's",
        "lets",
        "implement",
        "add ",
        "move ",
        "spec",
        "workflow",
        "ux",
        "ui",
    )
    return any(signal in normalized for signal in proposal_signals)


def _as_non_empty_string(value: Any) -> Optional[str]:
    if value is None:
        return None
    if isinstance(value, str):
        trimmed = value.strip()
        return trimmed or None
    if isinstance(value, (int, float, bool)):
        return str(value)
    return None


def _derive_conversation_title(turns: list["ConversationTurn"]) -> str:
    for turn in turns:
        if turn.role != "user":
            continue
        title = _as_non_empty_string(turn.content)
        if title:
            return _truncate_text(title, 64)
    return "New thread"


def _build_conversation_preview(turns: list["ConversationTurn"]) -> Optional[str]:
    for turn in reversed(turns):
        if turn.kind != "message":
            continue
        preview = _as_non_empty_string(turn.content)
        if preview:
            return _truncate_text(preview, 96)
    return None


def _extract_command_text(payload: dict[str, Any]) -> Optional[str]:
    for key in ("command", "commandLine", "command_line", "cmd", "commandText"):
        value = payload.get(key)
        if isinstance(value, list):
            pieces = [_as_non_empty_string(entry) for entry in value]
            command = " ".join(piece for piece in pieces if piece)
            if command:
                return command
        text = _as_non_empty_string(value)
        if text:
            return text
    nested = payload.get("command")
    if isinstance(nested, dict):
        return _extract_command_text(nested)
    return None


def _extract_file_paths(payload: dict[str, Any]) -> list[str]:
    paths: list[str] = []
    for key in ("path", "filePath", "file_path"):
        text = _as_non_empty_string(payload.get(key))
        if text:
            paths.append(text)
    for key in ("paths", "files"):
        value = payload.get(key)
        if isinstance(value, list):
            for entry in value:
                if isinstance(entry, dict):
                    nested_path = _as_non_empty_string(entry.get("path") or entry.get("filePath") or entry.get("file_path"))
                    if nested_path:
                        paths.append(nested_path)
                        continue
                text = _as_non_empty_string(entry)
                if text:
                    paths.append(text)
    changes = payload.get("changes")
    if isinstance(changes, list):
        for change in changes:
            if not isinstance(change, dict):
                continue
            nested_path = _as_non_empty_string(change.get("path") or change.get("filePath") or change.get("file_path"))
            if nested_path:
                paths.append(nested_path)
    seen: set[str] = set()
    deduped: list[str] = []
    for path in paths:
        if path in seen:
            continue
        seen.add(path)
        deduped.append(path)
    return deduped


def _append_tool_output(existing: Optional[str], delta: str, *, limit: int = 2400) -> str:
    combined = f"{existing or ''}{delta}"
    if len(combined) <= limit:
        return combined
    return combined[-limit:]


def _is_project_chat_debug_enabled() -> bool:
    value = str(os.environ.get("SPARKSPAWN_DEBUG_PROJECT_CHAT", "")).strip().lower()
    return value in {"1", "true", "yes", "on"}


def _summarize_turns_for_debug(turns: list["ConversationTurn"]) -> list[dict[str, Any]]:
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


def _log_project_chat_debug(message: str, **fields: Any) -> None:
    if not _is_project_chat_debug_enabled():
        return
    if fields:
        LOGGER.info("[project-chat] %s | %s", message, json.dumps(fields, sort_keys=True, default=str))
        return
    LOGGER.info("[project-chat] %s", message)


def _normalize_tool_call_status(value: Any) -> str:
    normalized = str(value or "").strip().lower()
    if normalized in {"inprogress", "running"}:
        return "running"
    if normalized in {"failed", "error"}:
        return "failed"
    return "completed"


def _extract_file_paths_from_item(item: dict[str, Any]) -> list[str]:
    changes = item.get("changes")
    if not isinstance(changes, list):
        return []
    file_paths: list[str] = []
    for change in changes:
        if not isinstance(change, dict):
            continue
        path = _as_non_empty_string(change.get("path") or change.get("filePath") or change.get("file_path"))
        if path:
            file_paths.append(path)
    seen: set[str] = set()
    deduped: list[str] = []
    for path in file_paths:
        if path in seen:
            continue
        seen.add(path)
        deduped.append(path)
    return deduped


def _tool_call_from_item(item: dict[str, Any]) -> Optional[ToolCallRecord]:
    item_type = str(item.get("type") or "").strip()
    item_id = _as_non_empty_string(item.get("id")) or f"tool-{uuid.uuid4().hex}"
    if item_type == "commandExecution":
        command = _extract_command_text(item)
        raw_output = item.get("aggregatedOutput")
        if raw_output is None:
            raw_output = item.get("aggregated_output")
        output = str(raw_output) if raw_output is not None and str(raw_output) else None
        return ToolCallRecord(
            id=item_id,
            kind="command_execution",
            status=_normalize_tool_call_status(item.get("status")),
            title="Run command",
            command=command,
            output=output,
        )
    if item_type == "fileChange":
        return ToolCallRecord(
            id=item_id,
            kind="file_change",
            status=_normalize_tool_call_status(item.get("status")),
            title="Apply file changes",
            file_paths=_extract_file_paths_from_item(item),
        )
    return None


def resolve_runtime_workspace_path(value: str) -> str:
    normalized = _normalize_project_path(value)
    if not normalized:
        return ""

    requested_path = Path(normalized)
    if requested_path.exists():
        return str(requested_path)

    runtime_roots: list[Path] = []
    host_root_override = os.environ.get("ATTRACTOR_HOST_REPO_ROOT", "").strip()
    runtime_root_override = os.environ.get("ATTRACTOR_RUNTIME_REPO_ROOT", "").strip()
    host_root = Path(host_root_override).expanduser().resolve(strict=False) if host_root_override else None
    if runtime_root_override:
        runtime_roots.append(Path(runtime_root_override).expanduser().resolve(strict=False))
    runtime_roots.append(RUNTIME_REPO_ROOT)

    requested_parts = requested_path.parts
    for runtime_root in runtime_roots:
        if not runtime_root.exists():
            continue
        if host_root is not None:
            try:
                relative_to_host_root = requested_path.relative_to(host_root)
            except ValueError:
                relative_to_host_root = None
            if relative_to_host_root is not None:
                candidate = runtime_root / relative_to_host_root
                if candidate.exists():
                    return str(candidate.resolve(strict=False))
            host_root_matching_indexes = [index for index, part in enumerate(requested_parts) if part == host_root.name]
            for index in reversed(host_root_matching_indexes):
                candidate = runtime_root.joinpath(*requested_parts[index + 1 :])
                if candidate.exists():
                    return str(candidate.resolve(strict=False))
        matching_indexes = [index for index, part in enumerate(requested_parts) if part == runtime_root.name]
        for index in reversed(matching_indexes):
            candidate = runtime_root.joinpath(*requested_parts[index + 1 :])
            if candidate.exists():
                return str(candidate.resolve(strict=False))

    return str(requested_path)


def build_codex_runtime_environment() -> dict[str, str]:
    env = os.environ.copy()
    original_home = Path(env.get("HOME", str(Path.home()))).expanduser()
    original_codex_home = Path(env.get("CODEX_HOME", str(original_home / ".codex"))).expanduser()
    configured_runtime_root = Path(env.get("ATTRACTOR_CODEX_RUNTIME_ROOT", "/codex-runtime")).expanduser()
    runtime_root = configured_runtime_root
    try:
        runtime_root.mkdir(parents=True, exist_ok=True)
    except OSError:
        runtime_root = Path(tempfile.gettempdir()) / "sparkspawn-codex-runtime"
    codex_home = Path(env.get("CODEX_HOME", str(runtime_root / ".codex"))).expanduser()
    xdg_config_home = Path(env.get("XDG_CONFIG_HOME", str(runtime_root / ".config"))).expanduser()
    xdg_data_home = Path(env.get("XDG_DATA_HOME", str(runtime_root / ".local/share"))).expanduser()
    for directory in (runtime_root, codex_home, xdg_config_home, xdg_data_home):
        directory.mkdir(parents=True, exist_ok=True)

    explicit_seed_dir = Path(env.get("ATTRACTOR_CODEX_SEED_DIR", "/codex-seed")).expanduser()
    seed_candidates: list[Path] = []
    for candidate in (explicit_seed_dir, original_codex_home):
        normalized_candidate = candidate.expanduser()
        if normalized_candidate == codex_home:
            continue
        if normalized_candidate in seed_candidates:
            continue
        seed_candidates.append(normalized_candidate)
    for file_name in ("auth.json", "config.toml"):
        source = next((candidate / file_name for candidate in seed_candidates if (candidate / file_name).exists()), None)
        if source is None:
            continue
        destination = codex_home / file_name
        if destination.exists():
            try:
                if source.read_bytes() == destination.read_bytes():
                    continue
            except OSError:
                pass
        shutil.copy2(source, destination)

    env.update(
        {
            "HOME": str(runtime_root),
            "CODEX_HOME": str(codex_home),
            "XDG_CONFIG_HOME": str(xdg_config_home),
            "XDG_DATA_HOME": str(xdg_data_home),
        }
    )
    return env


@dataclass
class ConversationTurn:
    id: str
    role: str
    content: str
    timestamp: str
    status: str = "complete"
    kind: str = "message"
    artifact_id: Optional[str] = None
    parent_turn_id: Optional[str] = None
    error: Optional[str] = None

    def to_dict(self) -> dict[str, Any]:
        payload: dict[str, Any] = {
            "id": self.id,
            "role": self.role,
            "content": self.content,
            "timestamp": self.timestamp,
            "status": self.status,
            "kind": self.kind,
        }
        if self.artifact_id:
            payload["artifact_id"] = self.artifact_id
        if self.parent_turn_id:
            payload["parent_turn_id"] = self.parent_turn_id
        if self.error:
            payload["error"] = self.error
        return payload

    @classmethod
    def from_dict(cls, payload: dict[str, Any]) -> "ConversationTurn":
        return cls(
            id=str(payload.get("id", "")),
            role=str(payload.get("role", "assistant")),
            content=str(payload.get("content", "")),
            timestamp=str(payload.get("timestamp", "")),
            status=str(payload.get("status", "complete") or "complete"),
            kind=str(payload.get("kind", "message") or "message"),
            artifact_id=str(payload.get("artifact_id")) if payload.get("artifact_id") is not None else None,
            parent_turn_id=str(payload.get("parent_turn_id")) if payload.get("parent_turn_id") is not None else None,
            error=str(payload.get("error")) if payload.get("error") is not None else None,
        )


@dataclass
class ToolCallRecord:
    id: str
    kind: str
    status: str
    title: str
    command: Optional[str] = None
    output: Optional[str] = None
    file_paths: list[str] = field(default_factory=list)

    def to_dict(self) -> dict[str, Any]:
        payload: dict[str, Any] = {
            "id": self.id,
            "kind": self.kind,
            "status": self.status,
            "title": self.title,
        }
        if self.command:
            payload["command"] = self.command
        if self.output:
            payload["output"] = self.output
        if self.file_paths:
            payload["file_paths"] = list(self.file_paths)
        return payload

    @classmethod
    def from_dict(cls, payload: dict[str, Any]) -> "ToolCallRecord":
        raw_paths = payload.get("file_paths")
        return cls(
            id=str(payload.get("id", "")),
            kind=str(payload.get("kind", "")),
            status=str(payload.get("status", "completed") or "completed"),
            title=str(payload.get("title", "")),
            command=str(payload.get("command")) if payload.get("command") is not None else None,
            output=str(payload.get("output")) if payload.get("output") is not None else None,
            file_paths=[str(path) for path in raw_paths] if isinstance(raw_paths, list) else [],
        )


@dataclass
class ChatTurnResult:
    assistant_message: str
    tool_calls: list[ToolCallRecord] = field(default_factory=list)


@dataclass
class ConversationTurnEvent:
    id: str
    turn_id: str
    sequence: int
    timestamp: str
    kind: str
    content_delta: Optional[str] = None
    message: Optional[str] = None
    tool_call_id: Optional[str] = None
    tool_call: Optional[ToolCallRecord] = None

    def to_dict(self) -> dict[str, Any]:
        payload: dict[str, Any] = {
            "id": self.id,
            "turn_id": self.turn_id,
            "sequence": self.sequence,
            "timestamp": self.timestamp,
            "kind": self.kind,
        }
        if self.content_delta is not None:
            payload["content_delta"] = self.content_delta
        if self.message is not None:
            payload["message"] = self.message
        if self.tool_call_id is not None:
            payload["tool_call_id"] = self.tool_call_id
        if self.tool_call is not None:
            payload["tool_call"] = self.tool_call.to_dict()
        return payload

    @classmethod
    def from_dict(cls, payload: dict[str, Any]) -> "ConversationTurnEvent":
        return cls(
            id=str(payload.get("id", "")),
            turn_id=str(payload.get("turn_id", "")),
            sequence=int(payload.get("sequence", 0) or 0),
            timestamp=str(payload.get("timestamp", "")),
            kind=str(payload.get("kind", "")),
            content_delta=str(payload.get("content_delta")) if payload.get("content_delta") is not None else None,
            message=str(payload.get("message")) if payload.get("message") is not None else None,
            tool_call_id=str(payload.get("tool_call_id")) if payload.get("tool_call_id") is not None else None,
            tool_call=ToolCallRecord.from_dict(payload.get("tool_call"))
            if isinstance(payload.get("tool_call"), dict)
            else None,
        )


def _migrate_legacy_turns(raw_turns: list[dict[str, Any]]) -> tuple[list[ConversationTurn], list[ConversationTurnEvent]]:
    turns: list[ConversationTurn] = []
    turn_events: list[ConversationTurnEvent] = []
    last_user_turn_id: Optional[str] = None
    last_assistant_turn_id: Optional[str] = None
    event_sequence_by_turn: dict[str, int] = {}

    for raw_turn in raw_turns:
        if not isinstance(raw_turn, dict):
            continue
        legacy_turn = ConversationTurn.from_dict(raw_turn)
        legacy_tool_call = ToolCallRecord.from_dict(raw_turn["tool_call"]) if isinstance(raw_turn.get("tool_call"), dict) else None
        if legacy_turn.kind == "tool_call" and legacy_tool_call is not None:
            target_turn_id = last_assistant_turn_id
            if target_turn_id is None:
                synthetic_assistant_turn = ConversationTurn(
                    id=f"turn-{uuid.uuid4().hex}",
                    role="assistant",
                    content="",
                    timestamp=legacy_turn.timestamp or _iso_now(),
                    status="complete" if legacy_tool_call.status != "running" else "streaming",
                    parent_turn_id=last_user_turn_id,
                )
                turns.append(synthetic_assistant_turn)
                last_assistant_turn_id = synthetic_assistant_turn.id
                target_turn_id = synthetic_assistant_turn.id
            next_sequence = event_sequence_by_turn.get(target_turn_id, 0) + 1
            event_sequence_by_turn[target_turn_id] = next_sequence
            event_kind = {
                "running": "tool_call_started",
                "failed": "tool_call_failed",
            }.get(legacy_tool_call.status, "tool_call_completed")
            turn_events.append(
                ConversationTurnEvent(
                    id=f"event-{uuid.uuid4().hex}",
                    turn_id=target_turn_id,
                    sequence=next_sequence,
                    timestamp=legacy_turn.timestamp or _iso_now(),
                    kind=event_kind,
                    tool_call_id=legacy_tool_call.id or legacy_turn.id,
                    tool_call=legacy_tool_call,
                )
            )
            continue

        migrated_turn = ConversationTurn(
            id=legacy_turn.id,
            role=legacy_turn.role,
            content=legacy_turn.content,
            timestamp=legacy_turn.timestamp,
            status="streaming" if legacy_turn.id.endswith(":assistant:live") else legacy_turn.status,
            kind=legacy_turn.kind,
            artifact_id=legacy_turn.artifact_id,
            parent_turn_id=legacy_turn.parent_turn_id or last_user_turn_id if legacy_turn.role == "assistant" else legacy_turn.parent_turn_id,
            error=legacy_turn.error,
        )
        turns.append(migrated_turn)
        if migrated_turn.role == "user":
            last_user_turn_id = migrated_turn.id
            last_assistant_turn_id = None
        elif migrated_turn.role == "assistant":
            last_assistant_turn_id = migrated_turn.id

    return turns, turn_events


@dataclass
class WorkflowEvent:
    message: str
    timestamp: str

    def to_dict(self) -> dict[str, Any]:
        return {
            "message": self.message,
            "timestamp": self.timestamp,
        }

    @classmethod
    def from_dict(cls, payload: dict[str, Any]) -> "WorkflowEvent":
        return cls(
            message=str(payload.get("message", "")),
            timestamp=str(payload.get("timestamp", "")),
        )


@dataclass
class SpecEditProposalChange:
    path: str
    before: str
    after: str

    def to_dict(self) -> dict[str, Any]:
        return {
            "path": self.path,
            "before": self.before,
            "after": self.after,
        }

    @classmethod
    def from_dict(cls, payload: dict[str, Any]) -> "SpecEditProposalChange":
        return cls(
            path=str(payload.get("path", "")),
            before=str(payload.get("before", "")),
            after=str(payload.get("after", "")),
        )


@dataclass
class SpecEditProposal:
    id: str
    created_at: str
    summary: str
    changes: list[SpecEditProposalChange]
    status: str = "pending"
    canonical_spec_edit_id: Optional[str] = None
    approved_at: Optional[str] = None
    git_branch: Optional[str] = None
    git_commit: Optional[str] = None

    def to_dict(self) -> dict[str, Any]:
        payload: dict[str, Any] = {
            "id": self.id,
            "created_at": self.created_at,
            "summary": self.summary,
            "status": self.status,
            "changes": [change.to_dict() for change in self.changes],
        }
        if self.canonical_spec_edit_id:
            payload["canonical_spec_edit_id"] = self.canonical_spec_edit_id
        if self.approved_at:
            payload["approved_at"] = self.approved_at
        if self.git_branch:
            payload["git_branch"] = self.git_branch
        if self.git_commit:
            payload["git_commit"] = self.git_commit
        return payload

    @classmethod
    def from_dict(cls, payload: dict[str, Any]) -> "SpecEditProposal":
        raw_changes = payload.get("changes")
        changes = [
            SpecEditProposalChange.from_dict(change)
            for change in raw_changes
            if isinstance(change, dict)
        ] if isinstance(raw_changes, list) else []
        return cls(
            id=str(payload.get("id", "")),
            created_at=str(payload.get("created_at", "")),
            summary=str(payload.get("summary", "")),
            changes=changes,
            status=str(payload.get("status", "pending") or "pending"),
            canonical_spec_edit_id=str(payload.get("canonical_spec_edit_id")) if payload.get("canonical_spec_edit_id") is not None else None,
            approved_at=str(payload.get("approved_at")) if payload.get("approved_at") is not None else None,
            git_branch=str(payload.get("git_branch")) if payload.get("git_branch") is not None else None,
            git_commit=str(payload.get("git_commit")) if payload.get("git_commit") is not None else None,
        )


@dataclass
class ExecutionCardReview:
    id: str
    disposition: str
    message: str
    created_at: str
    author: str = "user"

    def to_dict(self) -> dict[str, Any]:
        return {
            "id": self.id,
            "disposition": self.disposition,
            "message": self.message,
            "created_at": self.created_at,
            "author": self.author,
        }

    @classmethod
    def from_dict(cls, payload: dict[str, Any]) -> "ExecutionCardReview":
        return cls(
            id=str(payload.get("id", "")),
            disposition=str(payload.get("disposition", "")),
            message=str(payload.get("message", "")),
            created_at=str(payload.get("created_at", "")),
            author=str(payload.get("author", "user") or "user"),
        )


@dataclass
class ExecutionCardWorkItem:
    id: str
    title: str
    description: str
    acceptance_criteria: list[str] = field(default_factory=list)
    depends_on: list[str] = field(default_factory=list)

    def to_dict(self) -> dict[str, Any]:
        return {
            "id": self.id,
            "title": self.title,
            "description": self.description,
            "acceptance_criteria": list(self.acceptance_criteria),
            "depends_on": list(self.depends_on),
        }

    @classmethod
    def from_dict(cls, payload: dict[str, Any]) -> "ExecutionCardWorkItem":
        raw_acceptance = payload.get("acceptance_criteria")
        raw_depends_on = payload.get("depends_on")
        return cls(
            id=str(payload.get("id", "")),
            title=str(payload.get("title", "")),
            description=str(payload.get("description", "")),
            acceptance_criteria=[str(item) for item in raw_acceptance] if isinstance(raw_acceptance, list) else [],
            depends_on=[str(item) for item in raw_depends_on] if isinstance(raw_depends_on, list) else [],
        )


@dataclass
class ExecutionCard:
    id: str
    title: str
    summary: str
    objective: str
    source_spec_edit_id: str
    source_workflow_run_id: str
    created_at: str
    updated_at: str
    status: str = "draft"
    flow_source: Optional[str] = None
    work_items: list[ExecutionCardWorkItem] = field(default_factory=list)
    review_feedback: list[ExecutionCardReview] = field(default_factory=list)

    def to_dict(self) -> dict[str, Any]:
        payload: dict[str, Any] = {
            "id": self.id,
            "title": self.title,
            "summary": self.summary,
            "objective": self.objective,
            "source_spec_edit_id": self.source_spec_edit_id,
            "source_workflow_run_id": self.source_workflow_run_id,
            "created_at": self.created_at,
            "updated_at": self.updated_at,
            "status": self.status,
            "work_items": [item.to_dict() for item in self.work_items],
            "review_feedback": [entry.to_dict() for entry in self.review_feedback],
        }
        if self.flow_source:
            payload["flow_source"] = self.flow_source
        return payload

    @classmethod
    def from_dict(cls, payload: dict[str, Any]) -> "ExecutionCard":
        raw_items = payload.get("work_items")
        raw_reviews = payload.get("review_feedback")
        return cls(
            id=str(payload.get("id", "")),
            title=str(payload.get("title", "")),
            summary=str(payload.get("summary", "")),
            objective=str(payload.get("objective", "")),
            source_spec_edit_id=str(payload.get("source_spec_edit_id", "")),
            source_workflow_run_id=str(payload.get("source_workflow_run_id", "")),
            created_at=str(payload.get("created_at", "")),
            updated_at=str(payload.get("updated_at", "")),
            status=str(payload.get("status", "draft") or "draft"),
            flow_source=str(payload.get("flow_source")) if payload.get("flow_source") is not None else None,
            work_items=[
                ExecutionCardWorkItem.from_dict(item)
                for item in raw_items
                if isinstance(item, dict)
            ] if isinstance(raw_items, list) else [],
            review_feedback=[
                ExecutionCardReview.from_dict(item)
                for item in raw_reviews
                if isinstance(item, dict)
            ] if isinstance(raw_reviews, list) else [],
        )


@dataclass
class ExecutionWorkflowState:
    run_id: Optional[str] = None
    status: str = "idle"
    error: Optional[str] = None
    flow_source: Optional[str] = None

    def to_dict(self) -> dict[str, Any]:
        payload: dict[str, Any] = {
            "status": self.status,
        }
        if self.run_id:
            payload["run_id"] = self.run_id
        if self.error:
            payload["error"] = self.error
        if self.flow_source:
            payload["flow_source"] = self.flow_source
        return payload

    @classmethod
    def from_dict(cls, payload: dict[str, Any]) -> "ExecutionWorkflowState":
        return cls(
            run_id=str(payload.get("run_id")) if payload.get("run_id") is not None else None,
            status=str(payload.get("status", "idle") or "idle"),
            error=str(payload.get("error")) if payload.get("error") is not None else None,
            flow_source=str(payload.get("flow_source")) if payload.get("flow_source") is not None else None,
        )


@dataclass
class ConversationState:
    conversation_id: str
    project_path: str
    title: str = "New thread"
    created_at: str = ""
    updated_at: str = ""
    turns: list[ConversationTurn] = field(default_factory=list)
    turn_events: list[ConversationTurnEvent] = field(default_factory=list)
    event_log: list[WorkflowEvent] = field(default_factory=list)
    spec_edit_proposals: list[SpecEditProposal] = field(default_factory=list)
    execution_cards: list[ExecutionCard] = field(default_factory=list)
    execution_workflow: ExecutionWorkflowState = field(default_factory=ExecutionWorkflowState)

    def persisted_turn_events(self) -> list[ConversationTurnEvent]:
        return [
            event
            for event in self.turn_events
            if event.kind != "assistant_delta"
        ]

    def to_dict(self) -> dict[str, Any]:
        return {
            "conversation_id": self.conversation_id,
            "project_path": self.project_path,
            "title": self.title,
            "created_at": self.created_at,
            "updated_at": self.updated_at,
            "turns": [turn.to_dict() for turn in self.turns],
            "turn_events": [event.to_dict() for event in self.persisted_turn_events()],
            "event_log": [entry.to_dict() for entry in self.event_log],
            "spec_edit_proposals": [proposal.to_dict() for proposal in self.spec_edit_proposals],
            "execution_cards": [card.to_dict() for card in self.execution_cards],
            "execution_workflow": self.execution_workflow.to_dict(),
        }

    @classmethod
    def from_dict(cls, payload: dict[str, Any]) -> "ConversationState":
        raw_turns = payload.get("turns")
        raw_turn_events = payload.get("turn_events")
        raw_events = payload.get("event_log")
        raw_proposals = payload.get("spec_edit_proposals")
        raw_cards = payload.get("execution_cards")
        if isinstance(raw_turn_events, list):
            turns = [
                ConversationTurn.from_dict(turn)
                for turn in raw_turns
                if isinstance(turn, dict)
            ] if isinstance(raw_turns, list) else []
            turn_events = [
                ConversationTurnEvent.from_dict(event)
                for event in raw_turn_events
                if isinstance(event, dict)
            ]
        else:
            turns, turn_events = _migrate_legacy_turns(raw_turns if isinstance(raw_turns, list) else [])
        created_at = _as_non_empty_string(payload.get("created_at"))
        updated_at = _as_non_empty_string(payload.get("updated_at"))
        if not created_at:
            created_at = turns[0].timestamp if turns else ""
        if not updated_at:
            updated_at = turns[-1].timestamp if turns else created_at
        return cls(
            conversation_id=str(payload.get("conversation_id", "")),
            project_path=_normalize_project_path(str(payload.get("project_path", ""))),
            title=_as_non_empty_string(payload.get("title")) or _derive_conversation_title(turns),
            created_at=created_at or _iso_now(),
            updated_at=updated_at or created_at or _iso_now(),
            turns=turns,
            turn_events=turn_events,
            event_log=[
                WorkflowEvent.from_dict(entry)
                for entry in raw_events
                if isinstance(entry, dict)
            ] if isinstance(raw_events, list) else [],
            spec_edit_proposals=[
                SpecEditProposal.from_dict(entry)
                for entry in raw_proposals
                if isinstance(entry, dict)
            ] if isinstance(raw_proposals, list) else [],
            execution_cards=[
                ExecutionCard.from_dict(entry)
                for entry in raw_cards
                if isinstance(entry, dict)
            ] if isinstance(raw_cards, list) else [],
            execution_workflow=ExecutionWorkflowState.from_dict(payload.get("execution_workflow", {}))
            if isinstance(payload.get("execution_workflow"), dict)
            else ExecutionWorkflowState(),
        )


@dataclass
class ConversationSummary:
    conversation_id: str
    project_path: str
    title: str
    created_at: str
    updated_at: str
    last_message_preview: Optional[str] = None

    def to_dict(self) -> dict[str, Any]:
        payload: dict[str, Any] = {
            "conversation_id": self.conversation_id,
            "project_path": self.project_path,
            "title": self.title,
            "created_at": self.created_at,
            "updated_at": self.updated_at,
        }
        if self.last_message_preview:
            payload["last_message_preview"] = self.last_message_preview
        return payload


@dataclass
class ConversationSessionState:
    conversation_id: str
    updated_at: str
    project_path: str
    runtime_project_path: str
    thread_id: Optional[str] = None

    def to_dict(self) -> dict[str, Any]:
        payload: dict[str, Any] = {
            "conversation_id": self.conversation_id,
            "updated_at": self.updated_at,
            "project_path": self.project_path,
            "runtime_project_path": self.runtime_project_path,
        }
        if self.thread_id:
            payload["thread_id"] = self.thread_id
        return payload

    @classmethod
    def from_dict(cls, payload: dict[str, Any]) -> "ConversationSessionState":
        return cls(
            conversation_id=str(payload.get("conversation_id", "")),
            updated_at=str(payload.get("updated_at", "")),
            project_path=_normalize_project_path(str(payload.get("project_path", ""))),
            runtime_project_path=_normalize_project_path(str(payload.get("runtime_project_path", ""))),
            thread_id=str(payload.get("thread_id")) if payload.get("thread_id") is not None else None,
        )


class ConversationEventHub:
    def __init__(self) -> None:
        self._lock = threading.Lock()
        self._subscribers: dict[str, list[asyncio.Queue[dict[str, Any]]]] = {}

    def subscribe(self, conversation_id: str) -> asyncio.Queue[dict[str, Any]]:
        queue: asyncio.Queue[dict[str, Any]] = asyncio.Queue(maxsize=32)
        with self._lock:
            self._subscribers.setdefault(conversation_id, []).append(queue)
        return queue

    def unsubscribe(self, conversation_id: str, queue: asyncio.Queue[dict[str, Any]]) -> None:
        with self._lock:
            listeners = self._subscribers.get(conversation_id)
            if not listeners:
                return
            if queue in listeners:
                listeners.remove(queue)
            if not listeners:
                self._subscribers.pop(conversation_id, None)

    async def publish(self, conversation_id: str, payload: dict[str, Any]) -> None:
        with self._lock:
            listeners = list(self._subscribers.get(conversation_id, []))
        for queue in listeners:
            try:
                queue.put_nowait(payload)
            except asyncio.QueueFull:
                try:
                    queue.get_nowait()
                except asyncio.QueueEmpty:
                    pass
                try:
                    queue.put_nowait(payload)
                except asyncio.QueueFull:
                    continue


class CodexAppServerChatSession:
    def __init__(
        self,
        working_dir: str,
        *,
        persisted_thread_id: Optional[str] = None,
        on_thread_id_updated: Optional[Callable[[str], None]] = None,
    ) -> None:
        self.requested_working_dir = _normalize_project_path(working_dir)
        self.working_dir = resolve_runtime_workspace_path(working_dir)
        self._proc: Optional[subprocess.Popen[str]] = None
        self._selector: Optional[selectors.DefaultSelector] = None
        self._request_id = 0
        self._thread_id: Optional[str] = persisted_thread_id
        self._thread_initialized = False
        self._on_thread_id_updated = on_thread_id_updated
        self._lock = threading.Lock()

    def _close(self) -> None:
        if self._selector is not None:
            try:
                self._selector.close()
            except Exception:
                pass
            self._selector = None
        if self._proc is not None:
            try:
                if self._proc.poll() is None:
                    self._proc.terminate()
                    try:
                        self._proc.wait(timeout=2)
                    except subprocess.TimeoutExpired:
                        self._proc.kill()
            except Exception:
                pass
            self._proc = None
        self._thread_initialized = False

    def close(self) -> None:
        with self._lock:
            self._close()

    def _ensure_process(self) -> None:
        if self._proc is not None and self._proc.poll() is None:
            return
        self._close()
        try:
            proc = subprocess.Popen(
                ["codex", "app-server"],
                cwd=self.working_dir,
                env=build_codex_runtime_environment(),
                stdin=subprocess.PIPE,
                stdout=subprocess.PIPE,
                stderr=subprocess.STDOUT,
                text=True,
                bufsize=1,
                start_new_session=True,
            )
        except FileNotFoundError as exc:
            if not Path(self.working_dir).exists():
                raise RuntimeError(
                    "codex app-server working directory is unavailable in the runtime: "
                    f"requested {self.requested_working_dir or self.working_dir}, resolved {self.working_dir}"
                ) from exc
            raise RuntimeError("codex app-server not found on PATH") from exc
        selector = selectors.DefaultSelector()
        if proc.stdout is None:
            self._close()
            raise RuntimeError("codex app-server did not expose stdout")
        selector.register(proc.stdout, selectors.EVENT_READ)
        self._proc = proc
        self._selector = selector
        self._request_id = 0
        self._thread_initialized = False
        init_response = self._send_request("initialize", {"clientInfo": {"name": "sparkspawn", "version": "0.1"}})
        if init_response.get("error"):
            self._close()
            raise RuntimeError("codex app-server initialize failed")

    def _send_json(self, payload: dict[str, Any]) -> None:
        if self._proc is None or self._proc.stdin is None:
            raise RuntimeError("codex app-server stdin unavailable")
        self._proc.stdin.write(json.dumps(payload) + "\n")
        self._proc.stdin.flush()

    def _send_response(self, request_id: Any, result: Optional[dict[str, Any]] = None) -> None:
        self._send_json({
            "jsonrpc": "2.0",
            "id": request_id,
            "result": result or {},
        })

    def _read_line(self, wait: float) -> Optional[str]:
        if self._proc is None or self._selector is None or self._proc.stdout is None:
            return None
        events = self._selector.select(timeout=max(wait, 0))
        if not events:
            return None
        line = self._proc.stdout.readline()
        if not line:
            return None
        return line.rstrip("\n")

    def _handle_server_request(self, message: dict[str, Any]) -> None:
        method = message.get("method")
        request_id = message.get("id")
        if method in {"item/commandExecution/requestApproval", "item/fileChange/requestApproval"}:
            self._send_response(request_id, {"decision": "acceptForSession"})
            return
        self._send_json({
            "jsonrpc": "2.0",
            "id": request_id,
            "error": {"code": -32000, "message": f"Unsupported request: {method}"},
        })

    def _wait_for_response(self, target_id: int) -> dict[str, Any]:
        started_at = time.monotonic()
        while True:
            line = self._read_line(0.1)
            if line is None:
                if self._proc is not None and self._proc.poll() is not None:
                    raise RuntimeError("codex app-server exited unexpectedly")
                if time.monotonic() - started_at >= APP_SERVER_REQUEST_TIMEOUT_SECONDS:
                    self._close()
                    raise RuntimeError("codex app-server request timed out waiting for response")
                continue
            try:
                message = json.loads(line)
            except json.JSONDecodeError:
                continue
            if "id" in message and "method" in message:
                self._handle_server_request(message)
                continue
            if message.get("id") == target_id:
                return message

    def _send_request(self, method: str, params: Optional[dict[str, Any]]) -> dict[str, Any]:
        self._request_id += 1
        payload: dict[str, Any] = {"jsonrpc": "2.0", "id": self._request_id, "method": method}
        if params is not None:
            payload["params"] = params
        self._send_json(payload)
        return self._wait_for_response(self._request_id)

    def _set_thread_id(self, thread_id: str) -> None:
        normalized_thread_id = _as_non_empty_string(thread_id)
        if not normalized_thread_id:
            return
        self._thread_id = normalized_thread_id
        if self._on_thread_id_updated is not None:
            self._on_thread_id_updated(normalized_thread_id)

    def _resume_thread(self, model: Optional[str]) -> bool:
        if not self._thread_id:
            return False
        params: dict[str, Any] = {
            "threadId": self._thread_id,
            "cwd": self.working_dir,
            "sandbox": "danger-full-access",
            "approvalPolicy": "never",
        }
        if model:
            params["model"] = model
        response = self._send_request("thread/resume", params)
        if response.get("error"):
            return False
        thread = (response.get("result") or {}).get("thread") or {}
        thread_id = thread.get("id")
        if not thread_id:
            return False
        self._set_thread_id(str(thread_id))
        self._thread_initialized = True
        return True

    def _start_thread(self, model: Optional[str]) -> None:
        params: dict[str, Any] = {
            "cwd": self.working_dir,
            "sandbox": "danger-full-access",
            "approvalPolicy": "never",
            "ephemeral": False,
        }
        if model:
            params["model"] = model
        response = self._send_request("thread/start", params)
        if response.get("error"):
            raise RuntimeError("codex app-server thread/start failed")
        thread = (response.get("result") or {}).get("thread") or {}
        thread_id = thread.get("id")
        if not thread_id:
            raise RuntimeError("codex app-server did not return a thread id")
        self._set_thread_id(str(thread_id))
        self._thread_initialized = True

    def _ensure_thread(self, model: Optional[str]) -> None:
        if self._thread_initialized and self._thread_id:
            return
        if self._resume_thread(model):
            return
        self._start_thread(model)

    def _emit_progress(
        self,
        callback: Optional[Callable[[ChatTurnResult], None]],
        tool_calls: list[ToolCallRecord],
        assistant_message: str,
    ) -> None:
        if callback is None:
            return
        callback(
            ChatTurnResult(
                assistant_message=assistant_message,
                tool_calls=[
                    ToolCallRecord.from_dict(tool_call.to_dict())
                    for tool_call in tool_calls
                ],
            )
        )

    def turn(
        self,
        prompt: str,
        model: Optional[str],
        *,
        on_progress: Optional[Callable[[ChatTurnResult], None]] = None,
    ) -> ChatTurnResult:
        with self._lock:
            self._ensure_process()
            self._ensure_thread(model)
            agent_chunks: list[str] = []
            last_error: Optional[str] = None
            saw_final_agent_message = False
            saw_item_agent_message_delta = False
            last_activity_at = time.monotonic()
            last_agent_activity_at: float | None = None
            tool_calls: list[ToolCallRecord] = []
            tool_call_index_by_item_id: dict[str, int] = {}
            params: dict[str, Any] = {
                "threadId": self._thread_id,
                "input": [{"type": "text", "text": prompt}],
                "approvalPolicy": "never",
                "sandboxPolicy": {"type": "dangerFullAccess"},
                "cwd": self.working_dir,
            }
            if model:
                params["model"] = model
            response = self._send_request("turn/start", params)
            if response.get("error"):
                raise RuntimeError("codex app-server turn/start failed")
            while True:
                line = self._read_line(0.1)
                if line is None:
                    idle_for = time.monotonic() - last_activity_at
                    if idle_for >= CHAT_TURN_IDLE_TIMEOUT_SECONDS:
                        self._close()
                        raise RuntimeError("codex app-server turn timed out waiting for activity")
                    if saw_final_agent_message and "".join(agent_chunks).strip():
                        if idle_for >= FINAL_ANSWER_IDLE_GRACE_SECONDS:
                            _log_project_chat_debug(
                                "completed turn after final-answer quiet period",
                                thread_id=self._thread_id,
                                idle_for=idle_for,
                                assistant_message="".join(agent_chunks).strip(),
                            )
                            break
                    if (
                        not saw_final_agent_message
                        and last_agent_activity_at is not None
                        and "".join(agent_chunks).strip()
                        and not any(tool_call.status == "running" for tool_call in tool_calls)
                    ):
                        agent_idle_for = time.monotonic() - last_agent_activity_at
                        if agent_idle_for >= LIVE_ASSISTANT_IDLE_COMPLETION_GRACE_SECONDS:
                            _log_project_chat_debug(
                                "completed turn after live-assistant quiet period",
                                thread_id=self._thread_id,
                                idle_for=idle_for,
                                agent_idle_for=agent_idle_for,
                                assistant_message="".join(agent_chunks).strip(),
                            )
                            break
                    if self._proc is not None and self._proc.poll() is not None:
                        self._close()
                        raise RuntimeError("codex app-server exited before turn completion")
                    continue
                last_activity_at = time.monotonic()
                try:
                    message = json.loads(line)
                except json.JSONDecodeError:
                    continue
                if "id" in message and "method" in message:
                    request_method = message.get("method")
                    request_params = message.get("params") or {}
                    if request_method == "item/commandExecution/requestApproval":
                        command = _extract_command_text(request_params)
                        item_id = _as_non_empty_string(request_params.get("itemId")) or f"tool-{uuid.uuid4().hex}"
                        tool_calls.append(
                            ToolCallRecord(
                                id=item_id,
                                kind="command_execution",
                                status="running",
                                title="Run command",
                                command=command,
                            )
                        )
                        if item_id:
                            tool_call_index_by_item_id[item_id] = len(tool_calls) - 1
                        self._emit_progress(on_progress, tool_calls, "".join(agent_chunks).strip())
                    elif request_method == "item/fileChange/requestApproval":
                        file_paths = _extract_file_paths(request_params)
                        item_id = _as_non_empty_string(request_params.get("itemId")) or f"tool-{uuid.uuid4().hex}"
                        tool_calls.append(
                            ToolCallRecord(
                                id=item_id,
                                kind="file_change",
                                status="running",
                                title="Apply file changes",
                                file_paths=file_paths,
                            )
                        )
                        if item_id:
                            tool_call_index_by_item_id[item_id] = len(tool_calls) - 1
                        self._emit_progress(on_progress, tool_calls, "".join(agent_chunks).strip())
                    self._handle_server_request(message)
                    continue
                method = message.get("method")
                params = message.get("params") or {}
                if method == "item/started":
                    item = params.get("item")
                    if isinstance(item, dict):
                        item_id = _as_non_empty_string(item.get("id"))
                        tool_call = _tool_call_from_item(item)
                        if tool_call is not None:
                            if item_id and item_id in tool_call_index_by_item_id:
                                tool_calls[tool_call_index_by_item_id[item_id]] = tool_call
                            else:
                                tool_calls.append(tool_call)
                                if item_id:
                                    tool_call_index_by_item_id[item_id] = len(tool_calls) - 1
                            self._emit_progress(on_progress, tool_calls, "".join(agent_chunks).strip())
                    continue
                if method == "item/completed":
                    item = params.get("item")
                    if isinstance(item, dict):
                        item_id = _as_non_empty_string(item.get("id"))
                        tool_call = _tool_call_from_item(item)
                        item_type = str(item.get("type") or "").strip().lower()
                        item_phase = str(item.get("phase") or "").strip().lower()
                        if item_type == "agentmessage" and item_phase == "final_answer":
                            saw_final_agent_message = True
                            last_agent_activity_at = time.monotonic()
                        if tool_call is not None:
                            if item_id and item_id in tool_call_index_by_item_id:
                                tool_calls[tool_call_index_by_item_id[item_id]] = tool_call
                            else:
                                tool_calls.append(tool_call)
                                if item_id:
                                    tool_call_index_by_item_id[item_id] = len(tool_calls) - 1
                            self._emit_progress(on_progress, tool_calls, "".join(agent_chunks).strip())
                    continue
                if method == "item/agentMessage/delta":
                    delta = params.get("delta") or ""
                    if delta:
                        saw_item_agent_message_delta = True
                        agent_chunks.append(str(delta))
                        last_agent_activity_at = time.monotonic()
                        self._emit_progress(on_progress, tool_calls, "".join(agent_chunks).strip())
                    continue
                if method == "codex/event/agent_message_delta":
                    if saw_item_agent_message_delta:
                        continue
                    delta = (params.get("msg") or {}).get("delta") or ""
                    if delta:
                        agent_chunks.append(str(delta))
                        last_agent_activity_at = time.monotonic()
                        self._emit_progress(on_progress, tool_calls, "".join(agent_chunks).strip())
                    continue
                if method == "codex/event/agent_message":
                    msg = (params.get("msg") or {}).get("message")
                    phase = str((params.get("msg") or {}).get("phase") or "").strip().lower()
                    if msg:
                        agent_chunks = [str(msg)]
                        last_agent_activity_at = time.monotonic()
                        self._emit_progress(on_progress, tool_calls, "".join(agent_chunks).strip())
                    if phase == "final_answer":
                        saw_final_agent_message = True
                    continue
                if method == "item/commandExecution/outputDelta":
                    delta = _as_non_empty_string(params.get("delta"))
                    item_id = _as_non_empty_string(params.get("itemId"))
                    tool_call_index = tool_call_index_by_item_id.get(item_id or "")
                    if delta and tool_call_index is not None and tool_call_index < len(tool_calls):
                        tool_calls[tool_call_index].output = _append_tool_output(
                            tool_calls[tool_call_index].output,
                            delta,
                        )
                        self._emit_progress(on_progress, tool_calls, "".join(agent_chunks).strip())
                    continue
                if method == "error":
                    last_error = str(params.get("message") or "codex app-server error")
                    continue
                if method == "turn/completed":
                    turn = params.get("turn") or {}
                    status = str(turn.get("status") or "")
                    if status and status != "completed":
                        error = turn.get("error") or {}
                        last_error = str(error.get("message") or last_error or f"turn ended with status '{status}'")
                    break
                if method == "codex/event/task_complete":
                    msg = params.get("msg") or {}
                    last_agent_message = _as_non_empty_string(msg.get("last_agent_message"))
                    if last_agent_message and not "".join(agent_chunks).strip():
                        agent_chunks = [last_agent_message]
                    break
            for tool_call in tool_calls:
                if tool_call.status == "running":
                    tool_call.status = "failed" if last_error else "completed"
            self._emit_progress(on_progress, tool_calls, "".join(agent_chunks).strip())
            if last_error:
                raise RuntimeError(last_error)
            response_text = "".join(agent_chunks).strip()
            if not response_text:
                raise RuntimeError("codex app-server returned an empty chat response")
            return ChatTurnResult(
                assistant_message=response_text,
                tool_calls=tool_calls,
            )


class ProjectChatService:
    def __init__(self, data_dir: Path) -> None:
        self._data_dir = data_dir
        self._lock = threading.Lock()
        self._event_hub = ConversationEventHub()
        self._sessions_lock = threading.Lock()
        self._sessions: dict[str, CodexAppServerChatSession] = {}

    def events(self) -> ConversationEventHub:
        return self._event_hub

    def _conversations_root(self) -> Path:
        root = self._data_dir / "conversations"
        root.mkdir(parents=True, exist_ok=True)
        return root

    def _conversation_root(self, conversation_id: str) -> Path:
        root = self._conversations_root() / conversation_id
        root.mkdir(parents=True, exist_ok=True)
        return root

    def _conversation_state_path(self, conversation_id: str) -> Path:
        return self._conversation_root(conversation_id) / "state.json"

    def _conversation_session_path(self, conversation_id: str) -> Path:
        return self._conversation_root(conversation_id) / "session.json"

    def _touch_conversation_state(self, state: ConversationState, *, title_hint: Optional[str] = None) -> None:
        if not state.created_at:
            state.created_at = _iso_now()
        if title_hint:
            normalized_title_hint = _truncate_text(title_hint, 64)
            if state.title == "New thread" or not _as_non_empty_string(state.title):
                state.title = normalized_title_hint
        elif not _as_non_empty_string(state.title):
            state.title = _derive_conversation_title(state.turns)
        if state.title == "New thread":
            derived_title = _derive_conversation_title(state.turns)
            if derived_title != "New thread":
                state.title = derived_title
        state.updated_at = _iso_now()

    def _build_conversation_summary(self, state: ConversationState) -> ConversationSummary:
        return ConversationSummary(
            conversation_id=state.conversation_id,
            project_path=state.project_path,
            title=_as_non_empty_string(state.title) or _derive_conversation_title(state.turns),
            created_at=state.created_at or _iso_now(),
            updated_at=state.updated_at or state.created_at or _iso_now(),
            last_message_preview=_build_conversation_preview(state.turns),
        )

    def _read_state(self, conversation_id: str) -> Optional[ConversationState]:
        path = self._conversation_state_path(conversation_id)
        try:
            payload = json.loads(path.read_text(encoding="utf-8"))
        except Exception:
            return None
        if not isinstance(payload, dict):
            return None
        state = ConversationState.from_dict(payload)
        if not state.conversation_id:
            state.conversation_id = conversation_id
        return state

    def _write_state(self, state: ConversationState) -> None:
        path = self._conversation_state_path(state.conversation_id)
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(json.dumps(state.to_dict(), indent=2, sort_keys=True), encoding="utf-8")

    def _read_session_state(self, conversation_id: str) -> Optional[ConversationSessionState]:
        path = self._conversation_session_path(conversation_id)
        try:
            payload = json.loads(path.read_text(encoding="utf-8"))
        except Exception:
            return None
        if not isinstance(payload, dict):
            return None
        state = ConversationSessionState.from_dict(payload)
        if not state.conversation_id:
            state.conversation_id = conversation_id
        return state

    def _write_session_state(self, state: ConversationSessionState) -> None:
        path = self._conversation_session_path(state.conversation_id)
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(json.dumps(state.to_dict(), indent=2, sort_keys=True), encoding="utf-8")

    def _persist_session_thread(
        self,
        conversation_id: str,
        project_path: str,
        thread_id: str,
    ) -> None:
        normalized_project_path = _normalize_project_path(project_path)
        runtime_project_path = resolve_runtime_workspace_path(normalized_project_path)
        with self._lock:
            session_state = self._read_session_state(conversation_id)
            if session_state is None:
                session_state = ConversationSessionState(
                    conversation_id=conversation_id,
                    updated_at=_iso_now(),
                    project_path=normalized_project_path,
                    runtime_project_path=runtime_project_path,
                )
            session_state.thread_id = thread_id
            session_state.project_path = normalized_project_path
            session_state.runtime_project_path = runtime_project_path
            session_state.updated_at = _iso_now()
            self._write_session_state(session_state)

    async def publish_snapshot(self, conversation_id: str) -> None:
        snapshot = self.get_snapshot(conversation_id)
        await self._event_hub.publish(conversation_id, {"type": "conversation_snapshot", "state": snapshot})

    def list_conversations(self, project_path: str) -> list[dict[str, Any]]:
        normalized_project_path = _normalize_project_path(project_path)
        if not normalized_project_path:
            raise ValueError("Project path is required.")
        with self._lock:
            summaries: list[ConversationSummary] = []
            for state_path in self._conversations_root().glob("*/state.json"):
                try:
                    payload = json.loads(state_path.read_text(encoding="utf-8"))
                except Exception:
                    continue
                if not isinstance(payload, dict):
                    continue
                state = ConversationState.from_dict(payload)
                if state.project_path != normalized_project_path:
                    continue
                summaries.append(self._build_conversation_summary(state))
        summaries.sort(key=lambda entry: entry.updated_at, reverse=True)
        return [summary.to_dict() for summary in summaries]

    def get_snapshot(self, conversation_id: str, project_path: Optional[str] = None) -> dict[str, Any]:
        with self._lock:
            should_write_state = False
            state = self._read_state(conversation_id)
            if state is None:
                normalized_project_path = _normalize_project_path(project_path or "")
                if not normalized_project_path:
                    raise FileNotFoundError(conversation_id)
                state = ConversationState(
                    conversation_id=conversation_id,
                    project_path=normalized_project_path,
                )
                self._touch_conversation_state(state)
                should_write_state = True
            elif project_path:
                normalized_project_path = _normalize_project_path(project_path)
                if normalized_project_path and normalized_project_path != state.project_path:
                    raise ValueError("Conversation is already bound to a different project path.")
            if not state.created_at or not state.updated_at or not _as_non_empty_string(state.title):
                if not state.created_at:
                    state.created_at = _iso_now()
                if not state.updated_at:
                    state.updated_at = state.created_at
                if not _as_non_empty_string(state.title):
                    state.title = _derive_conversation_title(state.turns)
                should_write_state = True
            if len(state.persisted_turn_events()) != len(state.turn_events):
                should_write_state = True
            if should_write_state:
                self._write_state(state)
        return state.to_dict()

    def delete_conversation(self, conversation_id: str, project_path: str) -> dict[str, Any]:
        normalized_project_path = _normalize_project_path(project_path)
        if not normalized_project_path:
            raise ValueError("Project path is required.")

        conversation_root = self._conversations_root() / conversation_id
        with self._lock:
            state = self._read_state(conversation_id)
            if state is None or state.project_path != normalized_project_path:
                raise FileNotFoundError(conversation_id)

        with self._sessions_lock:
            session = self._sessions.pop(conversation_id, None)
        if session is not None:
            session.close()

        if conversation_root.exists():
            shutil.rmtree(conversation_root)

        return {
            "status": "deleted",
            "conversation_id": conversation_id,
            "project_path": normalized_project_path,
        }

    def _append_event(self, state: ConversationState, message: str) -> None:
        state.event_log.append(WorkflowEvent(message=message, timestamp=_iso_now()))

    def _next_turn_event_sequence(self, state: ConversationState, turn_id: str) -> int:
        max_sequence = 0
        for event in state.turn_events:
            if event.turn_id == turn_id and event.sequence > max_sequence:
                max_sequence = event.sequence
        return max_sequence + 1

    def _append_turn_event(
        self,
        state: ConversationState,
        turn_id: str,
        kind: str,
        *,
        content_delta: Optional[str] = None,
        message: Optional[str] = None,
        tool_call_id: Optional[str] = None,
        tool_call: Optional[ToolCallRecord] = None,
        timestamp: Optional[str] = None,
    ) -> ConversationTurnEvent:
        event = ConversationTurnEvent(
            id=f"event-{uuid.uuid4().hex}",
            turn_id=turn_id,
            sequence=self._next_turn_event_sequence(state, turn_id),
            timestamp=timestamp or _iso_now(),
            kind=kind,
            content_delta=content_delta,
            message=message,
            tool_call_id=tool_call_id,
            tool_call=ToolCallRecord.from_dict(tool_call.to_dict()) if tool_call is not None else None,
        )
        state.turn_events.append(event)
        return event

    def _upsert_turn(self, state: ConversationState, turn: ConversationTurn) -> None:
        for index, existing_turn in enumerate(state.turns):
            if existing_turn.id != turn.id:
                continue
            state.turns[index] = turn
            return
        state.turns.append(turn)

    def _get_turn(self, state: ConversationState, turn_id: str) -> Optional[ConversationTurn]:
        for turn in state.turns:
            if turn.id == turn_id:
                return turn
        return None

    def _publish_progress_payload(
        self,
        progress_callback: Optional[Callable[[dict[str, Any]], None]],
        payload: dict[str, Any],
    ) -> None:
        if progress_callback is None:
            return
        progress_callback(payload)

    def _build_turn_upsert_payload(
        self,
        state: ConversationState,
        turn: ConversationTurn,
    ) -> dict[str, Any]:
        return {
            "type": "turn_upsert",
            "conversation_id": state.conversation_id,
            "project_path": state.project_path,
            "title": state.title,
            "updated_at": state.updated_at,
            "turn": turn.to_dict(),
        }

    def _build_turn_event_payload(
        self,
        state: ConversationState,
        event: ConversationTurnEvent,
    ) -> dict[str, Any]:
        return {
            "type": "turn_event",
            "conversation_id": state.conversation_id,
            "project_path": state.project_path,
            "title": state.title,
            "updated_at": state.updated_at,
            "event": event.to_dict(),
        }

    def _build_chat_prompt(self, state: ConversationState, message: str) -> str:
        recent_turns = state.turns[-10:]
        history_lines = []
        for turn in recent_turns:
            if turn.kind != "message":
                continue
            history_lines.append(f"{turn.role.upper()}: {turn.content}")
        history_text = "\n".join(history_lines) if history_lines else "No prior conversation history."
        return (
            "You are the Sparkspawn project chat assistant. "
            "Reply in plain text only. "
            "Do not wrap your answer in JSON. "
            "Do not create spec proposals or execution cards inline; those are drafted separately by the app when needed. "
            "Keep the reply concise and practical.\n\n"
            f"Project path: {state.project_path}\n"
            f"Recent conversation:\n{history_text}\n\n"
            f"Latest user message:\n{message}\n"
        )

    def _build_spec_proposal_prompt(
        self,
        state: ConversationState,
        message: str,
        assistant_message: str,
    ) -> str:
        recent_turns = state.turns[-10:]
        history_lines = []
        for turn in recent_turns:
            if turn.kind != "message":
                continue
            history_lines.append(f"{turn.role.upper()}: {turn.content}")
        history_text = "\n".join(history_lines) if history_lines else "No prior conversation history."
        return (
            "You are drafting a reviewable Spark Spawn spec edit proposal. "
            "Respond with JSON only. "
            "Schema: "
            "{\"summary\": string, \"changes\": [{\"path\": string, \"before\": string, \"after\": string}]}. "
            "Return only valid JSON with at least one concrete change when a proposal is warranted. "
            "If no proposal is warranted, return {\"summary\":\"\",\"changes\":[]}.\n\n"
            f"Project path: {state.project_path}\n"
            f"Recent conversation:\n{history_text}\n\n"
            f"Latest user message:\n{message}\n\n"
            f"Assistant reply:\n{assistant_message}\n"
        )

    def _build_execution_planning_prompt(
        self,
        state: ConversationState,
        proposal: SpecEditProposal,
        review_feedback: Optional[str],
    ) -> str:
        recent_turns = state.turns[-12:]
        history_lines = []
        for turn in recent_turns:
            if turn.kind != "message":
                continue
            history_lines.append(f"{turn.role.upper()}: {turn.content}")
        history_text = "\n".join(history_lines) if history_lines else "No prior conversation history."
        review_text = review_feedback or "None."
        proposal_payload = json.dumps(proposal.to_dict(), indent=2, sort_keys=True)
        return (
            "You are generating a tracker-ready execution card from an approved spec edit. "
            "Respond with JSON only. "
            "Schema: "
            "{\"title\": string, \"summary\": string, \"objective\": string, "
            "\"work_items\": [{\"id\": string, \"title\": string, \"description\": string, "
            "\"acceptance_criteria\": [string], \"depends_on\": [string]}]}. "
            "Return 1-6 concrete work items. "
            "Do not include markdown fences.\n\n"
            f"Project path: {state.project_path}\n"
            f"Approved spec edit proposal:\n{proposal_payload}\n\n"
            f"Recent conversation:\n{history_text}\n\n"
            f"Latest reviewer feedback for this execution card:\n{review_text}\n"
        )

    def _build_session(self, conversation_id: str, project_path: str) -> CodexAppServerChatSession:
        with self._sessions_lock:
            session = self._sessions.get(conversation_id)
            if session is not None:
                return session
            persisted_session = self._read_session_state(conversation_id)
            session = CodexAppServerChatSession(
                project_path,
                persisted_thread_id=persisted_session.thread_id if persisted_session is not None else None,
                on_thread_id_updated=lambda thread_id: self._persist_session_thread(
                    conversation_id,
                    project_path,
                    thread_id,
                ),
            )
            self._sessions[conversation_id] = session
            return session

    def _replace_session(
        self,
        conversation_id: str,
        project_path: str,
        *,
        persisted_thread_id: Optional[str] = None,
    ) -> CodexAppServerChatSession:
        with self._sessions_lock:
            existing = self._sessions.pop(conversation_id, None)
            if existing is not None:
                existing._close()
            session = CodexAppServerChatSession(
                project_path,
                persisted_thread_id=persisted_thread_id,
                on_thread_id_updated=lambda thread_id: self._persist_session_thread(
                    conversation_id,
                    project_path,
                    thread_id,
                ),
            )
            self._sessions[conversation_id] = session
            return session

    def send_turn(
        self,
        conversation_id: str,
        project_path: str,
        message: str,
        model: Optional[str],
        progress_callback: Optional[Callable[[dict[str, Any]], None]] = None,
        spec_proposal_drafter: Optional[Callable[[str, Optional[str]], str]] = None,
    ) -> dict[str, Any]:
        normalized_project_path = _normalize_project_path(project_path)
        if not normalized_project_path:
            raise ValueError("Project path is required.")
        trimmed_message = message.strip()
        if not trimmed_message:
            raise ValueError("Message is required.")
        session = self._build_session(conversation_id, normalized_project_path)
        with self._lock:
            state = self._read_state(conversation_id)
            if state is None:
                state = ConversationState(conversation_id=conversation_id, project_path=normalized_project_path)
            elif state.project_path != normalized_project_path:
                raise ValueError("Conversation is already bound to a different project path.")
            user_turn = ConversationTurn(
                id=f"turn-{uuid.uuid4().hex}",
                role="user",
                content=trimmed_message,
                timestamp=_iso_now(),
                status="complete",
            )
            assistant_turn = ConversationTurn(
                id=f"turn-{uuid.uuid4().hex}",
                role="assistant",
                content="",
                timestamp=_iso_now(),
                status="pending",
                parent_turn_id=user_turn.id,
            )
            state.turns.append(user_turn)
            state.turns.append(assistant_turn)
            self._touch_conversation_state(state, title_hint=trimmed_message)
            self._write_state(state)
            _log_project_chat_debug(
                "appended user and assistant turns",
                conversation_id=conversation_id,
                project_path=normalized_project_path,
                turns=_summarize_turns_for_debug(state.turns),
            )
            self._publish_progress_payload(progress_callback, self._build_turn_upsert_payload(state, user_turn))
            self._publish_progress_payload(progress_callback, self._build_turn_upsert_payload(state, assistant_turn))

        previous_live_assistant_message = ""
        known_tool_calls: dict[str, ToolCallRecord] = {}

        def persist_progress(progress: ChatTurnResult) -> None:
            nonlocal previous_live_assistant_message, known_tool_calls
            live_assistant_message = _extract_live_assistant_message(progress.assistant_message)
            with self._lock:
                current_state = self._read_state(conversation_id) or state
                current_assistant_turn = self._get_turn(current_state, assistant_turn.id)
                if current_assistant_turn is None:
                    current_assistant_turn = ConversationTurn(
                        id=assistant_turn.id,
                        role="assistant",
                        content="",
                        timestamp=assistant_turn.timestamp,
                        status="pending",
                        parent_turn_id=user_turn.id,
                    )
                    self._upsert_turn(current_state, current_assistant_turn)

                emitted_payloads: list[dict[str, Any]] = []
                if live_assistant_message != previous_live_assistant_message:
                    content_delta = live_assistant_message
                    if live_assistant_message.startswith(previous_live_assistant_message):
                        content_delta = live_assistant_message[len(previous_live_assistant_message):]
                    previous_live_assistant_message = live_assistant_message
                    current_assistant_turn.content = live_assistant_message
                    current_assistant_turn.status = "streaming"
                    self._upsert_turn(current_state, current_assistant_turn)
                    if content_delta:
                        assistant_delta_event = self._append_turn_event(
                            current_state,
                            current_assistant_turn.id,
                            "assistant_delta",
                            content_delta=content_delta,
                        )
                        emitted_payloads.append(self._build_turn_event_payload(current_state, assistant_delta_event))
                    emitted_payloads.append(self._build_turn_upsert_payload(current_state, current_assistant_turn))

                for tool_call in progress.tool_calls:
                    previous_tool_call = known_tool_calls.get(tool_call.id)
                    event_kind: Optional[str] = None
                    if previous_tool_call is None:
                        event_kind = {
                            "running": "tool_call_started",
                            "failed": "tool_call_failed",
                        }.get(tool_call.status, "tool_call_completed")
                    elif tool_call.status != previous_tool_call.status:
                        event_kind = {
                            "failed": "tool_call_failed",
                            "completed": "tool_call_completed",
                        }.get(tool_call.status, "tool_call_updated")
                    elif tool_call.to_dict() != previous_tool_call.to_dict():
                        event_kind = "tool_call_updated"
                    known_tool_calls[tool_call.id] = ToolCallRecord.from_dict(tool_call.to_dict())
                    if event_kind is None:
                        continue
                    tool_event = self._append_turn_event(
                        current_state,
                        current_assistant_turn.id,
                        event_kind,
                        tool_call_id=tool_call.id,
                        tool_call=tool_call,
                    )
                    emitted_payloads.append(self._build_turn_event_payload(current_state, tool_event))

                self._touch_conversation_state(current_state)
                self._write_state(current_state)
                _log_project_chat_debug(
                    "persisted progress events",
                    conversation_id=conversation_id,
                    live_assistant_message=live_assistant_message,
                    tool_call_count=len(progress.tool_calls),
                    turns=_summarize_turns_for_debug(current_state.turns),
                )
            for payload in emitted_payloads:
                self._publish_progress_payload(progress_callback, payload)

        prompt = self._build_chat_prompt(state, trimmed_message)
        normalized_model = model.strip() if model else None
        try:
            turn_result = session.turn(
                prompt,
                normalized_model,
                on_progress=persist_progress,
            )
        except RuntimeError as exc:
            if "timed out" not in str(exc).lower():
                with self._lock:
                    current_state = self._read_state(conversation_id) or state
                    current_assistant_turn = self._get_turn(current_state, assistant_turn.id)
                    if current_assistant_turn is not None:
                        current_assistant_turn.status = "failed"
                        current_assistant_turn.error = str(exc)
                        self._upsert_turn(current_state, current_assistant_turn)
                        assistant_failed_event = self._append_turn_event(
                            current_state,
                            current_assistant_turn.id,
                            "assistant_failed",
                            message=str(exc),
                        )
                        self._touch_conversation_state(current_state)
                        self._write_state(current_state)
                        self._publish_progress_payload(progress_callback, self._build_turn_event_payload(current_state, assistant_failed_event))
                        self._publish_progress_payload(progress_callback, self._build_turn_upsert_payload(current_state, current_assistant_turn))
                raise
            with self._lock:
                current_state = self._read_state(conversation_id) or state
                current_assistant_turn = self._get_turn(current_state, assistant_turn.id)
                retry_event: Optional[ConversationTurnEvent] = None
                if current_assistant_turn is not None:
                    retry_event = self._append_turn_event(
                        current_state,
                        current_assistant_turn.id,
                        "retry_started",
                        message="Retrying turn after timeout.",
                    )
                self._touch_conversation_state(current_state)
                self._write_state(current_state)
                _log_project_chat_debug(
                    "retrying turn after timeout",
                    conversation_id=conversation_id,
                    turns=_summarize_turns_for_debug(current_state.turns),
                )
                if retry_event is not None:
                    self._publish_progress_payload(progress_callback, self._build_turn_event_payload(current_state, retry_event))
            session = self._replace_session(conversation_id, normalized_project_path, persisted_thread_id=None)
            turn_result = session.turn(
                prompt,
                normalized_model,
                on_progress=persist_progress,
            )
        assistant_message, _ = _parse_chat_response_payload(turn_result.assistant_message)
        if not assistant_message:
            assistant_message = "I reviewed that request."
        spec_proposal_payload: Optional[dict[str, Any]] = None
        if spec_proposal_drafter is not None and _should_draft_spec_proposal(trimmed_message):
            try:
                proposal_prompt = self._build_spec_proposal_prompt(state, trimmed_message, assistant_message)
                proposal_raw = spec_proposal_drafter(proposal_prompt, normalized_model)
                parsed_proposal = _extract_json_object(proposal_raw)
                if isinstance(parsed_proposal, dict):
                    spec_proposal_payload = parsed_proposal
            except Exception as exc:
                _log_project_chat_debug(
                    "spec proposal drafting failed",
                    conversation_id=conversation_id,
                    error=str(exc),
                )
        with self._lock:
            state = self._read_state(conversation_id) or state
            current_assistant_turn = self._get_turn(state, assistant_turn.id)
            if current_assistant_turn is None:
                current_assistant_turn = ConversationTurn(
                    id=assistant_turn.id,
                    role="assistant",
                    content="",
                    timestamp=assistant_turn.timestamp,
                    status="pending",
                    parent_turn_id=user_turn.id,
                )
            emitted_payloads: list[dict[str, Any]] = []
            if assistant_message != previous_live_assistant_message:
                content_delta = assistant_message
                if assistant_message.startswith(previous_live_assistant_message):
                    content_delta = assistant_message[len(previous_live_assistant_message):]
                if content_delta:
                    assistant_delta_event = self._append_turn_event(
                        state,
                        current_assistant_turn.id,
                        "assistant_delta",
                        content_delta=content_delta,
                    )
                    emitted_payloads.append(self._build_turn_event_payload(state, assistant_delta_event))
            current_assistant_turn.content = assistant_message
            current_assistant_turn.status = "complete"
            current_assistant_turn.error = None
            self._upsert_turn(state, current_assistant_turn)
            assistant_completed_event = self._append_turn_event(
                state,
                current_assistant_turn.id,
                "assistant_completed",
                message="Assistant turn completed.",
            )
            emitted_payloads.append(self._build_turn_event_payload(state, assistant_completed_event))
            emitted_payloads.append(self._build_turn_upsert_payload(state, current_assistant_turn))
            if isinstance(spec_proposal_payload, dict):
                raw_changes = spec_proposal_payload.get("changes")
                changes = [
                    SpecEditProposalChange.from_dict(change)
                    for change in raw_changes
                    if isinstance(change, dict)
                ] if isinstance(raw_changes, list) else []
                if changes:
                    proposal = SpecEditProposal(
                        id=f"proposal-{uuid.uuid4().hex[:12]}",
                        created_at=_iso_now(),
                        summary=str(spec_proposal_payload.get("summary", "")).strip() or assistant_message,
                        changes=changes,
                        status="pending",
                    )
                    state.spec_edit_proposals.append(proposal)
                    proposal_turn = ConversationTurn(
                        id=f"turn-{uuid.uuid4().hex}",
                        role="system",
                        content="",
                        timestamp=proposal.created_at,
                        status="complete",
                        kind="spec_edit_proposal",
                        artifact_id=proposal.id,
                        parent_turn_id=current_assistant_turn.id,
                    )
                    state.turns.append(proposal_turn)
                    self._append_event(state, f"Drafted spec edit proposal {proposal.id}.")
                    emitted_payloads.append(self._build_turn_upsert_payload(state, proposal_turn))
            self._touch_conversation_state(state)
            self._write_state(state)
            _log_project_chat_debug(
                "persisted final assistant turn",
                conversation_id=conversation_id,
                assistant_message=assistant_message,
                turns=_summarize_turns_for_debug(state.turns),
            )
        for payload in emitted_payloads:
            self._publish_progress_payload(progress_callback, payload)
        return state.to_dict()

    def reject_spec_edit(self, conversation_id: str, project_path: str, proposal_id: str) -> dict[str, Any]:
        normalized_project_path = _normalize_project_path(project_path)
        with self._lock:
            state = self._read_state(conversation_id)
            if state is None or state.project_path != normalized_project_path:
                raise ValueError("Conversation not found for project.")
            proposal = next((entry for entry in state.spec_edit_proposals if entry.id == proposal_id), None)
            if proposal is None:
                raise ValueError("Unknown spec edit proposal.")
            proposal.status = "rejected"
            self._append_event(state, f"Rejected spec edit proposal {proposal.id}.")
            self._touch_conversation_state(state)
            self._write_state(state)
        return state.to_dict()

    def _write_spec_edit_artifact(
        self,
        runtime_project_path: str,
        conversation_id: str,
        proposal: SpecEditProposal,
        canonical_spec_edit_id: str,
    ) -> Path:
        artifact_dir = Path(runtime_project_path) / ".sparkspawn" / "spec-edits"
        artifact_dir.mkdir(parents=True, exist_ok=True)
        artifact_path = artifact_dir / f"{canonical_spec_edit_id}.json"
        payload = {
            "canonical_spec_edit_id": canonical_spec_edit_id,
            "conversation_id": conversation_id,
            "approved_at": proposal.approved_at,
            "summary": proposal.summary,
            "changes": [change.to_dict() for change in proposal.changes],
            "proposal_id": proposal.id,
        }
        artifact_path.write_text(json.dumps(payload, indent=2, sort_keys=True), encoding="utf-8")
        return artifact_path

    def _git_commit_spec_edit(self, runtime_project_path: str, artifact_path: Path, canonical_spec_edit_id: str) -> tuple[str | None, str | None]:
        project_dir = Path(runtime_project_path)
        relative_artifact_path = artifact_path.relative_to(project_dir)
        subprocess.run(
            ["git", "-C", str(project_dir), "add", str(relative_artifact_path)],
            capture_output=True,
            text=True,
            check=True,
        )
        subprocess.run(
            ["git", "-C", str(project_dir), "commit", "-m", f"Approve spec edit {canonical_spec_edit_id}", "--", str(relative_artifact_path)],
            capture_output=True,
            text=True,
            check=True,
        )
        branch = subprocess.run(
            ["git", "-C", str(project_dir), "rev-parse", "--abbrev-ref", "HEAD"],
            capture_output=True,
            text=True,
            check=True,
        ).stdout.strip()
        commit = subprocess.run(
            ["git", "-C", str(project_dir), "rev-parse", "HEAD"],
            capture_output=True,
            text=True,
            check=True,
        ).stdout.strip()
        return branch or None, commit or None

    def approve_spec_edit(
        self,
        conversation_id: str,
        project_path: str,
        proposal_id: str,
    ) -> tuple[dict[str, Any], SpecEditProposal]:
        normalized_project_path = _normalize_project_path(project_path)
        runtime_project_path = resolve_runtime_workspace_path(normalized_project_path)
        with self._lock:
            state = self._read_state(conversation_id)
            if state is None or state.project_path != normalized_project_path:
                raise ValueError("Conversation not found for project.")
            proposal = next((entry for entry in state.spec_edit_proposals if entry.id == proposal_id), None)
            if proposal is None:
                raise ValueError("Unknown spec edit proposal.")
            canonical_spec_edit_id = proposal.canonical_spec_edit_id or f"spec-edit-{_slugify(Path(project_path).name)}-{uuid.uuid4().hex[:8]}"
            proposal.status = "applied"
            proposal.canonical_spec_edit_id = canonical_spec_edit_id
            proposal.approved_at = _iso_now()
        if not Path(runtime_project_path).exists():
            raise ValueError(
                "Project path is not available inside the backend runtime: "
                f"requested {normalized_project_path}, resolved {runtime_project_path}"
            )
        artifact_path = self._write_spec_edit_artifact(runtime_project_path, conversation_id, proposal, canonical_spec_edit_id)
        branch, commit = self._git_commit_spec_edit(runtime_project_path, artifact_path, canonical_spec_edit_id)
        with self._lock:
            state = self._read_state(conversation_id) or state
            proposal = next((entry for entry in state.spec_edit_proposals if entry.id == proposal_id), proposal)
            proposal.status = "applied"
            proposal.canonical_spec_edit_id = canonical_spec_edit_id
            proposal.approved_at = proposal.approved_at or _iso_now()
            proposal.git_branch = branch
            proposal.git_commit = commit
            self._append_event(
                state,
                f"Approved spec edit proposal {proposal.id} as canonical spec edit {canonical_spec_edit_id} and committed it to git.",
            )
            self._touch_conversation_state(state)
            self._write_state(state)
        return state.to_dict(), proposal

    def mark_execution_workflow_started(
        self,
        conversation_id: str,
        workflow_run_id: str,
        flow_source: Optional[str],
    ) -> dict[str, Any]:
        with self._lock:
            state = self._read_state(conversation_id)
            if state is None:
                raise ValueError("Unknown conversation.")
            state.execution_workflow = ExecutionWorkflowState(
                run_id=workflow_run_id,
                status="running",
                error=None,
                flow_source=flow_source,
            )
            if flow_source:
                self._append_event(state, f"Execution planning started ({workflow_run_id}) using {flow_source}.")
            else:
                self._append_event(state, f"Execution planning started ({workflow_run_id}).")
            self._touch_conversation_state(state)
            self._write_state(state)
        return state.to_dict()

    async def run_execution_workflow(
        self,
        conversation_id: str,
        proposal_id: str,
        model: Optional[str],
        flow_source: Optional[str],
        review_feedback: Optional[str],
        workflow_run_id: str,
        codex_runner: Any,
    ) -> None:
        try:
            state = self._read_state(conversation_id)
            if state is None:
                return
            proposal = next((entry for entry in state.spec_edit_proposals if entry.id == proposal_id), None)
            if proposal is None or not proposal.canonical_spec_edit_id:
                raise RuntimeError("Approved spec edit proposal was not found.")
            prompt = self._build_execution_planning_prompt(state, proposal, review_feedback)
            raw_response = await asyncio.to_thread(codex_runner, prompt, model)
            parsed = _extract_json_object(raw_response)
            raw_items = parsed.get("work_items")
            work_items = [
                ExecutionCardWorkItem.from_dict(item)
                for item in raw_items
                if isinstance(item, dict)
            ] if isinstance(raw_items, list) else []
            if not work_items:
                raise RuntimeError("Execution planning returned no work items.")
            now = _iso_now()
            execution_card = ExecutionCard(
                id=f"execution-card-{uuid.uuid4().hex[:12]}",
                title=str(parsed.get("title", "")).strip() or "Execution plan",
                summary=str(parsed.get("summary", "")).strip() or "Generated execution plan.",
                objective=str(parsed.get("objective", "")).strip() or "Implement the approved spec edit.",
                source_spec_edit_id=proposal.canonical_spec_edit_id,
                source_workflow_run_id=workflow_run_id,
                created_at=now,
                updated_at=now,
                status="draft",
                flow_source=flow_source,
                work_items=work_items,
            )
            with self._lock:
                state = self._read_state(conversation_id)
                if state is None:
                    return
                state.execution_cards.append(execution_card)
                state.turns.append(
                    ConversationTurn(
                        id=f"turn-{uuid.uuid4().hex}",
                        role="system",
                        content="",
                        timestamp=now,
                        kind="execution_card",
                        artifact_id=execution_card.id,
                    )
                )
                state.execution_workflow = ExecutionWorkflowState(
                    run_id=workflow_run_id,
                    status="idle",
                    error=None,
                    flow_source=flow_source,
                )
                self._append_event(state, f"Execution planning completed and produced {execution_card.id}.")
                self._touch_conversation_state(state)
                self._write_state(state)
            await self.publish_snapshot(conversation_id)
        except Exception as exc:  # noqa: BLE001
            with self._lock:
                state = self._read_state(conversation_id)
                if state is None:
                    return
                state.execution_workflow = ExecutionWorkflowState(
                    run_id=workflow_run_id,
                    status="failed",
                    error=str(exc),
                    flow_source=flow_source,
                )
                self._append_event(state, f"Execution planning failed: {exc}")
                self._touch_conversation_state(state)
                self._write_state(state)
            await self.publish_snapshot(conversation_id)

    def review_execution_card(
        self,
        conversation_id: str,
        project_path: str,
        execution_card_id: str,
        disposition: str,
        message: str,
        flow_source: Optional[str],
    ) -> tuple[dict[str, Any], ExecutionCard, Optional[str], Optional[str]]:
        normalized_project_path = _normalize_project_path(project_path)
        trimmed_message = message.strip()
        if not trimmed_message:
            raise ValueError("Review message is required.")
        with self._lock:
            state = self._read_state(conversation_id)
            if state is None or state.project_path != normalized_project_path:
                raise ValueError("Conversation not found for project.")
            execution_card = next((entry for entry in state.execution_cards if entry.id == execution_card_id), None)
            if execution_card is None:
                raise ValueError("Unknown execution card.")
            effective_flow_source = flow_source or execution_card.flow_source
            now = _iso_now()
            state.turns.append(
                ConversationTurn(
                    id=f"turn-{uuid.uuid4().hex}",
                    role="user",
                    content=trimmed_message,
                    timestamp=now,
                )
            )
            execution_card.review_feedback.append(
                ExecutionCardReview(
                    id=f"review-{uuid.uuid4().hex[:12]}",
                    disposition=disposition,
                    message=trimmed_message,
                    created_at=now,
                )
            )
            if disposition == "approved":
                execution_card.status = "approved"
                execution_card.updated_at = now
                self._append_event(state, f"Approved execution card {execution_card.id}.")
                workflow_run_id = None
                source_proposal_id = None
            else:
                execution_card.status = "revision-requested" if disposition == "revision_requested" else "rejected"
                execution_card.updated_at = now
                workflow_run_id = f"workflow-{uuid.uuid4().hex[:12]}"
                state.execution_workflow = ExecutionWorkflowState(
                    run_id=workflow_run_id,
                    status="running",
                    error=None,
                    flow_source=effective_flow_source,
                )
                source_proposal_id = next(
                    (
                        proposal.id
                        for proposal in reversed(state.spec_edit_proposals)
                        if proposal.canonical_spec_edit_id == execution_card.source_spec_edit_id
                    ),
                    None,
                )
                self._append_event(
                    state,
                    f"{'Requested revision for' if disposition == 'revision_requested' else 'Rejected'} execution card {execution_card.id}; regenerating with reviewer feedback.",
                )
            self._touch_conversation_state(state, title_hint=trimmed_message)
            self._write_state(state)
        return state.to_dict(), execution_card, source_proposal_id, workflow_run_id
