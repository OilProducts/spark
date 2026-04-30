from __future__ import annotations

import copy
import json
import logging
import re
import subprocess
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
    CONTINUITY_RESET_ERROR_CODE,
    CodexAppServerChatSession,
    UnifiedAgentChatSession,
    PersistedThreadContinuityResetError,
)
from spark.workspace.conversations.artifacts import ProjectChatReviewService
from spark.workspace.conversations.models import (
    ChatTurnResult,
    ConversationSegment,
    ConversationSegmentSource,
    ConversationEventHub,
    RequestUserInputRecord,
    REQUEST_USER_INPUT_EXPIRED_ERROR,
    ConversationSessionState,
    ConversationState,
    ConversationTurn,
    FlowLaunch,
    FlowRunRequest,
    PreparedChatTurn,
    ProposedPlanArtifact,
    ToolCallRecord,
    WorkflowEvent,
    normalize_chat_mode,
    TURN_KIND_MODE_CHANGE,
    validate_chat_mode,
    validate_reasoning_effort,
)
from spark_common.turn_stream import TurnStreamEvent, TurnStreamSource
from spark.workspace.conversations.repository import ProjectChatRepository
from spark.workspace.conversations.utils import (
    as_non_empty_string as _as_non_empty_string,
    iso_now as _iso_now,
    normalize_project_path_value as _normalize_project_path,
)
from spark.workspace.storage import ProjectPaths
from spark_common.codex_app_client import (
    APP_SERVER_REQUEST_TIMEOUT_SECONDS,
    CodexAppServerClient,
)
from spark_common.runtime_path import resolve_runtime_workspace_path
from unified_llm.models import list_models as list_unified_models


CHAT_RUNTIME_THREAD_KEY = "_attractor.runtime.thread_id"
LOGGER = logging.getLogger(__name__)
PROPOSED_PLAN_BLOCK_PATTERN = re.compile(r"(?is)<proposed_plan>\s*(.*?)\s*</proposed_plan>")
EXCESS_BLANK_LINES_PATTERN = re.compile(r"\n{3,}")


def _resolve_flow_validation_command() -> str:
    return "spark flow validate --file <path> --text"


def _normalize_provider(value: Any) -> str:
    normalized = str(value or "").strip().lower()
    return normalized or "codex"


def _validate_provider(value: Any) -> str:
    normalized = _normalize_provider(value)
    if normalized not in {"codex", "openai", "anthropic", "gemini", "openrouter", "litellm", "openai_compatible"}:
        raise ValueError("Provider must be blank or one of: codex, openai, anthropic, gemini, openrouter, litellm, openai_compatible.")
    return normalized


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


def _request_user_input_prompt_summary(request: RequestUserInputRecord) -> str:
    prompts = [
        _normalize_assistant_text(question.question)
        for question in request.questions
        if _normalize_assistant_text(question.question)
    ]
    if len(prompts) == 1:
        return prompts[0]
    if len(prompts) > 1:
        return f"{len(prompts)} questions need user input."
    return "User input requested."


def _request_user_input_answer_summary(request: RequestUserInputRecord) -> str:
    lines: list[str] = []
    for question in request.questions:
        prompt = _normalize_assistant_text(question.question)
        answer = _normalize_assistant_text(request.answers.get(question.id, ""))
        if not prompt or not answer:
            continue
        lines.append(f"{prompt}\nAnswer: {answer}")
    if lines:
        return "\n\n".join(lines)
    return _request_user_input_prompt_summary(request)


def _request_user_input_segment_content(request: RequestUserInputRecord) -> str:
    if request.status in {"answered", "expired"}:
        return _request_user_input_answer_summary(request)
    return _request_user_input_prompt_summary(request)


