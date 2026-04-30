from __future__ import annotations

import asyncio
import copy
from dataclasses import dataclass, field
from pathlib import Path
import threading
from time import gmtime, strftime
from typing import Any, Optional

CHAT_SESSION_VERSION = 2
CONVERSATION_STATE_SCHEMA_VERSION = 5
CHAT_MODE_CHAT = "chat"
CHAT_MODE_PLAN = "plan"
CHAT_MODES = frozenset({CHAT_MODE_CHAT, CHAT_MODE_PLAN})
REASONING_EFFORTS = frozenset({"low", "medium", "high", "xhigh"})
TURN_KIND_MESSAGE = "message"
TURN_KIND_MODE_CHANGE = "mode_change"
REQUEST_USER_INPUT_EXPIRED_ERROR = "The requested input expired before the answer could be used."


def _iso_now() -> str:
    return strftime("%Y-%m-%dT%H:%M:%SZ", gmtime())


def _normalize_project_path(value: str) -> str:
    trimmed = value.strip()
    if not trimmed:
        return ""
    return str(Path(trimmed).expanduser().resolve(strict=False))


def _as_non_empty_string(value: Any) -> Optional[str]:
    if not isinstance(value, str):
        return None
    trimmed = value.strip()
    return trimmed or None


def _truncate_text(value: str, limit: int) -> str:
    collapsed = " ".join(value.split())
    if len(collapsed) <= limit:
        return collapsed
    return collapsed[: max(0, limit - 1)].rstrip() + "…"


def _normalize_request_user_input_status(value: Any) -> str:
    normalized = _as_non_empty_string(value)
    if normalized in {"answered", "expired"}:
        return normalized
    return "pending"


def normalize_chat_mode(value: Any) -> str:
    if not isinstance(value, str):
        return CHAT_MODE_CHAT
    normalized = value.strip().lower()
    if normalized in CHAT_MODES:
        return normalized
    return CHAT_MODE_CHAT


def validate_chat_mode(value: Any) -> str:
    if not isinstance(value, str):
        raise ValueError("Chat mode must be 'chat' or 'plan'.")
    normalized = value.strip().lower()
    if normalized not in CHAT_MODES:
        raise ValueError("Chat mode must be 'chat' or 'plan'.")
    return normalized


def normalize_reasoning_effort(value: Any) -> Optional[str]:
    if not isinstance(value, str):
        return None
    normalized = value.strip().lower()
    if not normalized:
        return ""
    if normalized in REASONING_EFFORTS:
        return normalized
    return None


def validate_reasoning_effort(value: Any) -> Optional[str]:
    if value is None:
        return None
    if not isinstance(value, str):
        raise ValueError("Reasoning effort must be blank or one of: low, medium, high, xhigh.")
    normalized = value.strip().lower()
    if not normalized:
        return ""
    if normalized not in REASONING_EFFORTS:
        raise ValueError("Reasoning effort must be blank or one of: low, medium, high, xhigh.")
    return normalized


def _derive_conversation_title(turns: list["ConversationTurn"]) -> str:
    for turn in turns:
        if turn.kind != TURN_KIND_MESSAGE or turn.role != "user":
            continue
        title = _truncate_text(turn.content, 64)
        if title:
            return title
    return "New thread"


def _build_conversation_preview(turns: list["ConversationTurn"]) -> Optional[str]:
    for turn in reversed(turns):
        if turn.kind != TURN_KIND_MESSAGE:
            continue
        preview = _truncate_text(turn.content, 120)
        if preview:
            return preview
    return None


