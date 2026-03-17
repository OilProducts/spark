from __future__ import annotations

import json
from dataclasses import dataclass, field
from typing import Any, Optional


APP_SERVER_TURN_IDLE_TIMEOUT_SECONDS = 60.0


def as_non_empty_string(value: Any) -> Optional[str]:
    if value is None:
        return None
    if isinstance(value, str):
        trimmed = value.strip()
        return trimmed or None
    if isinstance(value, (int, float, bool)):
        return str(value)
    return None


def extract_command_text(payload: dict[str, Any]) -> Optional[str]:
    for key in ("command", "commandLine", "command_line", "cmd", "commandText"):
        value = payload.get(key)
        if isinstance(value, list):
            pieces = [as_non_empty_string(entry) for entry in value]
            command = " ".join(piece for piece in pieces if piece)
            if command:
                return command
        text = as_non_empty_string(value)
        if text:
            return text
    nested = payload.get("command")
    if isinstance(nested, dict):
        return extract_command_text(nested)
    return None


def extract_file_paths(payload: dict[str, Any]) -> list[str]:
    paths: list[str] = []
    for key in ("path", "filePath", "file_path"):
        text = as_non_empty_string(payload.get(key))
        if text:
            paths.append(text)
    for key in ("paths", "files"):
        value = payload.get(key)
        if isinstance(value, list):
            for entry in value:
                if isinstance(entry, dict):
                    nested_path = as_non_empty_string(entry.get("path") or entry.get("filePath") or entry.get("file_path"))
                    if nested_path:
                        paths.append(nested_path)
                        continue
                text = as_non_empty_string(entry)
                if text:
                    paths.append(text)
    changes = payload.get("changes")
    if isinstance(changes, list):
        for change in changes:
            if not isinstance(change, dict):
                continue
            nested_path = as_non_empty_string(change.get("path") or change.get("filePath") or change.get("file_path"))
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


def append_tool_output(existing: Optional[str], delta: str, *, limit: int = 2400) -> str:
    combined = f"{existing or ''}{delta}"
    if len(combined) <= limit:
        return combined
    return combined[-limit:]


def extract_agent_message_text_from_item(item: dict[str, Any]) -> Optional[str]:
    item_type = str(item.get("type") or "").strip().lower()
    if item_type not in {"agentmessage", "agent_message"}:
        return None
    content = item.get("content")
    if isinstance(content, list):
        parts: list[str] = []
        for entry in content:
            if not isinstance(entry, dict):
                continue
            entry_type = str(entry.get("type") or "").strip().lower()
            if entry_type == "text":
                text = entry.get("text")
                if text is not None:
                    parts.append(str(text))
        joined = "".join(parts).strip()
        if joined:
            return joined
    for key in ("text", "message", "contentText", "content_text"):
        value = item.get(key)
        if value is not None and str(value).strip():
            return str(value).strip()
    return None


def extract_agent_message_phase(item: dict[str, Any]) -> Optional[str]:
    phase = item.get("phase")
    if phase is None:
        return None
    normalized = str(phase).strip().lower()
    return normalized or None


def is_final_answer_phase(phase: Optional[str]) -> bool:
    return phase in {None, "", "final_answer", "finalanswer"}


def is_tool_item(item: dict[str, Any]) -> bool:
    item_type = str(item.get("type") or "").strip()
    return item_type in {"commandExecution", "fileChange"}


@dataclass
class CodexAppServerTurnEvent:
    kind: str
    text: Optional[str] = None
    item: Optional[dict[str, Any]] = None
    item_id: Optional[str] = None
    summary_index: Optional[int] = None
    phase: Optional[str] = None
    status: Optional[str] = None
    error: Optional[str] = None


@dataclass
class CodexAppServerTurnState:
    agent_chunks: list[str] = field(default_factory=list)
    command_chunks: list[str] = field(default_factory=list)
    final_agent_message: Optional[str] = None
    last_token_total: Optional[int] = None
    turn_status: Optional[str] = None
    turn_error: Optional[str] = None
    last_error: Optional[str] = None
    saw_final_answer_completion: bool = False
    reasoning_summary_buffer: str = ""
    agent_message_phases: dict[str, str] = field(default_factory=dict)

    def has_terminal_message(self) -> bool:
        response_text = self.final_agent_message if self.final_agent_message is not None else "".join(self.agent_chunks)
        return bool(response_text.strip())

    def can_finalize_without_turn_completed(self) -> bool:
        return self.saw_final_answer_completion and self.has_terminal_message()

    def resolved_agent_text(self) -> str:
        response_text = self.final_agent_message if self.final_agent_message is not None else "".join(self.agent_chunks)
        return response_text.strip()

    def resolved_command_text(self) -> str:
        return "".join(self.command_chunks).strip()


