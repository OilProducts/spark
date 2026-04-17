from __future__ import annotations

import json
import logging
import re
import threading
import uuid
from pathlib import Path
from typing import Any, Callable, Optional

from spark.authoring_assets import (
    dot_authoring_guide_path,
    spark_operations_guide_path,
)
from spark.chat.prompt_templates import (
    load_prompt_templates,
    render_chat_prompt,
    render_plan_prompt,
)
from spark.chat.response_parsing import (
    log_project_chat_debug as _log_project_chat_debug,
    normalize_flow_run_request_payload as _normalize_flow_run_request_payload,
    parse_chat_response_payload as _parse_chat_response_payload,
    summarize_turns_for_debug as _summarize_turns_for_debug,
)
from spark.chat.session import (
    CodexAppServerChatSession,
)
from spark.workspace.conversations.artifacts import ProjectChatReviewService
from spark.workspace.conversations.models import (
    ChatTurnLiveEvent,
    ChatTurnResult,
    ConversationSegment,
    ConversationSegmentSource,
    ConversationEventHub,
    ConversationSessionState,
    ConversationState,
    ConversationTurn,
    FlowRunRequest,
    PreparedChatTurn,
    ToolCallRecord,
    normalize_chat_mode,
    TURN_KIND_MODE_CHANGE,
    validate_chat_mode,
)
from spark.workspace.conversations.repository import ProjectChatRepository
from spark.workspace.conversations.utils import (
    as_non_empty_string as _as_non_empty_string,
    iso_now as _iso_now,
    normalize_project_path_value as _normalize_project_path,
)
from spark.workspace.storage import ProjectPaths


CHAT_RUNTIME_THREAD_KEY = "_attractor.runtime.thread_id"
LOGGER = logging.getLogger(__name__)
PROPOSED_PLAN_BLOCK_PATTERN = re.compile(r"(?is)<proposed_plan>\s*(.*?)\s*</proposed_plan>")
EXCESS_BLANK_LINES_PATTERN = re.compile(r"\n{3,}")


def _resolve_flow_validation_command() -> str:
    return "spark flow validate --file <path> --text"


class TurnInProgressError(RuntimeError):
    """Raised when a conversation already has an active assistant turn."""


def _normalize_assistant_phase(value: Any) -> Optional[str]:
    if value is None:
        return None
    normalized = str(value).strip().lower()
    if not normalized:
        return None
    if normalized in {"finalanswer", "final_answer"}:
        return "final_answer"
    if normalized == "commentary":
        return "commentary"
    return normalized


def _normalize_assistant_text(value: str) -> str:
    normalized = value.replace("\r\n", "\n").strip()
    if not normalized:
        return ""
    return EXCESS_BLANK_LINES_PATTERN.sub("\n\n", normalized)


def _render_proposed_plan_markup(value: str) -> str:
    return PROPOSED_PLAN_BLOCK_PATTERN.sub(lambda match: (match.group(1) or "").strip(), value)


def _remove_standalone_text_block(text: str, block: str) -> str:
    candidate = _normalize_assistant_text(text)
    target = _normalize_assistant_text(block)
    if not candidate or not target:
        return candidate
    if candidate == target:
        return ""
    pattern = re.compile(rf"(?s)(?:\A|\n\s*\n){re.escape(target)}(?:\n\s*\n|\Z)")
    match = pattern.search(candidate)
    if match is None:
        return candidate
    prefix = _normalize_assistant_text(candidate[: match.start()])
    suffix = _normalize_assistant_text(candidate[match.end() :])
    return "\n\n".join(part for part in (prefix, suffix) if part)


def _extract_plan_mode_assistant_remainder(text: str, plan_text: Optional[str]) -> str:
    normalized_plan = _normalize_assistant_text(plan_text or "")
    rendered_text = _normalize_assistant_text(_render_proposed_plan_markup(text))
    if not normalized_plan:
        return rendered_text
    without_plan_blocks = _normalize_assistant_text(PROPOSED_PLAN_BLOCK_PATTERN.sub("\n\n", text))
    remainder = _remove_standalone_text_block(without_plan_blocks, normalized_plan)
    if remainder:
        return remainder
    return _remove_standalone_text_block(rendered_text, normalized_plan)


def _build_mode_change_turn(chat_mode: str) -> ConversationTurn:
    return ConversationTurn(
        id=f"turn-{uuid.uuid4().hex}",
        role="system",
        content=chat_mode,
        timestamp=_iso_now(),
        status="complete",
        kind=TURN_KIND_MODE_CHANGE,
    )