@dataclass
class ConversationTurn:
    id: str
    role: str
    content: str
    timestamp: str
    status: str = "complete"
    kind: str = TURN_KIND_MESSAGE
    artifact_id: Optional[str] = None
    parent_turn_id: Optional[str] = None
    error: Optional[str] = None
    token_usage: Optional[dict[str, Any]] = None
    error_code: Optional[str] = None

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
        if isinstance(self.token_usage, dict):
            payload["token_usage"] = copy.deepcopy(self.token_usage)
        if self.error_code:
            payload["error_code"] = self.error_code
        return payload

    @classmethod
    def from_dict(cls, payload: dict[str, Any]) -> "ConversationTurn":
        token_usage = payload.get("token_usage")
        return cls(
            id=str(payload.get("id", "")),
            role=str(payload.get("role", "assistant")),
            content=str(payload.get("content", "")),
            timestamp=str(payload.get("timestamp", "")),
            status=str(payload.get("status", "complete") or "complete"),
            kind=str(payload.get("kind", TURN_KIND_MESSAGE) or TURN_KIND_MESSAGE),
            artifact_id=str(payload.get("artifact_id")) if payload.get("artifact_id") is not None else None,
            parent_turn_id=str(payload.get("parent_turn_id")) if payload.get("parent_turn_id") is not None else None,
            error=str(payload.get("error")) if payload.get("error") is not None else None,
            token_usage=copy.deepcopy(token_usage) if isinstance(token_usage, dict) else None,
            error_code=str(payload.get("error_code")) if payload.get("error_code") is not None else None,
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
class RequestUserInputOption:
    label: str
    description: Optional[str] = None

    def to_dict(self) -> dict[str, Any]:
        payload: dict[str, Any] = {
            "label": self.label,
        }
        if self.description is not None:
            payload["description"] = self.description
        return payload

    @classmethod
    def from_dict(cls, payload: dict[str, Any]) -> "RequestUserInputOption":
        return cls(
            label=str(payload.get("label", "")),
            description=str(payload.get("description")) if payload.get("description") is not None else None,
        )


@dataclass
class RequestUserInputQuestion:
    id: str
    header: str
    question: str
    question_type: str
    options: list[RequestUserInputOption] = field(default_factory=list)
    allow_other: bool = False
    is_secret: bool = False

    def to_dict(self) -> dict[str, Any]:
        return {
            "id": self.id,
            "header": self.header,
            "question": self.question,
            "question_type": self.question_type,
            "options": [option.to_dict() for option in self.options],
            "allow_other": self.allow_other,
            "is_secret": self.is_secret,
        }

    @classmethod
    def from_dict(cls, payload: dict[str, Any]) -> "RequestUserInputQuestion":
        raw_options = payload.get("options")
        return cls(
            id=str(payload.get("id", "")),
            header=str(payload.get("header", "")),
            question=str(payload.get("question", "")),
            question_type=str(payload.get("question_type", "FREEFORM") or "FREEFORM"),
            options=[
                RequestUserInputOption.from_dict(option)
                for option in raw_options
                if isinstance(option, dict)
            ] if isinstance(raw_options, list) else [],
            allow_other=bool(payload.get("allow_other")),
            is_secret=bool(payload.get("is_secret")),
        )


@dataclass
class RequestUserInputRecord:
    request_id: str
    status: str
    questions: list[RequestUserInputQuestion] = field(default_factory=list)
    answers: dict[str, str] = field(default_factory=dict)
    submitted_at: Optional[str] = None

    def to_dict(self) -> dict[str, Any]:
        payload: dict[str, Any] = {
            "request_id": self.request_id,
            "status": self.status,
            "questions": [question.to_dict() for question in self.questions],
            "answers": dict(self.answers),
        }
        if self.submitted_at is not None:
            payload["submitted_at"] = self.submitted_at
        return payload

    @classmethod
    def from_dict(cls, payload: dict[str, Any]) -> "RequestUserInputRecord":
        raw_questions = payload.get("questions")
        raw_answers = payload.get("answers")
        status = str(payload.get("status", "pending") or "pending")
        return cls(
            request_id=str(payload.get("request_id", "")),
            status=_normalize_request_user_input_status(status),
            questions=[
                RequestUserInputQuestion.from_dict(question)
                for question in raw_questions
                if isinstance(question, dict)
            ] if isinstance(raw_questions, list) else [],
            answers={
                str(key): str(value)
                for key, value in raw_answers.items()
                if value is not None
            } if isinstance(raw_answers, dict) else {},
            submitted_at=str(payload.get("submitted_at")) if payload.get("submitted_at") is not None else None,
        )


@dataclass
class ChatTurnResult:
    assistant_message: str
    token_usage: Optional[dict[str, Any]] = None


@dataclass
class PreparedChatTurn:
    conversation_id: str
    project_path: str
    chat_mode: str
    prompt: str
    provider: str
    model: Optional[str]
    llm_profile: Optional[str]
    reasoning_effort: Optional[str]
    user_turn: "ConversationTurn"
    assistant_turn: "ConversationTurn"


@dataclass
class ConversationSegmentSource:
    app_turn_id: Optional[str] = None
    item_id: Optional[str] = None
    summary_index: Optional[int] = None
    call_id: Optional[str] = None

    def to_dict(self) -> dict[str, Any]:
        payload: dict[str, Any] = {}
        if self.app_turn_id is not None:
            payload["app_turn_id"] = self.app_turn_id
        if self.item_id is not None:
            payload["item_id"] = self.item_id
        if self.summary_index is not None:
            payload["summary_index"] = self.summary_index
        if self.call_id is not None:
            payload["call_id"] = self.call_id
        return payload

    @classmethod
    def from_dict(cls, payload: dict[str, Any]) -> "ConversationSegmentSource":
        summary_index = payload.get("summary_index")
        return cls(
            app_turn_id=str(payload.get("app_turn_id")) if payload.get("app_turn_id") is not None else None,
            item_id=str(payload.get("item_id")) if payload.get("item_id") is not None else None,
            summary_index=int(summary_index) if isinstance(summary_index, int) else None,
            call_id=str(payload.get("call_id")) if payload.get("call_id") is not None else None,
        )


@dataclass
class ConversationSegment:
    id: str
    turn_id: str
    order: int
    kind: str
    role: str
    status: str
    timestamp: str
    updated_at: str
    content: str = ""
    completed_at: Optional[str] = None
    error: Optional[str] = None
    error_code: Optional[str] = None
    artifact_id: Optional[str] = None
    phase: Optional[str] = None
    tool_call: Optional[ToolCallRecord] = None
    request_user_input: Optional[RequestUserInputRecord] = None
    source: ConversationSegmentSource = field(default_factory=ConversationSegmentSource)

    def to_dict(self) -> dict[str, Any]:
        payload: dict[str, Any] = {
            "id": self.id,
            "turn_id": self.turn_id,
            "order": self.order,
            "kind": self.kind,
            "role": self.role,
            "status": self.status,
            "timestamp": self.timestamp,
            "updated_at": self.updated_at,
            "content": self.content,
            "source": self.source.to_dict(),
        }
        if self.completed_at is not None:
            payload["completed_at"] = self.completed_at
        if self.error is not None:
            payload["error"] = self.error
        if self.error_code is not None:
            payload["error_code"] = self.error_code
        if self.artifact_id is not None:
            payload["artifact_id"] = self.artifact_id
        if self.phase is not None:
            payload["phase"] = self.phase
        if self.tool_call is not None:
            payload["tool_call"] = self.tool_call.to_dict()
        if self.request_user_input is not None:
            payload["request_user_input"] = self.request_user_input.to_dict()
        return payload

    @classmethod
    def from_dict(cls, payload: dict[str, Any]) -> "ConversationSegment":
        source_payload = payload.get("source")
        return cls(
            id=str(payload.get("id", "")),
            turn_id=str(payload.get("turn_id", "")),
            order=int(payload.get("order", 0) or 0),
            kind=str(payload.get("kind", "")),
            role=str(payload.get("role", "assistant") or "assistant"),
            status=str(payload.get("status", "complete") or "complete"),
            timestamp=str(payload.get("timestamp", "")),
            updated_at=str(payload.get("updated_at", payload.get("timestamp", "")) or ""),
            content=str(payload.get("content", "")),
            completed_at=str(payload.get("completed_at")) if payload.get("completed_at") is not None else None,
            error=str(payload.get("error")) if payload.get("error") is not None else None,
            error_code=str(payload.get("error_code")) if payload.get("error_code") is not None else None,
            artifact_id=str(payload.get("artifact_id")) if payload.get("artifact_id") is not None else None,
            phase=str(payload.get("phase")) if payload.get("phase") is not None else None,
            tool_call=ToolCallRecord.from_dict(payload.get("tool_call"))
            if isinstance(payload.get("tool_call"), dict)
            else None,
            request_user_input=RequestUserInputRecord.from_dict(payload.get("request_user_input"))
            if isinstance(payload.get("request_user_input"), dict)
            else None,
            source=ConversationSegmentSource.from_dict(source_payload)
            if isinstance(source_payload, dict)
            else ConversationSegmentSource(),
        )


@dataclass
class WorkflowEvent:
    message: str
    timestamp: str
    kind: Optional[str] = None
    error_code: Optional[str] = None
    details: Optional[dict[str, Any]] = None

    def to_dict(self) -> dict[str, Any]:
        payload: dict[str, Any] = {
            "message": self.message,
            "timestamp": self.timestamp,
        }
        if self.kind is not None:
            payload["kind"] = self.kind
        if self.error_code is not None:
            payload["error_code"] = self.error_code
        if self.details is not None:
            payload["details"] = dict(self.details)
        return payload

    @classmethod
    def from_dict(cls, payload: dict[str, Any]) -> "WorkflowEvent":
        return cls(
            message=str(payload.get("message", "")),
            timestamp=str(payload.get("timestamp", "")),
            kind=str(payload.get("kind")) if payload.get("kind") is not None else None,
            error_code=str(payload.get("error_code")) if payload.get("error_code") is not None else None,
            details=dict(payload.get("details")) if isinstance(payload.get("details"), dict) else None,
        )


@dataclass
class FlowRunRequest:
    id: str
    created_at: str
    updated_at: str
    flow_name: str
    summary: str
    project_path: str
    conversation_id: str
    source_turn_id: str
    status: str = "pending"
    source_segment_id: Optional[str] = None
    goal: Optional[str] = None
    launch_context: Optional[dict[str, Any]] = None
    model: Optional[str] = None
    llm_provider: Optional[str] = None
    llm_profile: Optional[str] = None
    reasoning_effort: Optional[str] = None
    execution_container_image: Optional[str] = None
    run_id: Optional[str] = None
    launch_error: Optional[str] = None
    review_message: Optional[str] = None

    def to_dict(self) -> dict[str, Any]:
        payload: dict[str, Any] = {
            "id": self.id,
            "created_at": self.created_at,
            "updated_at": self.updated_at,
            "flow_name": self.flow_name,
            "summary": self.summary,
            "project_path": self.project_path,
            "conversation_id": self.conversation_id,
            "source_turn_id": self.source_turn_id,
            "status": self.status,
        }
        if self.source_segment_id:
            payload["source_segment_id"] = self.source_segment_id
        if self.goal:
            payload["goal"] = self.goal
        if self.launch_context:
            payload["launch_context"] = copy.deepcopy(self.launch_context)
        if self.model:
            payload["model"] = self.model
        if self.llm_provider:
            payload["llm_provider"] = self.llm_provider
        if self.llm_profile:
            payload["llm_profile"] = self.llm_profile
        if self.reasoning_effort:
            payload["reasoning_effort"] = self.reasoning_effort
        if self.execution_container_image:
            payload["execution_container_image"] = self.execution_container_image
        if self.run_id:
            payload["run_id"] = self.run_id
        if self.launch_error:
            payload["launch_error"] = self.launch_error
        if self.review_message:
            payload["review_message"] = self.review_message
        return payload

    @classmethod
    def from_dict(cls, payload: dict[str, Any]) -> "FlowRunRequest":
        return cls(
            id=str(payload.get("id", "")),
            created_at=str(payload.get("created_at", "")),
            updated_at=str(payload.get("updated_at", payload.get("created_at", "")) or ""),
            flow_name=str(payload.get("flow_name", "")),
            summary=str(payload.get("summary", "")),
            project_path=_normalize_project_path(str(payload.get("project_path", ""))),
            conversation_id=str(payload.get("conversation_id", "")),
            source_turn_id=str(payload.get("source_turn_id", "")),
            status=str(payload.get("status", "pending") or "pending"),
            source_segment_id=str(payload.get("source_segment_id")) if payload.get("source_segment_id") is not None else None,
            goal=str(payload.get("goal")) if payload.get("goal") is not None else None,
            launch_context=copy.deepcopy(payload.get("launch_context")) if isinstance(payload.get("launch_context"), dict) else None,
            model=str(payload.get("model")) if payload.get("model") is not None else None,
            llm_provider=str(payload.get("llm_provider")) if payload.get("llm_provider") is not None else None,
            llm_profile=str(payload.get("llm_profile")) if payload.get("llm_profile") is not None else None,
            reasoning_effort=str(payload.get("reasoning_effort")) if payload.get("reasoning_effort") is not None else None,
            execution_container_image=(
                str(payload.get("execution_container_image"))
                if payload.get("execution_container_image") is not None
                else None
            ),
            run_id=str(payload.get("run_id")) if payload.get("run_id") is not None else None,
            launch_error=str(payload.get("launch_error")) if payload.get("launch_error") is not None else None,
            review_message=str(payload.get("review_message")) if payload.get("review_message") is not None else None,
        )


@dataclass
class FlowLaunch:
    id: str
    created_at: str
    updated_at: str
    flow_name: str
    summary: str
    project_path: str
    conversation_id: str
    source_turn_id: str
    status: str = "pending"
    source_segment_id: Optional[str] = None
    goal: Optional[str] = None
    launch_context: Optional[dict[str, Any]] = None
    model: Optional[str] = None
    llm_provider: Optional[str] = None
    llm_profile: Optional[str] = None
    reasoning_effort: Optional[str] = None
    execution_container_image: Optional[str] = None
    run_id: Optional[str] = None
    launch_error: Optional[str] = None

    def to_dict(self) -> dict[str, Any]:
        payload: dict[str, Any] = {
            "id": self.id,
            "created_at": self.created_at,
            "updated_at": self.updated_at,
            "flow_name": self.flow_name,
            "summary": self.summary,
            "project_path": self.project_path,
            "conversation_id": self.conversation_id,
            "source_turn_id": self.source_turn_id,
            "status": self.status,
        }
        if self.source_segment_id:
            payload["source_segment_id"] = self.source_segment_id
        if self.goal:
            payload["goal"] = self.goal
        if self.launch_context:
            payload["launch_context"] = copy.deepcopy(self.launch_context)
        if self.model:
            payload["model"] = self.model
        if self.llm_provider:
            payload["llm_provider"] = self.llm_provider
        if self.llm_profile:
            payload["llm_profile"] = self.llm_profile
        if self.reasoning_effort:
            payload["reasoning_effort"] = self.reasoning_effort
        if self.execution_container_image:
            payload["execution_container_image"] = self.execution_container_image
        if self.run_id:
            payload["run_id"] = self.run_id
        if self.launch_error:
            payload["launch_error"] = self.launch_error
        return payload

    @classmethod
    def from_dict(cls, payload: dict[str, Any]) -> "FlowLaunch":
        return cls(
            id=str(payload.get("id", "")),
            created_at=str(payload.get("created_at", "")),
            updated_at=str(payload.get("updated_at", payload.get("created_at", "")) or ""),
            flow_name=str(payload.get("flow_name", "")),
            summary=str(payload.get("summary", "")),
            project_path=_normalize_project_path(str(payload.get("project_path", ""))),
            conversation_id=str(payload.get("conversation_id", "")),
            source_turn_id=str(payload.get("source_turn_id", "")),
            status=str(payload.get("status", "pending") or "pending"),
            source_segment_id=str(payload.get("source_segment_id")) if payload.get("source_segment_id") is not None else None,
            goal=str(payload.get("goal")) if payload.get("goal") is not None else None,
            launch_context=copy.deepcopy(payload.get("launch_context")) if isinstance(payload.get("launch_context"), dict) else None,
            model=str(payload.get("model")) if payload.get("model") is not None else None,
            llm_provider=str(payload.get("llm_provider")) if payload.get("llm_provider") is not None else None,
            llm_profile=str(payload.get("llm_profile")) if payload.get("llm_profile") is not None else None,
            reasoning_effort=str(payload.get("reasoning_effort")) if payload.get("reasoning_effort") is not None else None,
            execution_container_image=(
                str(payload.get("execution_container_image"))
                if payload.get("execution_container_image") is not None
                else None
            ),
            run_id=str(payload.get("run_id")) if payload.get("run_id") is not None else None,
            launch_error=str(payload.get("launch_error")) if payload.get("launch_error") is not None else None,
        )


@dataclass
class ProposedPlanArtifact:
    id: str
    created_at: str
    updated_at: str
    title: str
    content: str
    project_path: str
    conversation_id: str
    source_turn_id: str
    status: str = "pending_review"
    source_segment_id: Optional[str] = None
    review_note: Optional[str] = None
    written_change_request_path: Optional[str] = None
    flow_launch_id: Optional[str] = None
    run_id: Optional[str] = None
    launch_error: Optional[str] = None

    def to_dict(self) -> dict[str, Any]:
        payload: dict[str, Any] = {
            "id": self.id,
            "created_at": self.created_at,
            "updated_at": self.updated_at,
            "title": self.title,
            "content": self.content,
            "project_path": self.project_path,
            "conversation_id": self.conversation_id,
            "source_turn_id": self.source_turn_id,
            "status": self.status,
        }
        if self.source_segment_id:
            payload["source_segment_id"] = self.source_segment_id
        if self.review_note:
            payload["review_note"] = self.review_note
        if self.written_change_request_path:
            payload["written_change_request_path"] = self.written_change_request_path
        if self.flow_launch_id:
            payload["flow_launch_id"] = self.flow_launch_id
        if self.run_id:
            payload["run_id"] = self.run_id
        if self.launch_error:
            payload["launch_error"] = self.launch_error
        return payload

    @classmethod
    def from_dict(cls, payload: dict[str, Any]) -> "ProposedPlanArtifact":
        return cls(
            id=str(payload.get("id", "")),
            created_at=str(payload.get("created_at", "")),
            updated_at=str(payload.get("updated_at", payload.get("created_at", "")) or ""),
            title=_as_non_empty_string(payload.get("title")) or "Proposed Plan",
            content=str(payload.get("content", "")),
            project_path=_normalize_project_path(str(payload.get("project_path", ""))),
            conversation_id=str(payload.get("conversation_id", "")),
            source_turn_id=str(payload.get("source_turn_id", "")),
            status=str(payload.get("status", "pending_review") or "pending_review"),
            source_segment_id=str(payload.get("source_segment_id")) if payload.get("source_segment_id") is not None else None,
            review_note=str(payload.get("review_note")) if payload.get("review_note") is not None else None,
            written_change_request_path=(
                str(payload.get("written_change_request_path"))
                if payload.get("written_change_request_path") is not None
                else None
            ),
            flow_launch_id=str(payload.get("flow_launch_id")) if payload.get("flow_launch_id") is not None else None,
            run_id=str(payload.get("run_id")) if payload.get("run_id") is not None else None,
            launch_error=str(payload.get("launch_error")) if payload.get("launch_error") is not None else None,
        )


@dataclass
class ConversationState:
    conversation_id: str
    project_path: str
    chat_mode: str = CHAT_MODE_CHAT
    provider: str = "codex"
    model: Optional[str] = None
    llm_profile: Optional[str] = None
    reasoning_effort: Optional[str] = None
    conversation_handle: str = ""
    title: str = "New thread"
    created_at: str = ""
    updated_at: str = ""
    schema_version: int = CONVERSATION_STATE_SCHEMA_VERSION
    revision: int = 0
    turns: list[ConversationTurn] = field(default_factory=list)
    segments: list[ConversationSegment] = field(default_factory=list)
    event_log: list[WorkflowEvent] = field(default_factory=list)
    flow_run_requests: list[FlowRunRequest] = field(default_factory=list)
    flow_launches: list[FlowLaunch] = field(default_factory=list)
    proposed_plans: list[ProposedPlanArtifact] = field(default_factory=list)

    def normalize_request_user_input_state(self) -> bool:
        changed = False
        turns_by_id = {turn.id: turn for turn in self.turns}
        for segment in self.segments:
            if segment.kind != "request_user_input" or segment.request_user_input is None:
                continue
            request = segment.request_user_input
            if request.status != "expired":
                continue
            if segment.status != "failed":
                segment.status = "failed"
                changed = True
            if segment.error != REQUEST_USER_INPUT_EXPIRED_ERROR:
                segment.error = REQUEST_USER_INPUT_EXPIRED_ERROR
                changed = True
            if segment.completed_at is None:
                segment.completed_at = request.submitted_at or segment.updated_at or segment.timestamp
                changed = True
            target_turn = turns_by_id.get(segment.turn_id)
            if target_turn is None or target_turn.role != "assistant":
                continue
            if target_turn.status not in {"pending", "streaming", "failed"}:
                continue
            if target_turn.status != "failed":
                target_turn.status = "failed"
                changed = True
            if target_turn.error != REQUEST_USER_INPUT_EXPIRED_ERROR:
                target_turn.error = REQUEST_USER_INPUT_EXPIRED_ERROR
                changed = True
        return changed

    def to_dict(self) -> dict[str, Any]:
        return {
            "schema_version": self.schema_version,
            "revision": self.revision,
            "conversation_id": self.conversation_id,
            "conversation_handle": self.conversation_handle,
            "project_path": self.project_path,
            "chat_mode": self.chat_mode,
            "provider": self.provider,
            "model": self.model,
            "llm_profile": self.llm_profile,
            "reasoning_effort": self.reasoning_effort,
            "title": self.title,
            "created_at": self.created_at,
            "updated_at": self.updated_at,
            "turns": [turn.to_dict() for turn in self.turns],
            "segments": [segment.to_dict() for segment in self.segments],
            "event_log": [entry.to_dict() for entry in self.event_log],
            "flow_run_requests": [request.to_dict() for request in self.flow_run_requests],
            "flow_launches": [launch.to_dict() for launch in self.flow_launches],
            "proposed_plans": [artifact.to_dict() for artifact in self.proposed_plans],
        }

    @classmethod
    def from_dict(cls, payload: dict[str, Any]) -> "ConversationState":
        raw_turns = payload.get("turns")
        raw_segments = payload.get("segments")
        raw_events = payload.get("event_log")
        raw_flow_run_requests = payload.get("flow_run_requests")
        raw_flow_launches = payload.get("flow_launches")
        raw_proposed_plans = payload.get("proposed_plans")
        schema_version = payload.get("schema_version")
        if not isinstance(schema_version, int) or schema_version != CONVERSATION_STATE_SCHEMA_VERSION:
            raise ValueError(
                "Unsupported conversation state schema. Delete the local conversation and recreate it."
            )
        revision = payload.get("revision")
        if not isinstance(revision, int) or isinstance(revision, bool):
            raise ValueError(
                "Unsupported conversation state schema. Delete the local conversation and recreate it."
            )
        if not isinstance(raw_segments, list):
            raise ValueError(
                "Unsupported conversation state payload: missing canonical segments. Delete the local conversation and recreate it."
            )
        turns = [
            ConversationTurn.from_dict(turn)
            for turn in raw_turns
            if isinstance(turn, dict)
        ] if isinstance(raw_turns, list) else []
        created_at = _as_non_empty_string(payload.get("created_at"))
        updated_at = _as_non_empty_string(payload.get("updated_at"))
        if not created_at:
            created_at = turns[0].timestamp if turns else ""
        if not updated_at:
            updated_at = turns[-1].timestamp if turns else created_at
        return cls(
            conversation_id=str(payload.get("conversation_id", "")),
            conversation_handle=str(payload.get("conversation_handle", "") or ""),
            project_path=_normalize_project_path(str(payload.get("project_path", ""))),
            chat_mode=normalize_chat_mode(payload.get("chat_mode")),
            provider=_as_non_empty_string(payload.get("provider")) or "codex",
            model=_as_non_empty_string(payload.get("model")),
            llm_profile=_as_non_empty_string(payload.get("llm_profile")),
            reasoning_effort=normalize_reasoning_effort(payload.get("reasoning_effort")),
            title=_as_non_empty_string(payload.get("title")) or _derive_conversation_title(turns),
            created_at=created_at or _iso_now(),
            updated_at=updated_at or created_at or _iso_now(),
            schema_version=schema_version,
            revision=revision,
            turns=turns,
            segments=[
                ConversationSegment.from_dict(segment)
                for segment in raw_segments
                if isinstance(segment, dict)
            ],
            event_log=[
                WorkflowEvent.from_dict(entry)
                for entry in raw_events
                if isinstance(entry, dict)
            ] if isinstance(raw_events, list) else [],
            flow_run_requests=[
                FlowRunRequest.from_dict(entry)
                for entry in raw_flow_run_requests
                if isinstance(entry, dict)
            ] if isinstance(raw_flow_run_requests, list) else [],
            flow_launches=[
                FlowLaunch.from_dict(entry)
                for entry in raw_flow_launches
                if isinstance(entry, dict)
            ] if isinstance(raw_flow_launches, list) else [],
            proposed_plans=[
                ProposedPlanArtifact.from_dict(entry)
                for entry in raw_proposed_plans
                if isinstance(entry, dict)
            ] if isinstance(raw_proposed_plans, list) else [],
        )


@dataclass
class ConversationSummary:
    conversation_id: str
    conversation_handle: str
    project_path: str
    title: str
    created_at: str
    updated_at: str
    revision: int
    last_message_preview: Optional[str] = None

    def to_dict(self) -> dict[str, Any]:
        payload: dict[str, Any] = {
            "conversation_id": self.conversation_id,
            "conversation_handle": self.conversation_handle,
            "project_path": self.project_path,
            "title": self.title,
            "created_at": self.created_at,
            "updated_at": self.updated_at,
            "revision": self.revision,
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
    session_version: int = CHAT_SESSION_VERSION
    provider: str = "codex"
    thread_id: Optional[str] = None
    model: Optional[str] = None

    def to_dict(self) -> dict[str, Any]:
        payload: dict[str, Any] = {
            "conversation_id": self.conversation_id,
            "updated_at": self.updated_at,
            "project_path": self.project_path,
            "runtime_project_path": self.runtime_project_path,
            "session_version": self.session_version,
            "provider": self.provider,
        }
        if self.thread_id:
            payload["thread_id"] = self.thread_id
        if self.model:
            payload["model"] = self.model
        return payload

    @classmethod
    def from_dict(cls, payload: dict[str, Any]) -> "ConversationSessionState":
        session_version = payload.get("session_version")
        if not isinstance(session_version, int) or session_version != CHAT_SESSION_VERSION:
            raise ValueError(
                "Unsupported conversation session schema. Delete the local conversation session and recreate it."
            )
        return cls(
            conversation_id=str(payload.get("conversation_id", "")),
            updated_at=str(payload.get("updated_at", "")),
            project_path=_normalize_project_path(str(payload.get("project_path", ""))),
            runtime_project_path=_normalize_project_path(str(payload.get("runtime_project_path", ""))),
            session_version=session_version,
            provider=_as_non_empty_string(payload.get("provider")) or "codex",
            thread_id=str(payload.get("thread_id")) if payload.get("thread_id") is not None else None,
            model=_as_non_empty_string(payload.get("model")),
        )


@dataclass(frozen=True)
class _ConversationEventSubscriber:
    loop: asyncio.AbstractEventLoop
    queue: asyncio.Queue[dict[str, Any]]


class ConversationEventHub:
    def __init__(self) -> None:
        self._lock = threading.Lock()
        self._subscribers: dict[str, list[_ConversationEventSubscriber]] = {}

    @staticmethod
    def _publish_to_queue(queue: asyncio.Queue[dict[str, Any]], payload: dict[str, Any]) -> None:
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
                pass

    def subscribe(self, conversation_id: str) -> asyncio.Queue[dict[str, Any]]:
        queue: asyncio.Queue[dict[str, Any]] = asyncio.Queue(maxsize=32)
        subscriber = _ConversationEventSubscriber(
            loop=asyncio.get_running_loop(),
            queue=queue,
        )
        with self._lock:
            self._subscribers.setdefault(conversation_id, []).append(subscriber)
        return queue

    def unsubscribe(self, conversation_id: str, queue: asyncio.Queue[dict[str, Any]]) -> None:
        with self._lock:
            listeners = self._subscribers.get(conversation_id)
            if not listeners:
                return
            remaining = [listener for listener in listeners if listener.queue is not queue]
            if remaining:
                self._subscribers[conversation_id] = remaining
            else:
                self._subscribers.pop(conversation_id, None)

    def publish_nowait(self, conversation_id: str, payload: dict[str, Any]) -> None:
        try:
            current_loop = asyncio.get_running_loop()
        except RuntimeError:
            current_loop = None
        with self._lock:
            listeners = list(self._subscribers.get(conversation_id, []))
        stale_queues: list[asyncio.Queue[dict[str, Any]]] = []
        for listener in listeners:
            if listener.loop.is_closed():
                stale_queues.append(listener.queue)
                continue
            try:
                if current_loop is listener.loop:
                    self._publish_to_queue(listener.queue, payload)
                else:
                    listener.loop.call_soon_threadsafe(self._publish_to_queue, listener.queue, payload)
            except RuntimeError:
                stale_queues.append(listener.queue)
        if not stale_queues:
            return
        stale_queue_ids = {id(queue) for queue in stale_queues}
        with self._lock:
            listeners = self._subscribers.get(conversation_id)
            if not listeners:
                return
            remaining = [listener for listener in listeners if id(listener.queue) not in stale_queue_ids]
            if remaining:
                self._subscribers[conversation_id] = remaining
            else:
                self._subscribers.pop(conversation_id, None)

    async def publish(self, conversation_id: str, payload: dict[str, Any]) -> None:
        self.publish_nowait(conversation_id, payload)
