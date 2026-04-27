from __future__ import annotations

import json
import hashlib
import shutil
import threading
from pathlib import Path
from typing import Any, Optional

from spark.workspace.conversations.models import (
    CONVERSATION_STATE_SCHEMA_VERSION,
    ConversationSegment,
    ConversationSessionState,
    ConversationState,
    ConversationSummary,
    ConversationTurn,
    WorkflowEvent,
    normalize_chat_mode,
)
from spark.workspace.conversations.utils import (
    as_non_empty_string,
    build_conversation_preview,
    derive_conversation_title,
    iso_now,
    normalize_project_path_value,
    truncate_text,
)
from spark.workspace.storage import (
    ProjectPaths,
    ensure_conversation_handle,
    ensure_project_paths,
    find_conversation_by_handle,
    normalize_conversation_handle,
    read_project_paths_by_id,
    remove_conversation_handle,
    workspace_projects_root,
)
from spark_common.runtime_path import resolve_runtime_workspace_path


class ProjectChatRepository:
    def __init__(self, data_dir: Path, lock: threading.Lock) -> None:
        self._data_dir = data_dir
        self._lock = lock

    def projects_root(self) -> Path:
        return workspace_projects_root(self._data_dir)

    def project_paths(self, project_path: str) -> ProjectPaths:
        return ensure_project_paths(self._data_dir, project_path)

    def project_paths_for_conversation(
        self,
        conversation_id: str,
        project_path: Optional[str] = None,
    ) -> ProjectPaths:
        if project_path:
            return self.project_paths(project_path)
        candidates: list[ProjectPaths] = []
        for project_root in self.projects_root().iterdir():
            if not project_root.is_dir():
                continue
            project_record = read_project_paths_by_id(self._data_dir, project_root.name)
            if project_record is None:
                continue
            if (project_record.conversations_dir / conversation_id).exists():
                candidates.append(project_record)
        if len(candidates) == 1:
            return candidates[0]
        if not candidates:
            raise FileNotFoundError(conversation_id)
        raise RuntimeError(f"Conversation id is ambiguous across projects: {conversation_id}")

    def resolve_conversation_handle(self, conversation_handle: str) -> tuple[str, str]:
        normalized_handle = normalize_conversation_handle(conversation_handle)
        if not normalized_handle:
            raise ValueError(
                "Conversation handle must use the adjective-noun form, for example 'amber-otter'."
            )
        match = find_conversation_by_handle(self._data_dir, normalized_handle)
        if match is None:
            raise FileNotFoundError(normalized_handle)
        project_path = normalize_project_path_value(match["project_path"])
        if not project_path:
            raise FileNotFoundError(normalized_handle)
        return match["conversation_id"], project_path

    def conversation_root(self, conversation_id: str, project_path: Optional[str] = None) -> Path:
        project_paths = self.project_paths_for_conversation(conversation_id, project_path)
        root = project_paths.conversations_dir / conversation_id
        root.mkdir(parents=True, exist_ok=True)
        return root

    def conversation_state_path(self, conversation_id: str, project_path: Optional[str] = None) -> Path:
        return self.conversation_root(conversation_id, project_path) / "state.json"

    def conversation_session_path(self, conversation_id: str, project_path: Optional[str] = None) -> Path:
        return self.conversation_root(conversation_id, project_path) / "session.json"

    def conversation_keyed_session_path(
        self,
        conversation_id: str,
        project_path: Optional[str] = None,
        *,
        provider: str,
        model: Optional[str],
    ) -> Path:
        normalized_provider = as_non_empty_string(provider).lower() if as_non_empty_string(provider) else "codex"
        normalized_model = as_non_empty_string(model) or ""
        digest = hashlib.sha256(f"{normalized_provider}\0{normalized_model}".encode("utf-8")).hexdigest()[:24]
        return self.conversation_root(conversation_id, project_path) / "sessions" / f"{normalized_provider}-{digest}.json"

    def conversation_raw_log_path(self, conversation_id: str, project_path: Optional[str] = None) -> Path:
        return self.conversation_root(conversation_id, project_path) / "raw-log.jsonl"

    def flow_run_requests_state_path(self, conversation_id: str, project_path: Optional[str] = None) -> Path:
        project_paths = self.project_paths_for_conversation(conversation_id, project_path)
        return project_paths.flow_run_requests_dir / f"{conversation_id}.json"

    def flow_launches_state_path(self, conversation_id: str, project_path: Optional[str] = None) -> Path:
        project_paths = self.project_paths_for_conversation(conversation_id, project_path)
        return project_paths.flow_launches_dir / f"{conversation_id}.json"

    def proposed_plans_state_path(self, conversation_id: str, project_path: Optional[str] = None) -> Path:
        project_paths = self.project_paths_for_conversation(conversation_id, project_path)
        return project_paths.proposed_plans_dir / f"{conversation_id}.json"

    def touch_conversation_state(self, state: ConversationState, *, title_hint: Optional[str] = None) -> None:
        self.ensure_state_handle(state)
        state.chat_mode = normalize_chat_mode(state.chat_mode)
        if not state.created_at:
            state.created_at = iso_now()
        if title_hint:
            normalized_title_hint = truncate_text(title_hint, 64)
            if state.title == "New thread" or not as_non_empty_string(state.title):
                state.title = normalized_title_hint
        elif not as_non_empty_string(state.title):
            state.title = derive_conversation_title(state.turns)
        if state.title == "New thread":
            derived_title = derive_conversation_title(state.turns)
            if derived_title != "New thread":
                state.title = derived_title
        state.updated_at = iso_now()

    def build_conversation_summary(self, state: ConversationState) -> ConversationSummary:
        self.ensure_state_handle(state)
        return ConversationSummary(
            conversation_id=state.conversation_id,
            conversation_handle=state.conversation_handle,
            project_path=state.project_path,
            title=as_non_empty_string(state.title) or derive_conversation_title(state.turns),
            created_at=state.created_at or iso_now(),
            updated_at=state.updated_at or state.created_at or iso_now(),
            last_message_preview=build_conversation_preview(state.turns),
        )

    def read_state(self, conversation_id: str, project_path: Optional[str] = None) -> Optional[ConversationState]:
        try:
            path = self.conversation_state_path(conversation_id, project_path)
        except (FileNotFoundError, RuntimeError):
            return None
        try:
            payload = json.loads(path.read_text(encoding="utf-8"))
        except Exception:
            return None
        if not isinstance(payload, dict):
            return None
        normalized_project_path = normalize_project_path_value(str(payload.get("project_path", "")))
        flow_run_requests_payload = self.read_json_dict(self.flow_run_requests_state_path(conversation_id, normalized_project_path))
        flow_launches_payload = self.read_json_dict(self.flow_launches_state_path(conversation_id, normalized_project_path))
        proposed_plans_payload = self.read_json_dict(self.proposed_plans_state_path(conversation_id, normalized_project_path))
        if flow_run_requests_payload:
            payload["event_log"] = flow_run_requests_payload.get("event_log", payload.get("event_log", []))
            payload["flow_run_requests"] = flow_run_requests_payload.get("flow_run_requests", [])
        if flow_launches_payload:
            payload["flow_launches"] = flow_launches_payload.get("flow_launches", [])
        if proposed_plans_payload:
            payload["proposed_plans"] = proposed_plans_payload.get("proposed_plans", [])
        state = ConversationState.from_dict(payload)
        should_write_state = state.normalize_request_user_input_state()
        if not state.conversation_id:
            state.conversation_id = conversation_id
        self.ensure_state_handle(state)
        if payload.get("conversation_handle") != state.conversation_handle:
            should_write_state = True
        if should_write_state:
            self.write_state(state)
        return state

    def write_state(self, state: ConversationState) -> None:
        project_paths = self.project_paths(state.project_path)
        self.ensure_state_handle(state)
        path = self.conversation_state_path(state.conversation_id, state.project_path)
        path.parent.mkdir(parents=True, exist_ok=True)
        conversation_payload = {
            "schema_version": CONVERSATION_STATE_SCHEMA_VERSION,
            "conversation_id": state.conversation_id,
            "conversation_handle": state.conversation_handle,
            "project_path": state.project_path,
            "chat_mode": state.chat_mode,
            "provider": state.provider,
            "model": state.model,
            "reasoning_effort": state.reasoning_effort,
            "title": state.title,
            "created_at": state.created_at,
            "updated_at": state.updated_at,
            "turns": [turn.to_dict() for turn in state.turns],
            "segments": [segment.to_dict() for segment in state.segments],
        }
        path.write_text(json.dumps(conversation_payload, indent=2, sort_keys=True), encoding="utf-8")
        self.write_json(
            self.flow_run_requests_state_path(state.conversation_id, state.project_path),
            {
                "conversation_id": state.conversation_id,
                "project_id": project_paths.project_id,
                "project_path": state.project_path,
                "event_log": [entry.to_dict() for entry in state.event_log],
                "flow_run_requests": [request.to_dict() for request in state.flow_run_requests],
            },
        )
        self.write_json(
            self.flow_launches_state_path(state.conversation_id, state.project_path),
            {
                "conversation_id": state.conversation_id,
                "project_id": project_paths.project_id,
                "project_path": state.project_path,
                "flow_launches": [launch.to_dict() for launch in state.flow_launches],
            },
        )
        self.write_json(
            self.proposed_plans_state_path(state.conversation_id, state.project_path),
            {
                "conversation_id": state.conversation_id,
                "project_id": project_paths.project_id,
                "project_path": state.project_path,
                "proposed_plans": [artifact.to_dict() for artifact in state.proposed_plans],
            },
        )

    def read_json_dict(self, path: Path) -> dict[str, Any]:
        try:
            payload = json.loads(path.read_text(encoding="utf-8"))
        except Exception:
            return {}
        return payload if isinstance(payload, dict) else {}

    def write_json(self, path: Path, payload: dict[str, Any]) -> None:
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(json.dumps(payload, indent=2, sort_keys=True), encoding="utf-8")

    def append_raw_rpc_log(
        self,
        conversation_id: str,
        project_path: str,
        *,
        direction: str,
        line: str,
    ) -> None:
        path = self.conversation_raw_log_path(conversation_id, project_path)
        path.parent.mkdir(parents=True, exist_ok=True)
        payload = {
            "timestamp": iso_now(),
            "direction": direction,
            "line": line,
        }
        with path.open("a", encoding="utf-8") as handle:
            handle.write(json.dumps(payload, sort_keys=True) + "\n")

    def read_session_state(
        self,
        conversation_id: str,
        project_path: Optional[str] = None,
        *,
        provider: Optional[str] = None,
        model: Optional[str] = None,
    ) -> Optional[ConversationSessionState]:
        try:
            path = (
                self.conversation_keyed_session_path(
                    conversation_id,
                    project_path,
                    provider=provider,
                    model=model,
                )
                if provider is not None or model is not None
                else self.conversation_session_path(conversation_id, project_path)
            )
        except (FileNotFoundError, RuntimeError):
            return None
        try:
            payload = json.loads(path.read_text(encoding="utf-8"))
        except Exception:
            return None
        if not isinstance(payload, dict):
            return None
        try:
            state = ConversationSessionState.from_dict(payload)
        except ValueError:
            return None
        if not state.conversation_id:
            state.conversation_id = conversation_id
        if provider is not None and state.provider != (as_non_empty_string(provider).lower() if as_non_empty_string(provider) else "codex"):
            return None
        if model is not None and (state.model or "") != (as_non_empty_string(model) or ""):
            return None
        return state

    def write_session_state(self, state: ConversationSessionState) -> None:
        path = self.conversation_keyed_session_path(
            state.conversation_id,
            state.project_path,
            provider=state.provider,
            model=state.model,
        )
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(json.dumps(state.to_dict(), indent=2, sort_keys=True), encoding="utf-8")
        legacy_path = self.conversation_session_path(state.conversation_id, state.project_path)
        legacy_path.parent.mkdir(parents=True, exist_ok=True)
        legacy_path.write_text(json.dumps(state.to_dict(), indent=2, sort_keys=True), encoding="utf-8")

    def persist_session_thread(
        self,
        conversation_id: str,
        project_path: str,
        thread_id: str,
        *,
        provider: str = "codex",
        model: Optional[str] = None,
    ) -> None:
        normalized_project_path = normalize_project_path_value(project_path)
        runtime_project_path = resolve_runtime_workspace_path(normalized_project_path)
        normalized_provider = as_non_empty_string(provider).lower() if as_non_empty_string(provider) else "codex"
        normalized_model = as_non_empty_string(model)
        with self._lock:
            session_state = self.read_session_state(
                conversation_id,
                normalized_project_path,
                provider=normalized_provider,
                model=normalized_model,
            )
            if session_state is None:
                session_state = ConversationSessionState(
                    conversation_id=conversation_id,
                    updated_at=iso_now(),
                    project_path=normalized_project_path,
                    runtime_project_path=runtime_project_path,
                    provider=normalized_provider,
                    model=normalized_model,
                )
            session_state.thread_id = thread_id
            session_state.project_path = normalized_project_path
            session_state.runtime_project_path = runtime_project_path
            session_state.provider = normalized_provider
            session_state.model = normalized_model
            session_state.updated_at = iso_now()
            self.write_session_state(session_state)

    def persist_session_model(
        self,
        conversation_id: str,
        project_path: str,
        model: str,
        *,
        provider: str = "codex",
    ) -> None:
        normalized_project_path = normalize_project_path_value(project_path)
        runtime_project_path = resolve_runtime_workspace_path(normalized_project_path)
        normalized_model = as_non_empty_string(model)
        if not normalized_model:
            return
        normalized_provider = as_non_empty_string(provider).lower() if as_non_empty_string(provider) else "codex"
        with self._lock:
            session_state = self.read_session_state(
                conversation_id,
                normalized_project_path,
                provider=normalized_provider,
                model=normalized_model,
            )
            if session_state is None:
                session_state = ConversationSessionState(
                    conversation_id=conversation_id,
                    updated_at=iso_now(),
                    project_path=normalized_project_path,
                    runtime_project_path=runtime_project_path,
                    provider=normalized_provider,
                )
            session_state.model = normalized_model
            session_state.provider = normalized_provider
            session_state.project_path = normalized_project_path
            session_state.runtime_project_path = runtime_project_path
            session_state.updated_at = iso_now()
            self.write_session_state(session_state)

    def clear_session_thread(
        self,
        conversation_id: str,
        project_path: str,
    ) -> None:
        normalized_project_path = normalize_project_path_value(project_path)
        with self._lock:
            session_state = self.read_session_state(conversation_id, normalized_project_path)
            if session_state is None:
                return
            session_state.thread_id = None
            session_state.project_path = normalized_project_path
            session_state.runtime_project_path = resolve_runtime_workspace_path(normalized_project_path)
            session_state.updated_at = iso_now()
            self.write_session_state(session_state)

    def list_conversations(self, project_path: str) -> list[dict[str, Any]]:
        normalized_project_path = normalize_project_path_value(project_path)
        if not normalized_project_path:
            raise ValueError("Project path is required.")
        project_paths = self.project_paths(normalized_project_path)
        with self._lock:
            summaries: list[ConversationSummary] = []
            for state_path in project_paths.conversations_dir.glob("*/state.json"):
                try:
                    state = self.read_state(state_path.parent.name, normalized_project_path)
                except ValueError:
                    continue
                if state is None:
                    continue
                if state.project_path != normalized_project_path:
                    continue
                summaries.append(self.build_conversation_summary(state))
        summaries.sort(key=lambda entry: entry.updated_at, reverse=True)
        return [summary.to_dict() for summary in summaries]

    def get_snapshot(self, conversation_id: str, project_path: Optional[str] = None) -> dict[str, Any]:
        with self._lock:
            should_write_state = False
            state = self.read_state(conversation_id, project_path)
            if state is None:
                normalized_project_path = normalize_project_path_value(project_path or "")
                if not normalized_project_path:
                    raise FileNotFoundError(conversation_id)
                state = ConversationState(
                    conversation_id=conversation_id,
                    project_path=normalized_project_path,
                )
                self.touch_conversation_state(state)
                should_write_state = True
            elif project_path:
                normalized_project_path = normalize_project_path_value(project_path)
                if normalized_project_path and normalized_project_path != state.project_path:
                    raise ValueError("Conversation is already bound to a different project path.")
            if not state.created_at or not state.updated_at or not as_non_empty_string(state.title):
                if not state.created_at:
                    state.created_at = iso_now()
                if not state.updated_at:
                    state.updated_at = state.created_at
                if not as_non_empty_string(state.title):
                    state.title = derive_conversation_title(state.turns)
                should_write_state = True
            if should_write_state:
                self.write_state(state)
        return state.to_dict()

    def delete_conversation(self, conversation_id: str, project_path: str) -> dict[str, Any]:
        normalized_project_path = normalize_project_path_value(project_path)
        if not normalized_project_path:
            raise ValueError("Project path is required.")

        project_paths = self.project_paths(normalized_project_path)
        conversation_root = project_paths.conversations_dir / conversation_id
        with self._lock:
            state = self.read_state(conversation_id, normalized_project_path)
            if state is None or state.project_path != normalized_project_path:
                raise FileNotFoundError(conversation_id)

        if conversation_root.exists():
            shutil.rmtree(conversation_root)
        for sidecar in (
            project_paths.flow_run_requests_dir / f"{conversation_id}.json",
            project_paths.flow_launches_dir / f"{conversation_id}.json",
            project_paths.proposed_plans_dir / f"{conversation_id}.json",
        ):
            sidecar.unlink(missing_ok=True)
        remove_conversation_handle(self._data_dir, conversation_id)

        return {
            "status": "deleted",
            "conversation_id": conversation_id,
            "project_path": normalized_project_path,
        }

    def ensure_state_handle(self, state: ConversationState) -> str:
        normalized_project_path = normalize_project_path_value(state.project_path)
        if not normalized_project_path:
            raise ValueError("Project path is required.")
        project_paths = self.project_paths(normalized_project_path)
        if not state.created_at:
            state.created_at = iso_now()
        handle = ensure_conversation_handle(
            self._data_dir,
            conversation_id=state.conversation_id,
            project_id=project_paths.project_id,
            project_path=normalized_project_path,
            created_at=state.created_at,
            preferred_handle=state.conversation_handle,
        )
        state.project_path = normalized_project_path
        state.conversation_handle = handle
        return handle

    def append_event(self, state: ConversationState, message: str) -> None:
        state.event_log.append(WorkflowEvent(message=message, timestamp=iso_now()))

    def append_workflow_event(self, state: ConversationState, event: WorkflowEvent) -> None:
        state.event_log.append(
            WorkflowEvent(
                message=event.message,
                timestamp=event.timestamp or iso_now(),
                kind=event.kind,
                error_code=event.error_code,
                details=dict(event.details) if event.details is not None else None,
            )
        )

    def next_turn_segment_order(self, state: ConversationState, turn_id: str) -> int:
        max_order = 0
        for segment in state.segments:
            if segment.turn_id == turn_id and segment.order > max_order:
                max_order = segment.order
        return max_order + 1

    def upsert_segment(self, state: ConversationState, segment: ConversationSegment) -> None:
        for index, existing_segment in enumerate(state.segments):
            if existing_segment.id != segment.id:
                continue
            state.segments[index] = segment
            return
        state.segments.append(segment)

    def get_segment(self, state: ConversationState, segment_id: str) -> Optional[ConversationSegment]:
        for segment in state.segments:
            if segment.id == segment_id:
                return segment
        return None

    def upsert_turn(self, state: ConversationState, turn: ConversationTurn) -> None:
        for index, existing_turn in enumerate(state.turns):
            if existing_turn.id != turn.id:
                continue
            state.turns[index] = turn
            return
        state.turns.append(turn)

    def get_turn(self, state: ConversationState, turn_id: str) -> Optional[ConversationTurn]:
        for turn in state.turns:
            if turn.id == turn_id:
                return turn
        return None

    def build_turn_upsert_payload(
        self,
        state: ConversationState,
        turn: ConversationTurn,
    ) -> dict[str, Any]:
        serialized_turn = turn.to_dict()
        if (
            turn.role == "assistant"
            and turn.status in {"pending", "streaming"}
            and not as_non_empty_string(turn.content)
        ):
            serialized_turn["content"] = ""
        return {
            "type": "turn_upsert",
            "conversation_id": state.conversation_id,
            "project_path": state.project_path,
            "title": state.title,
            "updated_at": state.updated_at,
            "turn": serialized_turn,
        }

    def build_segment_upsert_payload(
        self,
        state: ConversationState,
        segment: ConversationSegment,
    ) -> dict[str, Any]:
        return {
            "type": "segment_upsert",
            "conversation_id": state.conversation_id,
            "project_path": state.project_path,
            "title": state.title,
            "updated_at": state.updated_at,
            "segment": segment.to_dict(),
        }