def _normalize_request_user_input_answers(
    request: RequestUserInputRecord,
    answers: dict[str, Any],
) -> dict[str, str]:
    normalized_answers: dict[str, str] = {}
    for question in request.questions:
        raw_value = answers.get(question.id)
        trimmed_value = str(raw_value).strip() if raw_value is not None else ""
        if not trimmed_value:
            raise ValueError(f"Missing answer for question '{question.id}'.")
        if question.question_type == "MULTIPLE_CHOICE":
            option_labels = {
                _normalize_assistant_text(option.label)
                for option in question.options
                if _normalize_assistant_text(option.label)
            }
            if trimmed_value not in option_labels and not question.allow_other:
                raise ValueError(f"Unsupported answer option for question '{question.id}'.")
        normalized_answers[question.id] = trimmed_value
    if len(normalized_answers) == 0:
        raise ValueError("At least one answer is required.")
    return normalized_answers


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
        self._sessions: dict[str, Any] = {}

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

    def _read_session_state(
        self,
        conversation_id: str,
        project_path: Optional[str] = None,
        *,
        provider: Optional[str] = None,
        model: Optional[str] = None,
    ) -> Optional[ConversationSessionState]:
        return self._repository.read_session_state(
            conversation_id,
            project_path,
            provider=provider,
            model=model,
        )

    def _write_session_state(self, state: ConversationSessionState) -> None:
        self._repository.write_session_state(state)

    def _persist_session_thread(
        self,
        conversation_id: str,
        project_path: str,
        thread_id: str,
        *,
        provider: str = "codex",
        model: Optional[str] = None,
    ) -> None:
        self._repository.persist_session_thread(
            conversation_id,
            project_path,
            thread_id,
            provider=provider,
            model=model,
        )

    def _persist_session_model(
        self,
        conversation_id: str,
        project_path: str,
        model: str,
        *,
        provider: str = "codex",
    ) -> None:
        self._repository.persist_session_model(conversation_id, project_path, model, provider=provider)

    def _clear_session_thread(
        self,
        conversation_id: str,
        project_path: str,
    ) -> None:
        self._repository.clear_session_thread(conversation_id, project_path)

    async def publish_snapshot(self, conversation_id: str) -> None:
        snapshot = self.get_snapshot(conversation_id)
        await self._event_hub.publish(conversation_id, {"type": "conversation_snapshot", "state": snapshot})

    def list_conversations(self, project_path: str) -> list[dict[str, Any]]:
        return self._repository.list_conversations(project_path)

    def get_snapshot(self, conversation_id: str, project_path: Optional[str] = None) -> dict[str, Any]:
        return self._repository.get_snapshot(conversation_id, project_path)

    def _serialize_conversation_state_for_ui(self, state: ConversationState) -> dict[str, Any]:
        return self._repository.serialize_conversation_state_for_ui(state)

    def get_segment_tool_output(
        self,
        conversation_id: str,
        segment_id: str,
        project_path: Optional[str] = None,
    ) -> dict[str, Any]:
        return self._repository.get_segment_tool_output(conversation_id, segment_id, project_path)

    def list_chat_models(self, project_path: str) -> dict[str, Any]:
        normalized_project_path = _normalize_project_path(project_path)
        if not normalized_project_path:
            raise ValueError("Project path is required.")
        unified_models = [
            {
                "provider": model.provider,
                "id": model.id,
                "display": model.display_name,
                "supported_reasoning_efforts": ["low", "medium", "high", "xhigh"]
                if model.supports_reasoning
                else [],
                "default_reasoning_effort": "medium" if model.supports_reasoning else None,
            }
            for model in list_unified_models()
            if model.provider in {"openai", "anthropic", "gemini", "openrouter", "litellm"}
        ]
        runtime_project_path = str(_normalize_project_path(resolve_runtime_workspace_path(normalized_project_path)))
        client = CodexAppServerClient(
            runtime_project_path,
            requested_working_dir=normalized_project_path,
            request_timeout_seconds=APP_SERVER_REQUEST_TIMEOUT_SECONDS,
        )
        codex_models: list[dict[str, Any]] = []
        try:
            client.ensure_process(popen_factory=subprocess.Popen)
            models = client.list_models()
            codex_models = [
                {"provider": "codex", **model.to_dict()}
                for model in models
            ]
        except Exception as exc:
            LOGGER.warning("codex app-server model discovery failed; returning unified models only: %s", exc)
        finally:
            client.close()
        return {"models": [*codex_models, *unified_models]}

    def get_snapshot_by_handle(self, conversation_handle: str) -> dict[str, Any]:
        conversation_id, project_path = self._repository.resolve_conversation_handle(conversation_handle)
        return self.get_snapshot(conversation_id, project_path)

    def delete_conversation(self, conversation_id: str, project_path: str) -> dict[str, Any]:
        snapshot = self._repository.delete_conversation(conversation_id, project_path)
        with self._sessions_lock:
            session_keys = [
                key
                for key in self._sessions
                if key == conversation_id or key.startswith(f"{conversation_id}::")
            ]
            sessions = [self._sessions.pop(key) for key in session_keys]
        for session in sessions:
            session.close()
        return snapshot

    def _append_event(self, state: ConversationState, message: str) -> None:
        self._repository.append_event(state, message)

    def _append_workflow_event(self, state: ConversationState, event: WorkflowEvent) -> None:
        self._repository.append_workflow_event(state, event)

    def _next_turn_segment_order(self, state: ConversationState, turn_id: str) -> int:
        return self._repository.next_turn_segment_order(state, turn_id)

    def _upsert_segment(self, state: ConversationState, segment: ConversationSegment) -> None:
        self._repository.upsert_segment(state, segment)

    def _get_segment(self, state: ConversationState, segment_id: str) -> Optional[ConversationSegment]:
        return self._repository.get_segment(state, segment_id)

    def _build_segment_source(
        self,
        event: TurnStreamEvent,
        *,
        tool_call_id: Optional[str] = None,
    ) -> ConversationSegmentSource:
        return ConversationSegmentSource(
            app_turn_id=event.source.app_turn_id,
            item_id=event.source.item_id,
            summary_index=event.source.summary_index,
            call_id=tool_call_id or (event.tool_call.id if isinstance(event.tool_call, ToolCallRecord) else None),
        )

    def _build_reasoning_segment_id(self, turn_id: str, event: TurnStreamEvent) -> str:
        app_turn_id = event.source.app_turn_id or turn_id
        item_id = event.source.item_id or "reasoning"
        summary_index = event.source.summary_index if event.source.summary_index is not None else 0
        return f"segment-reasoning-{app_turn_id}-{item_id}-{summary_index}"

    def _build_assistant_segment_id(self, turn_id: str, event: TurnStreamEvent) -> str:
        if event.source.app_turn_id and event.source.item_id:
            return f"segment-assistant-{event.source.app_turn_id}-{event.source.item_id}"
        return f"segment-assistant-{turn_id}"

    def _build_plan_segment_id(self, turn_id: str, event: TurnStreamEvent) -> str:
        if event.source.app_turn_id and event.source.item_id:
            return f"segment-plan-{event.source.app_turn_id}-{event.source.item_id}"
        return f"segment-plan-{turn_id}"

    def _build_tool_segment_id(self, turn_id: str, event: TurnStreamEvent) -> str:
        app_turn_id = event.source.app_turn_id or turn_id
        call_id = (event.tool_call.id if isinstance(event.tool_call, ToolCallRecord) else None) or event.source.item_id or "tool"
        return f"segment-tool-{app_turn_id}-{call_id}"

    def _build_context_compaction_segment_id(self, turn_id: str, event: TurnStreamEvent) -> str:
        app_turn_id = event.source.app_turn_id or turn_id
        return f"segment-context-compaction-{app_turn_id}"

    def _build_request_user_input_segment_id(self, turn_id: str, event: TurnStreamEvent) -> str:
        app_turn_id = event.source.app_turn_id or turn_id
        request_id = event.request_user_input.request_id if event.request_user_input is not None else (event.source.item_id or "request")
        return f"segment-request-user-input-{app_turn_id}-{request_id}"

    def _assistant_segment_phase(self, event: TurnStreamEvent, segment: Optional[ConversationSegment] = None) -> Optional[str]:
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

    def _persist_proposed_plan_segment_artifact(
        self,
        state: ConversationState,
        turn: ConversationTurn,
        plan_segment: ConversationSegment,
    ) -> ProposedPlanArtifact:
        return self._reviews.persist_proposed_plan_artifact(state, turn, plan_segment)

    def _request_user_input_segment_by_request_id(
        self,
        state: ConversationState,
        request_or_question_id: str,
    ) -> Optional[ConversationSegment]:
        normalized_request_id = _as_non_empty_string(request_or_question_id)
        if not normalized_request_id:
            return None
        for segment in state.segments:
            if segment.kind != "request_user_input" or segment.request_user_input is None:
                continue
            if segment.request_user_input.request_id == normalized_request_id:
                return segment
            if any(question.id == normalized_request_id for question in segment.request_user_input.questions):
                return segment
        return None

    def _waiting_request_user_input_session(
        self,
        conversation_id: str,
        request_or_question_id: str,
    ) -> Optional[Any]:
        with self._sessions_lock:
            sessions = [
                session
                for key, session in self._sessions.items()
                if key == conversation_id or key.startswith(f"{conversation_id}::")
            ]
        for session in sessions:
            if not hasattr(session, "has_pending_request_user_input"):
                continue
            try:
                if session.has_pending_request_user_input(request_or_question_id):
                    return session
            except Exception:
                LOGGER.warning(
                    "project chat could not inspect live request_user_input state for conversation %s",
                    conversation_id,
                    exc_info=True,
                )
        return None

    def _expire_request_user_input_in_state(
        self,
        state: ConversationState,
        segment: ConversationSegment,
        request: RequestUserInputRecord,
        *,
        submitted_at: str,
    ) -> tuple[ConversationSegment, Optional[ConversationTurn]]:
        request.status = "expired"
        request.submitted_at = submitted_at
        segment.status = "failed"
        segment.updated_at = submitted_at
        segment.completed_at = submitted_at
        segment.error = REQUEST_USER_INPUT_EXPIRED_ERROR
        segment.content = _request_user_input_answer_summary(request)
        segment.request_user_input = request
        self._upsert_segment(state, segment)

        assistant_turn = self._get_turn(state, segment.turn_id)
        if assistant_turn is None or assistant_turn.role != "assistant":
            return segment, None
        if assistant_turn.status not in {"pending", "streaming", "failed"}:
            return segment, None
        assistant_turn.status = "failed"
        assistant_turn.error = REQUEST_USER_INPUT_EXPIRED_ERROR
        self._upsert_turn(state, assistant_turn)
        return segment, assistant_turn

    def _materialize_segment_for_live_event(
        self,
        state: ConversationState,
        turn: ConversationTurn,
        event: TurnStreamEvent,
    ) -> Optional[ConversationSegment]:
        timestamp = _iso_now()
        if event.kind == "content_delta" and event.channel == "reasoning":
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
        if event.kind == "content_delta" and event.channel == "assistant":
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
        if event.kind == "content_delta" and event.channel == "plan":
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
            if segment.source.app_turn_id is None and event.source.app_turn_id is not None:
                segment.source.app_turn_id = event.source.app_turn_id
            if event.source.item_id is not None:
                segment.source.item_id = event.source.item_id
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
        if event.kind == "request_user_input_requested" and event.request_user_input is not None:
            request = RequestUserInputRecord.from_dict(event.request_user_input.to_dict())
            segment_id = self._build_request_user_input_segment_id(turn.id, event)
            segment = self._get_segment(state, segment_id)
            if segment is not None and segment.status == "complete":
                return None
            if segment is None:
                segment = ConversationSegment(
                    id=segment_id,
                    turn_id=turn.id,
                    order=self._next_turn_segment_order(state, turn.id),
                    kind="request_user_input",
                    role="system",
                    status="pending",
                    timestamp=timestamp,
                    updated_at=timestamp,
                    content=_request_user_input_segment_content(request),
                    request_user_input=request,
                    source=self._build_segment_source(event),
                )
                self._upsert_segment(state, segment)
                return segment
            segment.status = "pending"
            segment.updated_at = timestamp
            segment.completed_at = None
            segment.error = None
            segment.content = _request_user_input_segment_content(request)
            segment.request_user_input = request
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
        if event.kind == "content_completed" and event.channel == "assistant":
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
        if event.kind == "content_completed" and event.channel == "reasoning":
            segment_id = self._build_reasoning_segment_id(turn.id, event)
            segment = self._get_segment(state, segment_id) or ConversationSegment(
                id=segment_id,
                turn_id=turn.id,
                order=self._next_turn_segment_order(state, turn.id),
                kind="reasoning",
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
            self._upsert_segment(state, segment)
            return segment
        if event.kind == "content_completed" and event.channel == "plan":
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
        if event.kind == "error":
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

    def _stamp_progress_payloads_with_state_revision(
        self,
        state: ConversationState,
        payloads: list[dict[str, Any]],
    ) -> None:
        for payload in payloads:
            payload["revision"] = state.revision
            payload["updated_at"] = state.updated_at

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
        chat_mode: Optional[str] = None,
        model: Optional[str] = None,
        reasoning_effort: Optional[str] = None,
        provider: Optional[str] = None,
        llm_profile: Optional[str] = None,
    ) -> dict[str, Any]:
        normalized_project_path = _normalize_project_path(project_path)
        if not normalized_project_path:
            raise ValueError("Project path is required.")
        normalized_chat_mode = validate_chat_mode(chat_mode) if chat_mode is not None else None
        normalized_provider = _validate_provider(provider) if provider is not None else None
        normalized_profile = _as_non_empty_string(llm_profile) if llm_profile is not None else None
        normalized_model = _as_non_empty_string(model) if model is not None else None
        normalized_reasoning_effort = (
            validate_reasoning_effort(reasoning_effort)
            if reasoning_effort is not None
            else None
        )
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
            if normalized_chat_mode is not None and normalized_chat_mode != current_chat_mode:
                state.turns.append(_build_mode_change_turn(normalized_chat_mode))
                state.chat_mode = normalized_chat_mode
            else:
                state.chat_mode = current_chat_mode
            if model is not None:
                state.model = normalized_model
            if provider is not None:
                state.provider = normalized_provider or "codex"
            if llm_profile is not None:
                state.llm_profile = normalized_profile
            if reasoning_effort is not None:
                state.reasoning_effort = normalized_reasoning_effort
            self._touch_conversation_state(state)
            self._write_state(state)
            return self._serialize_conversation_state_for_ui(state)

    def _prepare_turn(
        self,
        conversation_id: str,
        project_path: str,
        message: str,
        model: Optional[str],
        chat_mode: Optional[str] = None,
        reasoning_effort: Optional[str] = None,
        progress_callback: Optional[Callable[[dict[str, Any]], None]] = None,
        provider: Optional[str] = None,
        llm_profile: Optional[str] = None,
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
            if model is not None:
                state.model = _as_non_empty_string(model)
            if provider is not None:
                state.provider = _validate_provider(provider)
            if llm_profile is not None:
                state.llm_profile = _as_non_empty_string(llm_profile)
            if reasoning_effort is not None:
                state.reasoning_effort = validate_reasoning_effort(reasoning_effort)
            effective_provider = _normalize_provider(state.provider)
            effective_model = state.model
            effective_profile = state.llm_profile
            effective_reasoning_effort = state.reasoning_effort
            if (effective_profile or effective_provider in {"openrouter", "litellm", "openai_compatible"}) and not effective_model:
                raise ValueError(f"Provider {effective_provider} requires an explicit model.")
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
            snapshot = self._serialize_conversation_state_for_ui(state)
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
        return (
            PreparedChatTurn(
                conversation_id=conversation_id,
                project_path=normalized_project_path,
                chat_mode=effective_chat_mode,
                prompt=prompt,
                provider=effective_provider,
                model=effective_model,
                llm_profile=effective_profile,
                reasoning_effort=effective_reasoning_effort,
                user_turn=user_turn,
                assistant_turn=assistant_turn,
            ),
            snapshot,
        )

    def _persist_assistant_turn_failure_for_turn(
        self,
        conversation_id: str,
        project_path: str,
        assistant_turn: ConversationTurn,
        error_message: str,
        *,
        error_code: Optional[str] = None,
        progress_callback: Optional[Callable[[dict[str, Any]], None]] = None,
    ) -> None:
        with self._lock:
            current_state = self._read_state(conversation_id, project_path)
            if current_state is None:
                return
            current_assistant_turn = self._get_turn(current_state, assistant_turn.id)
            if current_assistant_turn is None:
                current_assistant_turn = ConversationTurn.from_dict(assistant_turn.to_dict())
            current_assistant_turn.status = "failed"
            current_assistant_turn.error = error_message
            current_assistant_turn.error_code = error_code
            self._upsert_turn(current_state, current_assistant_turn)
            emitted_payloads: list[dict[str, Any]] = []
            for segment in current_state.segments:
                if segment.turn_id != current_assistant_turn.id or segment.kind not in {"assistant_message", "plan"}:
                    continue
                segment.status = "failed"
                segment.error = error_message
                segment.error_code = error_code
                segment.updated_at = _iso_now()
                segment.completed_at = segment.updated_at
                self._upsert_segment(current_state, segment)
                emitted_payloads.append(self._build_segment_upsert_payload(current_state, segment))
            self._touch_conversation_state(current_state)
            self._stamp_progress_payloads_with_state_revision(current_state, emitted_payloads)
            self._write_state(current_state)
            assistant_upsert_payload = self._build_turn_upsert_payload(current_state, current_assistant_turn)
        for payload in emitted_payloads:
            self._publish_progress_payload(progress_callback, payload)
        self._publish_progress_payload(progress_callback, assistant_upsert_payload)

    def _persist_assistant_turn_failure(
        self,
        prepared: PreparedChatTurn,
        error_message: str,
        *,
        error_code: Optional[str] = None,
        progress_callback: Optional[Callable[[dict[str, Any]], None]] = None,
    ) -> None:
        self._persist_assistant_turn_failure_for_turn(
            prepared.conversation_id,
            prepared.project_path,
            prepared.assistant_turn,
            error_message,
            error_code=error_code,
            progress_callback=progress_callback,
        )

    def _execute_turn(
        self,
        prepared: PreparedChatTurn,
        persist_live_event: Callable[[TurnStreamEvent], None],
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
                    reasoning_effort=prepared.reasoning_effort,
                    on_event=persist_live_event,
                )
            finally:
                clear_raw_rpc_logger(target_session)

        try:
            session = self._build_session(
                prepared.conversation_id,
                prepared.project_path,
                prepared.provider,
                prepared.model,
                llm_profile=prepared.llm_profile,
                history_exclude_turn_ids={prepared.user_turn.id, prepared.assistant_turn.id},
            )
        except TypeError:
            try:
                if prepared.llm_profile is not None:
                    session = self._build_session(
                        prepared.conversation_id,
                        prepared.project_path,
                        prepared.provider,
                        prepared.model,
                        llm_profile=prepared.llm_profile,
                    )
                else:
                    session = self._build_session(
                        prepared.conversation_id,
                        prepared.project_path,
                        prepared.provider,
                        prepared.model,
                    )
            except TypeError:
                session = self._build_session(prepared.conversation_id, prepared.project_path)
        return run_session(session)

    def _run_prepared_turn(
        self,
        prepared: PreparedChatTurn,
        progress_callback: Optional[Callable[[dict[str, Any]], None]] = None,
    ) -> dict[str, Any]:
        return self._run_assistant_turn_from_session(
            conversation_id=prepared.conversation_id,
            project_path=prepared.project_path,
            chat_mode=prepared.chat_mode,
            assistant_turn=prepared.assistant_turn,
            progress_callback=progress_callback,
            session_runner=lambda persist_live_event, _progress_callback: self._execute_turn(
                prepared,
                persist_live_event,
                _progress_callback,
            ),
            persist_failure=True,
        )

    def _run_assistant_turn_from_session(
        self,
        *,
        conversation_id: str,
        project_path: str,
        chat_mode: str,
        assistant_turn: ConversationTurn,
        session_runner: Callable[
            [Callable[[TurnStreamEvent], None], Optional[Callable[[dict[str, Any]], None]]],
            ChatTurnResult,
        ],
        progress_callback: Optional[Callable[[dict[str, Any]], None]] = None,
        persist_failure: bool,
    ) -> dict[str, Any]:
        with self._lock:
            state = self._read_state(conversation_id, project_path)
            if state is None:
                raise RuntimeError("Conversation state disappeared before the turn started.")

        buffered_plan_assistant_event: TurnStreamEvent | None = None

        def persist_live_event(event: TurnStreamEvent) -> None:
            nonlocal buffered_plan_assistant_event
            with self._lock:
                current_state = self._read_state(conversation_id, project_path) or state
                current_assistant_turn = self._get_turn(current_state, assistant_turn.id)
                if current_assistant_turn is None:
                    current_assistant_turn = ConversationTurn.from_dict(assistant_turn.to_dict())
                    self._upsert_turn(current_state, current_assistant_turn)

                emitted_payloads: list[dict[str, Any]] = []
                if current_assistant_turn.status == "pending":
                    current_assistant_turn.status = "streaming"
                    self._upsert_turn(current_state, current_assistant_turn)
                    emitted_payloads.append(self._build_turn_upsert_payload(current_state, current_assistant_turn))

                if event.kind == "token_usage_updated" and isinstance(event.token_usage, dict):
                    current_assistant_turn.token_usage = copy.deepcopy(event.token_usage)
                    self._upsert_turn(current_state, current_assistant_turn)
                    emitted_payloads.append(self._build_turn_upsert_payload(current_state, current_assistant_turn))
                elif event.kind == "content_delta" and event.channel == "assistant":
                    if event.content_delta and chat_mode != "plan":
                        segment = self._materialize_segment_for_live_event(current_state, current_assistant_turn, event)
                        if segment is not None:
                            emitted_payloads.append(self._build_segment_upsert_payload(current_state, segment))
                elif event.kind == "content_delta" and event.channel == "reasoning":
                    if event.content_delta:
                        segment = self._materialize_segment_for_live_event(current_state, current_assistant_turn, event)
                        if segment is not None:
                            emitted_payloads.append(self._build_segment_upsert_payload(current_state, segment))
                elif event.kind == "content_completed" and event.channel == "reasoning":
                    segment = self._materialize_segment_for_live_event(current_state, current_assistant_turn, event)
                    if segment is not None:
                        emitted_payloads.append(self._build_segment_upsert_payload(current_state, segment))
                elif event.kind == "content_delta" and event.channel == "plan":
                    if event.content_delta:
                        segment = self._materialize_segment_for_live_event(current_state, current_assistant_turn, event)
                        if segment is not None:
                            emitted_payloads.append(self._build_segment_upsert_payload(current_state, segment))
                elif event.kind == "content_completed" and event.channel == "plan":
                    segment = self._materialize_segment_for_live_event(current_state, current_assistant_turn, event)
                    if segment is not None:
                        self._persist_proposed_plan_segment_artifact(current_state, current_assistant_turn, segment)
                        emitted_payloads.append(self._build_segment_upsert_payload(current_state, segment))
                elif event.kind in {"context_compaction_started", "context_compaction_completed"}:
                    segment = self._materialize_segment_for_live_event(current_state, current_assistant_turn, event)
                    if segment is not None:
                        emitted_payloads.append(self._build_segment_upsert_payload(current_state, segment))
                elif event.kind == "request_user_input_requested" and event.request_user_input is not None:
                    segment = self._materialize_segment_for_live_event(current_state, current_assistant_turn, event)
                    if segment is not None:
                        emitted_payloads.append(self._build_segment_upsert_payload(current_state, segment))
                elif event.kind == "content_completed" and event.channel == "assistant":
                    assistant_message = _as_non_empty_string(event.content_delta)
                    current_assistant_turn.status = "streaming"
                    current_assistant_turn.error = None
                    self._upsert_turn(current_state, current_assistant_turn)
                    if chat_mode == "plan":
                        if assistant_message and self._is_final_answer_phase(event.phase):
                            buffered_plan_assistant_event = TurnStreamEvent(
                                kind=event.kind,
                                channel=event.channel,
                                content_delta=assistant_message,
                                message=event.message,
                                source=event.source,
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
                self._stamp_progress_payloads_with_state_revision(current_state, emitted_payloads)
                self._write_state(current_state)
                _log_project_chat_debug(
                    "persisted progress events",
                    conversation_id=conversation_id,
                    event_kind=event.kind,
                    turns=_summarize_turns_for_debug(current_state.turns),
                )
            for payload in emitted_payloads:
                self._publish_progress_payload(progress_callback, payload)

        try:
            turn_result = session_runner(persist_live_event, progress_callback)
        except PersistedThreadContinuityResetError as exc:
            self._clear_session_thread(conversation_id, project_path)
            continuity_reset_payload = exc.to_debug_payload()
            with self._lock:
                current_state = self._read_state(conversation_id, project_path)
                if current_state is not None:
                    self._append_workflow_event(
                        current_state,
                        WorkflowEvent(
                            message=str(exc),
                            timestamp=_iso_now(),
                            kind=str(continuity_reset_payload.get("event") or "continuity_reset"),
                            error_code=(
                                str(continuity_reset_payload.get("error_code"))
                                if continuity_reset_payload.get("error_code") is not None
                                else None
                            ),
                            details={
                                key: value
                                for key, value in continuity_reset_payload.items()
                                if key not in {"event", "error_code"}
                            },
                        ),
                    )
                    self._touch_conversation_state(current_state)
                    self._write_state(current_state)
            _log_project_chat_debug(
                "persisted continuity reset after failed thread resume",
                conversation_id=conversation_id,
                project_path=project_path,
                persisted_thread_id=exc.persisted_thread_id,
                resume_failure=exc.failure.to_dict(),
            )
            if persist_failure:
                self._persist_assistant_turn_failure_for_turn(
                    conversation_id,
                    project_path,
                    assistant_turn,
                    str(exc),
                    error_code=CONTINUITY_RESET_ERROR_CODE,
                    progress_callback=progress_callback,
                )
            raise
        except Exception as exc:
            if persist_failure:
                self._persist_assistant_turn_failure_for_turn(
                    conversation_id,
                    project_path,
                    assistant_turn,
                    str(exc),
                    progress_callback=progress_callback,
                )
            raise

        assistant_message, _ = _parse_chat_response_payload(turn_result.assistant_message)
        with self._lock:
            state = self._read_state(conversation_id, project_path) or state
            current_assistant_turn = self._get_turn(state, assistant_turn.id)
            if current_assistant_turn is None:
                current_assistant_turn = ConversationTurn.from_dict(assistant_turn.to_dict())
            plan_segment = self._completed_plan_segment(state, current_assistant_turn.id)
            emitted_payloads: list[dict[str, Any]] = []
            if plan_segment is not None and not plan_segment.artifact_id:
                self._persist_proposed_plan_segment_artifact(state, current_assistant_turn, plan_segment)
                emitted_payloads.append(self._build_segment_upsert_payload(state, plan_segment))
            assistant_display_text = assistant_message
            if chat_mode == "plan":
                assistant_display_text = _extract_plan_mode_assistant_remainder(
                    assistant_message,
                    plan_segment.content if plan_segment is not None else None,
                )
                if assistant_display_text:
                    base_event = buffered_plan_assistant_event
                    final_answer_segment = self._materialize_segment_for_live_event(
                        state,
                        current_assistant_turn,
                        TurnStreamEvent(
                            kind="content_completed",
                            channel="assistant",
                            content_delta=assistant_display_text,
                            source=base_event.source if base_event is not None else TurnStreamSource(),
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
                failure = RuntimeError("codex app-server completed the turn without a final answer item.")
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
                    or (
                        isinstance(turn_result.token_usage, dict)
                        and current_assistant_turn.token_usage != turn_result.token_usage
                    )
                )
                current_assistant_turn.content = resolved_content or preview_fallback
                current_assistant_turn.status = "complete"
                current_assistant_turn.error = None
                if isinstance(turn_result.token_usage, dict):
                    current_assistant_turn.token_usage = copy.deepcopy(turn_result.token_usage)
                self._upsert_turn(state, current_assistant_turn)
                if turn_changed:
                    emitted_payloads.append(self._build_turn_upsert_payload(state, current_assistant_turn))
                self._touch_conversation_state(state)
                self._stamp_progress_payloads_with_state_revision(state, emitted_payloads)
                self._write_state(state)
                snapshot = self._serialize_conversation_state_for_ui(state)
            _log_project_chat_debug(
                "persisted final assistant turn",
                conversation_id=conversation_id,
                assistant_message=current_assistant_turn.content,
                turns=_summarize_turns_for_debug(state.turns),
            )
        if failure is not None:
            if persist_failure:
                self._persist_assistant_turn_failure_for_turn(
                    conversation_id,
                    project_path,
                    assistant_turn,
                    str(failure),
                    progress_callback=progress_callback,
                )
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

    def _session_key(
        self,
        conversation_id: str,
        provider: str,
        model: Optional[str],
        llm_profile: Optional[str] = None,
    ) -> str:
        if llm_profile:
            return f"{conversation_id}::{_normalize_provider(provider)}::{str(llm_profile).strip()}::{str(model or '').strip()}"
        return f"{conversation_id}::{_normalize_provider(provider)}::{str(model or '').strip()}"

    def _build_session(
        self,
        conversation_id: str,
        project_path: str,
        provider: str = "codex",
        model: Optional[str] = None,
        llm_profile: Optional[str] = None,
        history_exclude_turn_ids: set[str] | None = None,
    ) -> Any:
        normalized_provider = _validate_provider(provider)
        normalized_profile = _as_non_empty_string(llm_profile)
        session_key = self._session_key(conversation_id, normalized_provider, model, normalized_profile)
        with self._sessions_lock:
            session = self._sessions.get(session_key)
            if session is not None:
                target_session = session
            elif normalized_provider != "codex" or normalized_profile is not None:
                state = self._read_state(conversation_id, project_path)
                excluded = history_exclude_turn_ids or set()
                persisted_history = [
                    turn
                    for turn in (state.turns if state is not None else [])
                    if turn.id not in excluded
                ]
                session_kwargs: dict[str, Any] = {
                    "provider": normalized_provider,
                    "model": model,
                    "persisted_history": persisted_history,
                }
                if normalized_profile is not None:
                    session_kwargs["llm_profile"] = normalized_profile
                    session_kwargs["config_dir"] = self._data_dir / "config"
                target_session = UnifiedAgentChatSession(project_path, **session_kwargs)
                self._sessions[session_key] = target_session
            else:
                persisted_session = self._read_session_state(
                    conversation_id,
                    project_path,
                    provider=normalized_provider,
                    model=model,
                )
                if persisted_session is None and model is None:
                    persisted_session = self._read_session_state(conversation_id, project_path)
                persisted_thread_id = (
                    persisted_session.thread_id
                    if persisted_session is not None and persisted_session.provider == "codex"
                    else None
                )
                persisted_model = (
                    persisted_session.model
                    if persisted_session is not None and persisted_session.provider == "codex"
                    else None
                )
                session_identity: dict[str, Optional[str]] = {"model": model}

                def persist_thread(thread_id: str) -> None:
                    self._persist_session_thread(
                        conversation_id,
                        project_path,
                        thread_id,
                        provider=normalized_provider,
                        model=session_identity["model"],
                    )

                def persist_model(resolved_model: str) -> None:
                    session_identity["model"] = resolved_model
                    self._persist_session_model(
                        conversation_id,
                        project_path,
                        resolved_model,
                        provider=normalized_provider,
                    )

                target_session = CodexAppServerChatSession(
                    project_path,
                    persisted_thread_id=persisted_thread_id,
                    persisted_model=persisted_model,
                    on_thread_id_updated=persist_thread,
                    on_model_updated=persist_model,
                )
                self._sessions[session_key] = target_session
        return target_session

    def start_turn(
        self,
        conversation_id: str,
        project_path: str,
        message: str,
        model: Optional[str] = None,
        chat_mode: Optional[str] = None,
        reasoning_effort: Optional[str] = None,
        progress_callback: Optional[Callable[[dict[str, Any]], None]] = None,
        provider: Optional[str] = None,
        llm_profile: Optional[str] = None,
    ) -> dict[str, Any]:
        prepared, snapshot = self._prepare_turn(
            conversation_id,
            project_path,
            message,
            model,
            chat_mode,
            reasoning_effort,
            progress_callback,
            provider=provider,
            llm_profile=llm_profile,
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
        model: Optional[str] = None,
        chat_mode: Optional[str] = None,
        reasoning_effort: Optional[str] = None,
        progress_callback: Optional[Callable[[dict[str, Any]], None]] = None,
        provider: Optional[str] = None,
        llm_profile: Optional[str] = None,
    ) -> dict[str, Any]:
        prepared, _ = self._prepare_turn(
            conversation_id,
            project_path,
            message,
            model,
            chat_mode,
            reasoning_effort,
            progress_callback,
            provider=provider,
            llm_profile=llm_profile,
        )
        return self._run_prepared_turn(prepared, progress_callback)

    def submit_request_user_input_answer(
        self,
        conversation_id: str,
        project_path: str,
        request_or_question_id: str,
        answers: dict[str, Any],
        progress_callback: Optional[Callable[[dict[str, Any]], None]] = None,
    ) -> dict[str, Any]:
        normalized_project_path = _normalize_project_path(project_path)
        if not normalized_project_path:
            raise ValueError("Project path is required.")
        normalized_lookup_id = _as_non_empty_string(request_or_question_id)
        if not normalized_lookup_id:
            raise ValueError("Request id is required.")
        if not isinstance(answers, dict):
            raise ValueError("Answers are required.")

        emitted_payloads: list[dict[str, Any]] = []
        with self._lock:
            state = self._read_state(conversation_id, normalized_project_path)
            if state is None:
                raise FileNotFoundError(conversation_id)
            if state.project_path != normalized_project_path:
                raise ValueError("Conversation is already bound to a different project path.")
            segment = self._request_user_input_segment_by_request_id(state, normalized_lookup_id)
            if segment is None or segment.request_user_input is None:
                raise FileNotFoundError(normalized_lookup_id)
            request = RequestUserInputRecord.from_dict(segment.request_user_input.to_dict())
            normalized_answers = _normalize_request_user_input_answers(request, answers)
            if request.status == "answered":
                if request.answers == normalized_answers:
                    snapshot = self._serialize_conversation_state_for_ui(state)
                else:
                    raise ValueError("That conversation request is already answered.")
            elif request.status == "expired":
                if request.answers == normalized_answers:
                    snapshot = self._serialize_conversation_state_for_ui(state)
                else:
                    raise ValueError(REQUEST_USER_INPUT_EXPIRED_ERROR)
            else:
                submitted_at = _iso_now()
                request.answers = normalized_answers
                request.submitted_at = submitted_at
                waiting_session = self._waiting_request_user_input_session(conversation_id, normalized_lookup_id)
                if waiting_session is None and request.request_id != normalized_lookup_id:
                    waiting_session = self._waiting_request_user_input_session(conversation_id, request.request_id)
                accepted = False
                if waiting_session is not None:
                    try:
                        accepted = bool(waiting_session.submit_request_user_input_answers(request.request_id, normalized_answers))
                    except Exception:
                        LOGGER.warning(
                            "project chat could not submit live request_user_input answers for conversation %s request %s",
                            conversation_id,
                            request.request_id,
                            exc_info=True,
                        )
                if accepted:
                    request.status = "answered"
                    segment.status = "complete"
                    segment.updated_at = submitted_at
                    segment.completed_at = submitted_at
                    segment.error = None
                    segment.content = _request_user_input_segment_content(request)
                    segment.request_user_input = request
                    self._upsert_segment(state, segment)
                else:
                    expired_segment, expired_turn = self._expire_request_user_input_in_state(
                        state,
                        segment,
                        request,
                        submitted_at=submitted_at,
                    )
                    if expired_turn is not None:
                        emitted_payloads.append(self._build_turn_upsert_payload(state, expired_turn))
                    segment = expired_segment
                self._touch_conversation_state(state)
                self._stamp_progress_payloads_with_state_revision(state, emitted_payloads)
                emitted_payloads.append(self._build_segment_upsert_payload(state, segment))
                self._write_state(state)
                snapshot = self._serialize_conversation_state_for_ui(state)

        for payload in emitted_payloads:
            self._publish_progress_payload(progress_callback, payload)
        return snapshot

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
        llm_provider: Optional[str] = None,
        llm_profile: Optional[str] = None,
        reasoning_effort: Optional[str] = None,
        execution_container_image: Optional[str] = None,
    ) -> tuple[dict[str, Any], "FlowRunRequest"]:
        return self._reviews.review_flow_run_request(
            conversation_id,
            project_path,
            request_id,
            disposition,
            message,
            flow_name,
            model,
            llm_provider,
            llm_profile,
            reasoning_effort,
            execution_container_image,
        )

    def review_proposed_plan(
        self,
        conversation_id: str,
        project_path: str,
        plan_id: str,
        disposition: str,
        review_note: Optional[str],
    ) -> tuple[dict[str, Any], ProposedPlanArtifact, Optional[FlowLaunch]]:
        return self._reviews.review_proposed_plan(
            conversation_id,
            project_path,
            plan_id,
            disposition,
            review_note,
        )

    def note_proposed_plan_launch_started(
        self,
        conversation_id: str,
        plan_id: str,
        run_id: str,
        flow_name: str,
    ) -> dict[str, Any]:
        return self._reviews.note_proposed_plan_launch_started(conversation_id, plan_id, run_id, flow_name)

    def fail_proposed_plan_launch(
        self,
        conversation_id: str,
        plan_id: str,
        flow_name: str,
        error: str,
    ) -> dict[str, Any]:
        return self._reviews.fail_proposed_plan_launch(conversation_id, plan_id, flow_name, error)

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