class ProjectChatService:
    def __init__(
        self,
        data_dir: Path,
        *,
        flows_dir: Path | None = None,
        authoring_guide_path: Path | None = None,
    ) -> None:
        self._data_dir = data_dir
        self._flows_dir = (flows_dir or (data_dir / "flows")).expanduser().resolve(strict=False)
        self._authoring_guide_path = (authoring_guide_path or dot_authoring_guide_path()).expanduser().resolve(strict=False)
        self._operations_guide_path = spark_operations_guide_path().expanduser().resolve(strict=False)
        self._prompt_templates = load_prompt_templates(data_dir / "config")
        self._lock = threading.Lock()
        self._repository = ProjectChatRepository(data_dir, self._lock)
        self._reviews = ProjectChatReviewService(self._repository)
        self._event_hub = ConversationEventHub()
        self._sessions_lock = threading.Lock()
        self._sessions: dict[str, CodexAppServerChatSession] = {}

    def events(self) -> ConversationEventHub:
        return self._event_hub

    def _projects_root(self) -> Path:
        return self._repository.projects_root()

    def _project_paths(self, project_path: str) -> ProjectPaths:
        return self._repository.project_paths(project_path)

    def _project_paths_for_conversation(
        self,
        conversation_id: str,
        project_path: Optional[str] = None,
    ) -> ProjectPaths:
        return self._repository.project_paths_for_conversation(conversation_id, project_path)

    def _conversation_root(self, conversation_id: str, project_path: Optional[str] = None) -> Path:
        return self._repository.conversation_root(conversation_id, project_path)

    def _conversation_state_path(self, conversation_id: str, project_path: Optional[str] = None) -> Path:
        return self._repository.conversation_state_path(conversation_id, project_path)

    def _conversation_session_path(self, conversation_id: str, project_path: Optional[str] = None) -> Path:
        return self._repository.conversation_session_path(conversation_id, project_path)

    def _conversation_raw_log_path(self, conversation_id: str, project_path: Optional[str] = None) -> Path:
        return self._repository.conversation_raw_log_path(conversation_id, project_path)

    def _flow_run_requests_state_path(self, conversation_id: str, project_path: Optional[str] = None) -> Path:
        return self._repository.flow_run_requests_state_path(conversation_id, project_path)

    def _touch_conversation_state(self, state: ConversationState, *, title_hint: Optional[str] = None) -> None:
        self._repository.touch_conversation_state(state, title_hint=title_hint)

    def _read_state(self, conversation_id: str, project_path: Optional[str] = None) -> Optional[ConversationState]:
        return self._repository.read_state(conversation_id, project_path)

    def _write_state(self, state: ConversationState) -> None:
        self._repository.write_state(state)

    def _read_json_dict(self, path: Path) -> dict[str, Any]:
        return self._repository.read_json_dict(path)

    def _write_json(self, path: Path, payload: dict[str, Any]) -> None:
        self._repository.write_json(path, payload)

    def _append_raw_rpc_log(
        self,
        conversation_id: str,
        project_path: str,
        *,
        direction: str,
        line: str,
    ) -> None:
        self._repository.append_raw_rpc_log(
            conversation_id,
            project_path,
            direction=direction,
            line=line,
        )

    def _read_session_state(self, conversation_id: str, project_path: Optional[str] = None) -> Optional[ConversationSessionState]:
        return self._repository.read_session_state(conversation_id, project_path)

    def _write_session_state(self, state: ConversationSessionState) -> None:
        self._repository.write_session_state(state)

    def _persist_session_thread(
        self,
        conversation_id: str,
        project_path: str,
        thread_id: str,
    ) -> None:
        self._repository.persist_session_thread(conversation_id, project_path, thread_id)

    def _persist_session_model(
        self,
        conversation_id: str,
        project_path: str,
        model: str,
    ) -> None:
        self._repository.persist_session_model(conversation_id, project_path, model)

    async def publish_snapshot(self, conversation_id: str) -> None:
        snapshot = self.get_snapshot(conversation_id)
        await self._event_hub.publish(conversation_id, {"type": "conversation_snapshot", "state": snapshot})

    def list_conversations(self, project_path: str) -> list[dict[str, Any]]:
        return self._repository.list_conversations(project_path)

    def get_snapshot(self, conversation_id: str, project_path: Optional[str] = None) -> dict[str, Any]:
        return self._repository.get_snapshot(conversation_id, project_path)

    def get_snapshot_by_handle(self, conversation_handle: str) -> dict[str, Any]:
        conversation_id, project_path = self._repository.resolve_conversation_handle(conversation_handle)
        return self.get_snapshot(conversation_id, project_path)

    def delete_conversation(self, conversation_id: str, project_path: str) -> dict[str, Any]:
        snapshot = self._repository.delete_conversation(conversation_id, project_path)
        with self._sessions_lock:
            session = self._sessions.pop(conversation_id, None)
        if session is not None:
            session.close()
        return snapshot

    def _append_event(self, state: ConversationState, message: str) -> None:
        self._repository.append_event(state, message)

    def _next_turn_segment_order(self, state: ConversationState, turn_id: str) -> int:
        return self._repository.next_turn_segment_order(state, turn_id)

    def _upsert_segment(self, state: ConversationState, segment: ConversationSegment) -> None:
        self._repository.upsert_segment(state, segment)

    def _get_segment(self, state: ConversationState, segment_id: str) -> Optional[ConversationSegment]:
        return self._repository.get_segment(state, segment_id)

    def _build_segment_source(
        self,
        event: ChatTurnLiveEvent,
        *,
        tool_call_id: Optional[str] = None,
    ) -> ConversationSegmentSource:
        return ConversationSegmentSource(
            app_turn_id=event.app_turn_id,
            item_id=event.item_id,
            summary_index=event.summary_index,
            call_id=tool_call_id or event.tool_call_id,
        )

    def _build_reasoning_segment_id(self, turn_id: str, event: ChatTurnLiveEvent) -> str:
        app_turn_id = event.app_turn_id or turn_id
        item_id = event.item_id or "reasoning"
        summary_index = event.summary_index if event.summary_index is not None else 0
        return f"segment-reasoning-{app_turn_id}-{item_id}-{summary_index}"

    def _build_assistant_segment_id(self, turn_id: str, event: ChatTurnLiveEvent) -> str:
        if event.app_turn_id and event.item_id:
            return f"segment-assistant-{event.app_turn_id}-{event.item_id}"
        return f"segment-assistant-{turn_id}"

    def _build_plan_segment_id(self, turn_id: str, event: ChatTurnLiveEvent) -> str:
        if event.app_turn_id and event.item_id:
            return f"segment-plan-{event.app_turn_id}-{event.item_id}"
        return f"segment-plan-{turn_id}"

    def _build_tool_segment_id(self, turn_id: str, event: ChatTurnLiveEvent) -> str:
        app_turn_id = event.app_turn_id or turn_id
        call_id = event.tool_call_id or event.item_id or "tool"
        return f"segment-tool-{app_turn_id}-{call_id}"

    def _build_context_compaction_segment_id(self, turn_id: str, event: ChatTurnLiveEvent) -> str:
        app_turn_id = event.app_turn_id or turn_id
        return f"segment-context-compaction-{app_turn_id}"

    def _assistant_segment_phase(self, event: ChatTurnLiveEvent, segment: Optional[ConversationSegment] = None) -> Optional[str]:
        if event.phase is not None:
            return _normalize_assistant_phase(event.phase)
        if segment is not None:
            return _normalize_assistant_phase(segment.phase)
        return None

    def _is_final_answer_phase(self, phase: Optional[str]) -> bool:
        normalized = _normalize_assistant_phase(phase)
        return normalized in {None, "final_answer"}

    def _completed_final_answer_segment(self, state: ConversationState, turn_id: str) -> Optional[ConversationSegment]:
        for segment in state.segments:
            if segment.turn_id != turn_id or segment.kind != "assistant_message":
                continue
            if segment.status != "complete":
                continue
            if self._is_final_answer_phase(segment.phase):
                return segment
        return None

    def _completed_plan_segment(self, state: ConversationState, turn_id: str) -> Optional[ConversationSegment]:
        for segment in state.segments:
            if segment.turn_id != turn_id or segment.kind != "plan":
                continue
            if segment.status == "complete" and _as_non_empty_string(segment.content):
                return segment
        return None

    def _materialize_segment_for_live_event(
        self,
        state: ConversationState,
        turn: ConversationTurn,
        event: ChatTurnLiveEvent,
    ) -> Optional[ConversationSegment]:
        timestamp = _iso_now()
        if event.kind == "reasoning_summary":
            segment_id = self._build_reasoning_segment_id(turn.id, event)
            segment = self._get_segment(state, segment_id) or ConversationSegment(
                id=segment_id,
                turn_id=turn.id,
                order=self._next_turn_segment_order(state, turn.id),
                kind="reasoning",
                role="assistant",
                status="streaming",
                timestamp=timestamp,
                updated_at=timestamp,
                source=self._build_segment_source(event),
            )
            segment.content = f"{segment.content}{event.content_delta or ''}"
            segment.status = "streaming"
            segment.updated_at = timestamp
            self._upsert_segment(state, segment)
            return segment
        if event.kind == "assistant_delta":
            segment_id = self._build_assistant_segment_id(turn.id, event)
            segment = self._get_segment(state, segment_id) or ConversationSegment(
                id=segment_id,
                turn_id=turn.id,
                order=self._next_turn_segment_order(state, turn.id),
                kind="assistant_message",
                role="assistant",
                status="streaming",
                timestamp=timestamp,
                updated_at=timestamp,
                phase=self._assistant_segment_phase(event),
                source=self._build_segment_source(event),
            )
            segment.content = f"{segment.content}{event.content_delta or ''}"
            segment.status = "streaming"
            segment.updated_at = timestamp
            segment.error = None
            segment.phase = self._assistant_segment_phase(event, segment)
            self._upsert_segment(state, segment)
            return segment
        if event.kind == "plan_delta":
            segment_id = self._build_plan_segment_id(turn.id, event)
            segment = self._get_segment(state, segment_id) or ConversationSegment(
                id=segment_id,
                turn_id=turn.id,
                order=self._next_turn_segment_order(state, turn.id),
                kind="plan",
                role="assistant",
                status="streaming",
                timestamp=timestamp,
                updated_at=timestamp,
                source=self._build_segment_source(event),
            )
            segment.content = f"{segment.content}{event.content_delta or ''}"
            segment.status = "streaming"
            segment.updated_at = timestamp
            segment.error = None
            self._upsert_segment(state, segment)
            return segment
        if event.kind in {"context_compaction_started", "context_compaction_completed"}:
            segment_id = self._build_context_compaction_segment_id(turn.id, event)
            complete = event.kind == "context_compaction_completed"
            target_status = "complete" if complete else "running"
            target_content = (
                "Context compacted to continue the turn."
                if complete
                else "Compacting conversation context…"
            )
            segment = self._get_segment(state, segment_id)
            if segment is None:
                segment = ConversationSegment(
                    id=segment_id,
                    turn_id=turn.id,
                    order=self._next_turn_segment_order(state, turn.id),
                    kind="context_compaction",
                    role="system",
                    status=target_status,
                    timestamp=timestamp,
                    updated_at=timestamp,
                    content=target_content,
                    completed_at=timestamp if complete else None,
                    source=self._build_segment_source(event),
                )
                self._upsert_segment(state, segment)
                return segment
            if segment.source.app_turn_id is None and event.app_turn_id is not None:
                segment.source.app_turn_id = event.app_turn_id
            if event.item_id is not None:
                segment.source.item_id = event.item_id
            if segment.status == target_status and segment.content == target_content:
                return None
            if segment.status == "complete" and not complete:
                return None
            segment.status = target_status
            segment.content = target_content
            segment.updated_at = timestamp
            segment.error = None
            segment.completed_at = timestamp if complete else None
            self._upsert_segment(state, segment)
            return segment
        if event.kind in {"tool_call_started", "tool_call_updated", "tool_call_completed", "tool_call_failed"} and event.tool_call is not None:
            segment_id = self._build_tool_segment_id(turn.id, event)
            status = event.tool_call.status
            segment = self._get_segment(state, segment_id) or ConversationSegment(
                id=segment_id,
                turn_id=turn.id,
                order=self._next_turn_segment_order(state, turn.id),
                kind="tool_call",
                role="system",
                status=status,
                timestamp=timestamp,
                updated_at=timestamp,
                tool_call=ToolCallRecord.from_dict(event.tool_call.to_dict()),
                source=self._build_segment_source(event, tool_call_id=event.tool_call.id),
            )
            segment.status = status
            segment.updated_at = timestamp
            segment.tool_call = ToolCallRecord.from_dict(event.tool_call.to_dict())
            if status != "running":
                segment.completed_at = timestamp
            self._upsert_segment(state, segment)
            return segment
        if event.kind == "assistant_completed":
            segment_id = self._build_assistant_segment_id(turn.id, event)
            segment = self._get_segment(state, segment_id) or ConversationSegment(
                id=segment_id,
                turn_id=turn.id,
                order=self._next_turn_segment_order(state, turn.id),
                kind="assistant_message",
                role="assistant",
                status="complete",
                timestamp=timestamp,
                updated_at=timestamp,
                phase=self._assistant_segment_phase(event),
                source=self._build_segment_source(event),
            )
            if event.content_delta:
                segment.content = event.content_delta
            elif turn.content:
                segment.content = turn.content
            segment.status = "complete"
            segment.updated_at = timestamp
            segment.completed_at = timestamp
            segment.error = None
            segment.phase = self._assistant_segment_phase(event, segment)
            self._upsert_segment(state, segment)
            return segment
        if event.kind == "plan_completed":
            segment_id = self._build_plan_segment_id(turn.id, event)
            segment = self._get_segment(state, segment_id) or ConversationSegment(
                id=segment_id,
                turn_id=turn.id,
                order=self._next_turn_segment_order(state, turn.id),
                kind="plan",
                role="assistant",
                status="complete",
                timestamp=timestamp,
                updated_at=timestamp,
                source=self._build_segment_source(event),
            )
            if event.content_delta:
                segment.content = event.content_delta
            segment.status = "complete"
            segment.updated_at = timestamp
            segment.completed_at = timestamp
            segment.error = None
            self._upsert_segment(state, segment)
            return segment
        if event.kind == "assistant_failed":
            segment_id = self._build_assistant_segment_id(turn.id, event)
            segment = self._get_segment(state, segment_id) or ConversationSegment(
                id=segment_id,
                turn_id=turn.id,
                order=self._next_turn_segment_order(state, turn.id),
                kind="assistant_message",
                role="assistant",
                status="failed",
                timestamp=timestamp,
                updated_at=timestamp,
                phase=self._assistant_segment_phase(event),
                source=self._build_segment_source(event),
            )
            segment.content = event.content_delta or turn.content or event.message or segment.content
            segment.status = "failed"
            segment.error = turn.error or event.message
            segment.updated_at = timestamp
            segment.completed_at = timestamp
            segment.phase = self._assistant_segment_phase(event, segment)
            self._upsert_segment(state, segment)
            return segment
        return None

    def _upsert_turn(self, state: ConversationState, turn: ConversationTurn) -> None:
        self._repository.upsert_turn(state, turn)

    def _get_turn(self, state: ConversationState, turn_id: str) -> Optional[ConversationTurn]:
        return self._repository.get_turn(state, turn_id)

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
        return self._repository.build_turn_upsert_payload(state, turn)

    def _build_segment_upsert_payload(
        self,
        state: ConversationState,
        segment: ConversationSegment,
    ) -> dict[str, Any]:
        return self._repository.build_segment_upsert_payload(state, segment)

    def _build_prompt_values(self, state: ConversationState, message: str) -> dict[str, str]:
        return {
            "conversation_handle": state.conversation_handle,
            "project_path": state.project_path,
            "flow_library_path": str(self._flows_dir),
            "dot_authoring_guide_path": str(self._authoring_guide_path),
            "spark_operations_guide_path": str(self._operations_guide_path),
            "flow_validation_command": _resolve_flow_validation_command(),
            "latest_user_message": message,
        }

    def _build_chat_prompt(self, state: ConversationState, message: str) -> str:
        return render_chat_prompt(
            self._prompt_templates.chat,
            self._build_prompt_values(state, message),
        )

    def _build_plan_prompt(self, state: ConversationState, message: str) -> str:
        return render_plan_prompt(
            self._prompt_templates.plan,
            self._build_prompt_values(state, message),
        )

    def _build_prompt(self, state: ConversationState, message: str, chat_mode: str) -> str:
        if normalize_chat_mode(chat_mode) == "plan":
            return self._build_plan_prompt(state, message)
        return self._build_chat_prompt(state, message)

    def update_conversation_settings(
        self,
        conversation_id: str,
        project_path: str,
        chat_mode: str,
    ) -> dict[str, Any]:
        normalized_project_path = _normalize_project_path(project_path)
        if not normalized_project_path:
            raise ValueError("Project path is required.")
        normalized_chat_mode = validate_chat_mode(chat_mode)
        with self._lock:
            state = self._read_state(conversation_id)
            if state is None:
                state = ConversationState(
                    conversation_id=conversation_id,
                    project_path=normalized_project_path,
                )
            elif state.project_path != normalized_project_path:
                raise ValueError("Conversation is already bound to a different project path.")
            current_chat_mode = normalize_chat_mode(state.chat_mode)
            if normalized_chat_mode != current_chat_mode:
                state.turns.append(_build_mode_change_turn(normalized_chat_mode))
            state.chat_mode = normalized_chat_mode
            self._touch_conversation_state(state)
            self._write_state(state)
            return state.to_dict()

    def _prepare_turn(
        self,
        conversation_id: str,
        project_path: str,
        message: str,
        model: Optional[str],
        chat_mode: Optional[str] = None,
        progress_callback: Optional[Callable[[dict[str, Any]], None]] = None,
    ) -> tuple[PreparedChatTurn, dict[str, Any]]:
        normalized_project_path = _normalize_project_path(project_path)
        if not normalized_project_path:
            raise ValueError("Project path is required.")
        trimmed_message = message.strip()
        if not trimmed_message:
            raise ValueError("Message is required.")
        with self._lock:
            state = self._read_state(conversation_id)
            if state is None:
                state = ConversationState(conversation_id=conversation_id, project_path=normalized_project_path)
            elif state.project_path != normalized_project_path:
                raise ValueError("Conversation is already bound to a different project path.")
            current_chat_mode = normalize_chat_mode(state.chat_mode)
            effective_chat_mode = validate_chat_mode(chat_mode) if chat_mode is not None else current_chat_mode
            mode_change_turn: ConversationTurn | None = None
            if effective_chat_mode != current_chat_mode:
                mode_change_turn = _build_mode_change_turn(effective_chat_mode)
                state.turns.append(mode_change_turn)
            state.chat_mode = effective_chat_mode
            active_assistant_turn = next(
                (
                    turn
                    for turn in state.turns
                    if turn.role == "assistant" and turn.status in {"pending", "streaming"}
                ),
                None,
            )
            if active_assistant_turn is not None:
                raise TurnInProgressError(
                    "An assistant turn is still in progress for this conversation. Wait for it to finish before sending another message."
                )
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
            prompt = self._build_prompt(state, trimmed_message, effective_chat_mode)
            self._write_state(state)
            snapshot = state.to_dict()
            _log_project_chat_debug(
                "appended user and assistant turns",
                conversation_id=conversation_id,
                project_path=normalized_project_path,
                turns=_summarize_turns_for_debug(state.turns),
            )
            if mode_change_turn is not None:
                self._publish_progress_payload(progress_callback, self._build_turn_upsert_payload(state, mode_change_turn))
            self._publish_progress_payload(progress_callback, self._build_turn_upsert_payload(state, user_turn))
            self._publish_progress_payload(progress_callback, self._build_turn_upsert_payload(state, assistant_turn))
        normalized_model = model.strip() if model else None
        return (
            PreparedChatTurn(
                conversation_id=conversation_id,
                project_path=normalized_project_path,
                chat_mode=effective_chat_mode,
                prompt=prompt,
                model=normalized_model,
                user_turn=user_turn,
                assistant_turn=assistant_turn,
            ),
            snapshot,
        )

    def _persist_assistant_turn_failure(
        self,
        prepared: PreparedChatTurn,
        error_message: str,
        progress_callback: Optional[Callable[[dict[str, Any]], None]] = None,
    ) -> None:
        with self._lock:
            current_state = self._read_state(prepared.conversation_id, prepared.project_path)
            if current_state is None:
                return
            current_assistant_turn = self._get_turn(current_state, prepared.assistant_turn.id)
            if current_assistant_turn is None:
                current_assistant_turn = ConversationTurn(
                    id=prepared.assistant_turn.id,
                    role="assistant",
                    content="",
                    timestamp=prepared.assistant_turn.timestamp,
                    status="pending",
                    parent_turn_id=prepared.user_turn.id,
                )
            current_assistant_turn.status = "failed"
            current_assistant_turn.error = error_message
            self._upsert_turn(current_state, current_assistant_turn)
            emitted_payloads: list[dict[str, Any]] = []
            for segment in current_state.segments:
                if segment.turn_id != current_assistant_turn.id or segment.kind not in {"assistant_message", "plan"}:
                    continue
                segment.status = "failed"
                segment.error = error_message
                segment.updated_at = _iso_now()
                segment.completed_at = segment.updated_at
                self._upsert_segment(current_state, segment)
                emitted_payloads.append(self._build_segment_upsert_payload(current_state, segment))
            self._touch_conversation_state(current_state)
            self._write_state(current_state)
            assistant_upsert_payload = self._build_turn_upsert_payload(current_state, current_assistant_turn)
        for payload in emitted_payloads:
            self._publish_progress_payload(progress_callback, payload)
        self._publish_progress_payload(progress_callback, assistant_upsert_payload)

    def _execute_turn(
        self,
        prepared: PreparedChatTurn,
        persist_live_event: Callable[[ChatTurnLiveEvent], None],
        progress_callback: Optional[Callable[[dict[str, Any]], None]] = None,
    ) -> ChatTurnResult:
        def bind_raw_rpc_logger(target_session: Any) -> None:
            if hasattr(target_session, "set_raw_rpc_logger"):
                target_session.set_raw_rpc_logger(
                    lambda direction, line: self._append_raw_rpc_log(
                        prepared.conversation_id,
                        prepared.project_path,
                        direction=direction,
                        line=line,
                    )
                )

        def clear_raw_rpc_logger(target_session: Any) -> None:
            if hasattr(target_session, "clear_raw_rpc_logger"):
                target_session.clear_raw_rpc_logger()

        def run_session(target_session: Any) -> ChatTurnResult:
            bind_raw_rpc_logger(target_session)
            try:
                return target_session.turn(
                    prepared.prompt,
                    prepared.model,
                    chat_mode=prepared.chat_mode,
                    on_event=persist_live_event,
                )
            finally:
                clear_raw_rpc_logger(target_session)

        session = self._build_session(prepared.conversation_id, prepared.project_path)
        return run_session(session)

    def _run_prepared_turn(
        self,
        prepared: PreparedChatTurn,
        progress_callback: Optional[Callable[[dict[str, Any]], None]] = None,
    ) -> dict[str, Any]:
        with self._lock:
            state = self._read_state(prepared.conversation_id, prepared.project_path)
            if state is None:
                raise RuntimeError("Conversation state disappeared before the turn started.")

        buffered_plan_assistant_event: ChatTurnLiveEvent | None = None

        def persist_live_event(event: ChatTurnLiveEvent) -> None:
            nonlocal buffered_plan_assistant_event
            with self._lock:
                current_state = self._read_state(prepared.conversation_id, prepared.project_path) or state
                current_assistant_turn = self._get_turn(current_state, prepared.assistant_turn.id)
                if current_assistant_turn is None:
                    current_assistant_turn = ConversationTurn(
                        id=prepared.assistant_turn.id,
                        role="assistant",
                        content="",
                        timestamp=prepared.assistant_turn.timestamp,
                        status="pending",
                        parent_turn_id=prepared.user_turn.id,
                    )
                    self._upsert_turn(current_state, current_assistant_turn)

                emitted_payloads: list[dict[str, Any]] = []
                if current_assistant_turn.status == "pending":
                    current_assistant_turn.status = "streaming"
                    self._upsert_turn(current_state, current_assistant_turn)
                    emitted_payloads.append(self._build_turn_upsert_payload(current_state, current_assistant_turn))

                if event.kind == "assistant_delta":
                    if event.content_delta and prepared.chat_mode != "plan":
                        segment = self._materialize_segment_for_live_event(current_state, current_assistant_turn, event)
                        if segment is not None:
                            emitted_payloads.append(self._build_segment_upsert_payload(current_state, segment))
                elif event.kind == "reasoning_summary":
                    if event.content_delta:
                        segment = self._materialize_segment_for_live_event(current_state, current_assistant_turn, event)
                        if segment is not None:
                            emitted_payloads.append(self._build_segment_upsert_payload(current_state, segment))
                elif event.kind == "plan_delta":
                    if event.content_delta:
                        segment = self._materialize_segment_for_live_event(current_state, current_assistant_turn, event)
                        if segment is not None:
                            emitted_payloads.append(self._build_segment_upsert_payload(current_state, segment))
                elif event.kind == "plan_completed":
                    segment = self._materialize_segment_for_live_event(current_state, current_assistant_turn, event)
                    if segment is not None:
                        emitted_payloads.append(self._build_segment_upsert_payload(current_state, segment))
                elif event.kind in {"context_compaction_started", "context_compaction_completed"}:
                    segment = self._materialize_segment_for_live_event(current_state, current_assistant_turn, event)
                    if segment is not None:
                        emitted_payloads.append(self._build_segment_upsert_payload(current_state, segment))
                elif event.kind == "assistant_completed":
                    assistant_message = _as_non_empty_string(event.content_delta)
                    current_assistant_turn.status = "streaming"
                    current_assistant_turn.error = None
                    self._upsert_turn(current_state, current_assistant_turn)
                    if prepared.chat_mode == "plan":
                        if assistant_message and self._is_final_answer_phase(event.phase):
                            buffered_plan_assistant_event = ChatTurnLiveEvent(
                                kind=event.kind,
                                content_delta=assistant_message,
                                message=event.message,
                                app_turn_id=event.app_turn_id,
                                item_id=event.item_id,
                                phase=event.phase,
                            )
                    else:
                        if assistant_message and self._is_final_answer_phase(event.phase):
                            current_assistant_turn.content = assistant_message
                            self._upsert_turn(current_state, current_assistant_turn)
                        segment = self._materialize_segment_for_live_event(current_state, current_assistant_turn, event)
                        emitted_payloads.append(self._build_turn_upsert_payload(current_state, current_assistant_turn))
                        if segment is not None:
                            emitted_payloads.append(self._build_segment_upsert_payload(current_state, segment))
                elif event.kind in {"tool_call_started", "tool_call_updated", "tool_call_completed", "tool_call_failed"} and event.tool_call is not None:
                    segment = self._materialize_segment_for_live_event(current_state, current_assistant_turn, event)
                    if segment is not None:
                        emitted_payloads.append(self._build_segment_upsert_payload(current_state, segment))
                else:
                    return

                self._touch_conversation_state(current_state)
                self._write_state(current_state)
                _log_project_chat_debug(
                    "persisted progress events",
                    conversation_id=prepared.conversation_id,
                    event_kind=event.kind,
                    turns=_summarize_turns_for_debug(current_state.turns),
                )
            for payload in emitted_payloads:
                self._publish_progress_payload(progress_callback, payload)

        try:
            turn_result = self._execute_turn(prepared, persist_live_event, progress_callback)
        except RuntimeError as exc:
            self._persist_assistant_turn_failure(prepared, str(exc), progress_callback)
            raise

        assistant_message, _ = _parse_chat_response_payload(turn_result.assistant_message)
        with self._lock:
            state = self._read_state(prepared.conversation_id, prepared.project_path) or state
            current_assistant_turn = self._get_turn(state, prepared.assistant_turn.id)
            if current_assistant_turn is None:
                current_assistant_turn = ConversationTurn(
                    id=prepared.assistant_turn.id,
                    role="assistant",
                    content="",
                    timestamp=prepared.assistant_turn.timestamp,
                    status="pending",
                    parent_turn_id=prepared.user_turn.id,
                )
            plan_segment = self._completed_plan_segment(state, current_assistant_turn.id)
            emitted_payloads = []
            assistant_display_text = assistant_message
            if prepared.chat_mode == "plan":
                assistant_display_text = _extract_plan_mode_assistant_remainder(
                    assistant_message,
                    plan_segment.content if plan_segment is not None else None,
                )
                if assistant_display_text:
                    base_event = buffered_plan_assistant_event
                    final_answer_segment = self._materialize_segment_for_live_event(
                        state,
                        current_assistant_turn,
                        ChatTurnLiveEvent(
                            kind="assistant_completed",
                            content_delta=assistant_display_text,
                            app_turn_id=base_event.app_turn_id if base_event is not None else None,
                            item_id=base_event.item_id if base_event is not None else None,
                            phase=base_event.phase if base_event is not None and base_event.phase is not None else "final_answer",
                        ),
                    )
                    if final_answer_segment is not None:
                        emitted_payloads.append(self._build_segment_upsert_payload(state, final_answer_segment))
                else:
                    final_answer_segment = self._completed_final_answer_segment(state, current_assistant_turn.id)
            else:
                final_answer_segment = self._completed_final_answer_segment(state, current_assistant_turn.id)
            preview_fallback = assistant_display_text
            if not _as_non_empty_string(preview_fallback) and plan_segment is not None:
                preview_fallback = plan_segment.content
            if not _as_non_empty_string(preview_fallback):
                preview_fallback = "I reviewed that request."
            if final_answer_segment is None and plan_segment is None:
                error_message = "codex app-server completed the turn without a final answer item."
                failure = RuntimeError(error_message)
            else:
                failure = None
                resolved_content = (
                    final_answer_segment.content
                    if final_answer_segment is not None and _as_non_empty_string(final_answer_segment.content)
                    else (plan_segment.content if plan_segment is not None else preview_fallback)
                )
                turn_changed = (
                    current_assistant_turn.content != resolved_content
                    or current_assistant_turn.status != "complete"
                    or current_assistant_turn.error is not None
                )
                current_assistant_turn.content = resolved_content or preview_fallback
                current_assistant_turn.status = "complete"
                current_assistant_turn.error = None
                self._upsert_turn(state, current_assistant_turn)
                if turn_changed:
                    emitted_payloads.append(self._build_turn_upsert_payload(state, current_assistant_turn))
                self._touch_conversation_state(state)
                self._write_state(state)
                snapshot = state.to_dict()
            _log_project_chat_debug(
                "persisted final assistant turn",
                conversation_id=prepared.conversation_id,
                assistant_message=current_assistant_turn.content,
                turns=_summarize_turns_for_debug(state.turns),
            )
        if failure is not None:
            self._persist_assistant_turn_failure(prepared, str(failure), progress_callback)
            raise failure
        for payload in emitted_payloads:
            self._publish_progress_payload(progress_callback, payload)
        return snapshot

    def _run_prepared_turn_background(
        self,
        prepared: PreparedChatTurn,
        progress_callback: Optional[Callable[[dict[str, Any]], None]] = None,
    ) -> None:
        try:
            self._run_prepared_turn(prepared, progress_callback)
        except RuntimeError as exc:
            LOGGER.warning(
                "project chat background turn ended with runtime error for conversation %s: %s",
                prepared.conversation_id,
                exc,
            )
        except Exception:
            LOGGER.exception(
                "project chat background turn failed for conversation %s",
                prepared.conversation_id,
            )

    def _build_session(self, conversation_id: str, project_path: str) -> CodexAppServerChatSession:
        with self._sessions_lock:
            session = self._sessions.get(conversation_id)
            if session is not None:
                return session
            persisted_session = self._read_session_state(conversation_id, project_path)
            persisted_thread_id = persisted_session.thread_id if persisted_session is not None else None
            persisted_model = persisted_session.model if persisted_session is not None else None
            session = CodexAppServerChatSession(
                project_path,
                persisted_thread_id=persisted_thread_id,
                persisted_model=persisted_model,
                on_thread_id_updated=lambda thread_id: self._persist_session_thread(
                    conversation_id,
                    project_path,
                    thread_id,
                ),
                on_model_updated=lambda model: self._persist_session_model(
                    conversation_id,
                    project_path,
                    model,
                ),
            )
            self._sessions[conversation_id] = session
            return session

    def start_turn(
        self,
        conversation_id: str,
        project_path: str,
        message: str,
        model: Optional[str],
        chat_mode: Optional[str] = None,
        progress_callback: Optional[Callable[[dict[str, Any]], None]] = None,
    ) -> dict[str, Any]:
        prepared, snapshot = self._prepare_turn(
            conversation_id,
            project_path,
            message,
            model,
            chat_mode,
            progress_callback,
        )
        worker = threading.Thread(
            target=self._run_prepared_turn_background,
            args=(prepared, progress_callback),
            daemon=True,
            name=f"project-chat-{conversation_id[-8:]}",
        )
        worker.start()
        return snapshot

    def send_turn(
        self,
        conversation_id: str,
        project_path: str,
        message: str,
        model: Optional[str],
        chat_mode: Optional[str] = None,
        progress_callback: Optional[Callable[[dict[str, Any]], None]] = None,
    ) -> dict[str, Any]:
        prepared, _ = self._prepare_turn(
            conversation_id,
            project_path,
            message,
            model,
            chat_mode,
            progress_callback,
        )
        return self._run_prepared_turn(prepared, progress_callback)

    def resolve_conversation_handle(self, conversation_handle: str) -> tuple[str, str]:
        return self._repository.resolve_conversation_handle(conversation_handle)

    def create_flow_run_request(
        self,
        conversation_id: str,
        project_path: str,
        payload: dict[str, Any],
    ) -> dict[str, object]:
        normalized_project_path = _normalize_project_path(project_path)
        if not normalized_project_path:
            raise ValueError("Project path is required.")
        normalized_payload = _normalize_flow_run_request_payload(
            payload,
            source_name="spark convo run-request",
        )
        return self._reviews.create_flow_run_request(
            conversation_id,
            normalized_project_path,
            normalized_payload,
        )

    def create_flow_run_request_by_handle(
        self,
        conversation_handle: str,
        payload: dict[str, Any],
    ) -> dict[str, object]:
        conversation_id, project_path = self._repository.resolve_conversation_handle(conversation_handle)
        result = self.create_flow_run_request(conversation_id, project_path, payload)
        return {
            **result,
            "conversation_handle": conversation_handle,
        }

    def create_flow_launch(
        self,
        conversation_id: str,
        project_path: str,
        payload: dict[str, Any],
    ) -> dict[str, object]:
        normalized_project_path = _normalize_project_path(project_path)
        if not normalized_project_path:
            raise ValueError("Project path is required.")
        normalized_payload = _normalize_flow_run_request_payload(
            payload,
            source_name="spark run launch",
        )
        return self._reviews.create_flow_launch(
            conversation_id,
            normalized_project_path,
            normalized_payload,
        )

    def create_flow_launch_by_handle(
        self,
        conversation_handle: str,
        payload: dict[str, Any],
    ) -> dict[str, object]:
        conversation_id, project_path = self._repository.resolve_conversation_handle(conversation_handle)
        result = self.create_flow_launch(conversation_id, project_path, payload)
        return {
            **result,
            "conversation_handle": conversation_handle,
        }

    def review_flow_run_request(
        self,
        conversation_id: str,
        project_path: str,
        request_id: str,
        disposition: str,
        message: str,
        flow_name: Optional[str],
        model: Optional[str],
    ) -> tuple[dict[str, Any], "FlowRunRequest"]:
        return self._reviews.review_flow_run_request(
            conversation_id,
            project_path,
            request_id,
            disposition,
            message,
            flow_name,
            model,
        )

    def note_flow_run_request_launched(
        self,
        conversation_id: str,
        request_id: str,
        run_id: str,
        flow_name: str,
    ) -> dict[str, Any]:
        return self._reviews.note_flow_run_request_launched(conversation_id, request_id, run_id, flow_name)

    def fail_flow_run_request_launch(
        self,
        conversation_id: str,
        request_id: str,
        flow_name: str,
        error: str,
    ) -> dict[str, Any]:
        return self._reviews.fail_flow_run_request_launch(conversation_id, request_id, flow_name, error)

    def note_flow_launch_started(
        self,
        conversation_id: str,
        launch_id: str,
        run_id: str,
        flow_name: str,
    ) -> dict[str, Any]:
        return self._reviews.note_flow_launch_started(conversation_id, launch_id, run_id, flow_name)

    def fail_flow_launch(
        self,
        conversation_id: str,
        launch_id: str,
        flow_name: str,
        error: str,
    ) -> dict[str, Any]:
        return self._reviews.fail_flow_launch(conversation_id, launch_id, flow_name, error)