def process_turn_message(message: dict[str, Any], state: CodexAppServerTurnState) -> list[CodexAppServerTurnEvent]:
    method = message.get("method")
    if not method:
        return []
    params = message.get("params") or {}
    events: list[CodexAppServerTurnEvent] = []

    def remember_agent_message_phase(item: dict[str, Any]) -> Optional[str]:
        item_type = str(item.get("type") or "").strip().lower()
        if item_type not in {"agentmessage", "agent_message"}:
            return None
        item_id = as_non_empty_string(item.get("id"))
        phase = extract_agent_message_phase(item)
        if item_id and phase:
            state.agent_message_phases[item_id] = phase
        return phase

    if method == "item/started":
        item = params.get("item")
        if isinstance(item, dict):
            remember_agent_message_phase(item)
            if is_tool_item(item):
                events.append(
                    CodexAppServerTurnEvent(
                        kind="tool_item_started",
                        item=item,
                        item_id=as_non_empty_string(item.get("id")),
                    )
                )
        return events

    if method == "item/completed":
        item = params.get("item")
        if isinstance(item, dict):
            item_id = as_non_empty_string(item.get("id"))
            agent_message_text = extract_agent_message_text_from_item(item)
            if agent_message_text:
                phase = remember_agent_message_phase(item) or extract_agent_message_phase(item)
                state.final_agent_message = agent_message_text
                if is_final_answer_phase(phase):
                    state.saw_final_answer_completion = True
                events.append(
                    CodexAppServerTurnEvent(
                        kind="assistant_message_completed",
                        text=agent_message_text,
                        item=item,
                        item_id=item_id,
                        phase=phase,
                    )
                )
                return events
            if is_tool_item(item):
                events.append(
                    CodexAppServerTurnEvent(
                        kind="tool_item_completed",
                        item=item,
                        item_id=as_non_empty_string(item.get("id")),
                    )
                )
        return events

    if method == "item/agentMessage/delta":
        delta = params.get("delta") or ""
        if delta:
            state.agent_chunks.append(str(delta))
            item_id = as_non_empty_string(params.get("itemId"))
            events.append(
                CodexAppServerTurnEvent(
                    kind="assistant_delta",
                    text=str(delta),
                    item_id=item_id,
                    phase=state.agent_message_phases.get(item_id or ""),
                )
            )
        return events

    if method == "item/reasoning/summaryTextDelta":
        delta = params.get("delta") or ""
        if delta:
            summary_index = params.get("summaryIndex")
            item_id = as_non_empty_string(params.get("itemId"))
            normalized_summary_index = int(summary_index) if isinstance(summary_index, int) else None
            state.reasoning_summary_buffer = f"{state.reasoning_summary_buffer}{delta}"
            events.append(
                CodexAppServerTurnEvent(
                    kind="reasoning_delta",
                    text=str(delta),
                    item_id=item_id,
                    summary_index=normalized_summary_index,
                )
            )
        return events

    if method == "item/reasoning/summaryPartAdded":
        part = params.get("part")
        summary_text: Optional[str] = None
        summary_index = params.get("summaryIndex")
        if isinstance(part, dict):
            for key in ("text", "summaryText", "summary_text"):
                value = part.get(key)
                if value is not None:
                    summary_text = str(value)
                    break
        if summary_text:
            if state.reasoning_summary_buffer and summary_text.startswith(state.reasoning_summary_buffer):
                remaining_summary_text = summary_text[len(state.reasoning_summary_buffer):]
                state.reasoning_summary_buffer = ""
                if remaining_summary_text:
                    events.append(
                        CodexAppServerTurnEvent(
                            kind="reasoning_delta",
                            text=remaining_summary_text,
                            item_id=as_non_empty_string(params.get("itemId")),
                            summary_index=int(summary_index) if isinstance(summary_index, int) else None,
                        )
                    )
            else:
                state.reasoning_summary_buffer = ""
                events.append(
                    CodexAppServerTurnEvent(
                        kind="reasoning_delta",
                        text=summary_text,
                        item_id=as_non_empty_string(params.get("itemId")),
                        summary_index=int(summary_index) if isinstance(summary_index, int) else None,
                    )
                )
        return events

    if method == "item/commandExecution/outputDelta":
        delta = as_non_empty_string(params.get("delta"))
        if delta:
            state.command_chunks.append(delta)
            events.append(
                CodexAppServerTurnEvent(
                    kind="command_output_delta",
                    text=delta,
                    item_id=as_non_empty_string(params.get("itemId")),
                )
            )
        return events

    if method == "thread/tokenUsage/updated":
        token_usage = params.get("tokenUsage") or {}
        total_tokens = (token_usage.get("total") or {}).get("totalTokens")
        if isinstance(total_tokens, int):
            state.last_token_total = total_tokens
        return events

    if method == "error":
        state.turn_status = "failed"
        state.turn_error = str(params.get("message") or "codex app-server error")
        state.last_error = state.turn_error
        events.append(CodexAppServerTurnEvent(kind="error", error=state.turn_error))
        return events

    if method == "turn/completed":
        turn = params.get("turn") or {}
        status = str(turn.get("status") or "")
        state.turn_status = status or None
        if status and status != "completed":
            error = turn.get("error") or {}
            state.turn_error = str(error.get("message") or state.turn_error or f"turn ended with status '{status}'")
            state.last_error = state.turn_error
        events.append(
            CodexAppServerTurnEvent(
                kind="turn_completed",
                status=state.turn_status,
                error=state.turn_error,
            )
        )
        return events
    return events


def parse_jsonrpc_line(line: str) -> Optional[dict[str, Any]]:
    try:
        parsed = json.loads(line)
    except json.JSONDecodeError:
        return None
    if not isinstance(parsed, dict):
        return None
    return parsed
