from __future__ import annotations

import copy
import re
import unicodedata
import uuid
from pathlib import Path
from typing import Optional

from spark.workspace.conversations.models import (
    ConversationSegment,
    ConversationSegmentSource,
    ConversationState,
    ConversationTurn,
    FlowLaunch,
    FlowRunRequest,
    ProposedPlanArtifact,
)
from spark.workspace.conversations.repository import ProjectChatRepository
from spark.workspace.conversations.utils import (
    iso_now,
    normalize_project_path_value,
)

IMPLEMENT_CHANGE_REQUEST_FLOW = "software-development/implement-change-request.dot"
_PLAN_HEADING_PATTERN = re.compile(r"(?m)^\s*#\s+(.+?)\s*$")
_MARKDOWN_LINK_PATTERN = re.compile(r"\[([^\]]+)\]\([^)]+\)")
_MARKDOWN_DECORATION_PATTERN = re.compile(r"[*_~`#>\[\]()!]")
_NON_SLUG_PATTERN = re.compile(r"[^a-z0-9]+")
_CHANGE_REQUEST_DIR_PATTERN = re.compile(r"^CR-(\d{4})-(\d{4})(?:-.+)?$")


def _strip_markdown_text(value: str) -> str:
    plain = _MARKDOWN_LINK_PATTERN.sub(r"\1", value)
    plain = _MARKDOWN_DECORATION_PATTERN.sub("", plain)
    return " ".join(plain.split()).strip()


def _proposed_plan_title(content: str) -> str:
    match = _PLAN_HEADING_PATTERN.search(content)
    if match is None:
        return "Proposed Plan"
    title = _strip_markdown_text(match.group(1) or "")
    return title or "Proposed Plan"


def _slugify_filename_base(value: str) -> str:
    normalized = unicodedata.normalize("NFKD", value).encode("ascii", "ignore").decode("ascii").lower()
    collapsed = _NON_SLUG_PATTERN.sub("-", normalized).strip("-")
    return collapsed or "change-request"


def _allocate_change_request_dir(project_root: Path, title: str, *, created_at: str) -> tuple[str, Path]:
    changes_dir = project_root / "changes"
    changes_dir.mkdir(parents=True, exist_ok=True)
    year = created_at[:4] if len(created_at) >= 4 and created_at[:4].isdigit() else "0000"
    max_sequence = 0
    for child in changes_dir.iterdir():
        if not child.is_dir():
            continue
        match = _CHANGE_REQUEST_DIR_PATTERN.match(child.name)
        if match is None or match.group(1) != year:
            continue
        max_sequence = max(max_sequence, int(match.group(2)))
    base_name = _slugify_filename_base(title)
    sequence = max_sequence + 1
    while True:
        change_request_id = f"CR-{year}-{sequence:04d}-{base_name}"
        candidate = changes_dir / change_request_id
        if not candidate.exists():
            candidate.mkdir(parents=True, exist_ok=False)
            return change_request_id, candidate
        sequence += 1


class ProjectChatReviewService:
    def __init__(self, repository: ProjectChatRepository) -> None:
        self._repository = repository

    def persist_flow_run_request(
        self,
        state: ConversationState,
        parent_turn: ConversationTurn,
        flow_run_request_payload: dict[str, object],
    ) -> Optional[tuple[FlowRunRequest, ConversationSegment]]:
        flow_name = str(flow_run_request_payload.get("flow_name", "")).strip()
        summary = str(flow_run_request_payload.get("summary", "")).strip()
        if not flow_name or not summary:
            return None
        goal = str(flow_run_request_payload.get("goal", "")).strip() or None
        launch_context = copy.deepcopy(flow_run_request_payload.get("launch_context")) if isinstance(flow_run_request_payload.get("launch_context"), dict) else None
        model = str(flow_run_request_payload.get("model", "")).strip() or None
        llm_provider = str(flow_run_request_payload.get("llm_provider", "")).strip().lower() or None
        llm_profile = str(flow_run_request_payload.get("llm_profile", "")).strip() or None
        reasoning_effort = str(flow_run_request_payload.get("reasoning_effort", "")).strip().lower() or None
        execution_container_image = str(flow_run_request_payload.get("execution_container_image", "")).strip() or None

        requests_by_id = {request.id: request for request in state.flow_run_requests}
        for segment in state.segments:
            if segment.turn_id != parent_turn.id or segment.kind != "flow_run_request" or not segment.artifact_id:
                continue
            existing_request = requests_by_id.get(segment.artifact_id)
            if existing_request is None:
                continue
            if (
                existing_request.flow_name == flow_name
                and existing_request.summary == summary
                and existing_request.goal == goal
                and existing_request.launch_context == launch_context
                and existing_request.model == model
                and existing_request.llm_provider == llm_provider
                and existing_request.llm_profile == llm_profile
                and existing_request.reasoning_effort == reasoning_effort
                and existing_request.execution_container_image == execution_container_image
            ):
                return None

        now = iso_now()
        request = FlowRunRequest(
            id=f"flow-run-request-{uuid.uuid4().hex[:12]}",
            created_at=now,
            updated_at=now,
            flow_name=flow_name,
            summary=summary,
            project_path=state.project_path,
            conversation_id=state.conversation_id,
            source_turn_id=parent_turn.id,
            goal=goal,
            launch_context=launch_context,
            model=model,
            llm_provider=llm_provider,
            llm_profile=llm_profile,
            reasoning_effort=reasoning_effort,
            execution_container_image=execution_container_image,
        )
        request_segment = ConversationSegment(
            id=f"segment-artifact-{request.id}",
            turn_id=parent_turn.id,
            order=self._repository.next_turn_segment_order(state, parent_turn.id),
            kind="flow_run_request",
            role="system",
            status="complete",
            timestamp=now,
            updated_at=now,
            artifact_id=request.id,
            source=ConversationSegmentSource(),
        )
        request.source_segment_id = request_segment.id
        state.flow_run_requests.append(request)
        self._repository.upsert_segment(state, request_segment)
        self._repository.append_event(state, f"Created flow run request {request.id} for {flow_name}.")
        return request, request_segment

    def create_flow_run_request(
        self,
        conversation_id: str,
        project_path: str,
        flow_run_request_payload: dict[str, object],
    ) -> dict[str, object]:
        normalized_project_path = normalize_project_path_value(project_path)
        with self._repository._lock:
            state = self._repository.read_state(conversation_id, normalized_project_path)
            if state is None or state.project_path != normalized_project_path:
                raise FileNotFoundError(conversation_id)
            parent_turn = next(
                (
                    turn
                    for turn in reversed(state.turns)
                    if turn.role == "assistant" and turn.kind == "message"
                ),
                None,
            )
            if parent_turn is None:
                raise ValueError("Conversation has no assistant turn that can own a flow run request.")
            persisted = self.persist_flow_run_request(state, parent_turn, flow_run_request_payload)
            if persisted is None:
                raise ValueError("Flow run request was not created because an identical request already exists on the latest assistant turn.")
            request, request_segment = persisted
            self._repository.touch_conversation_state(state)
            self._repository.write_state(state)
            return {
                "conversation_id": conversation_id,
                "project_path": normalized_project_path,
                "turn_id": parent_turn.id,
                "flow_run_request_id": request.id,
                "segment_id": request_segment.id,
            }

    def persist_flow_launch(
        self,
        state: ConversationState,
        parent_turn: ConversationTurn,
        flow_launch_payload: dict[str, object],
        *,
        create_segment: bool = True,
    ) -> tuple[FlowLaunch, Optional[ConversationSegment]]:
        flow_name = str(flow_launch_payload.get("flow_name", "")).strip()
        summary = str(flow_launch_payload.get("summary", "")).strip()
        if not flow_name or not summary:
            raise ValueError("Flow launch requires a non-empty flow_name and summary.")
        goal = str(flow_launch_payload.get("goal", "")).strip() or None
        launch_context = copy.deepcopy(flow_launch_payload.get("launch_context")) if isinstance(flow_launch_payload.get("launch_context"), dict) else None
        model = str(flow_launch_payload.get("model", "")).strip() or None
        llm_provider = str(flow_launch_payload.get("llm_provider", "")).strip().lower() or None
        llm_profile = str(flow_launch_payload.get("llm_profile", "")).strip() or None
        reasoning_effort = str(flow_launch_payload.get("reasoning_effort", "")).strip().lower() or None
        execution_container_image = str(flow_launch_payload.get("execution_container_image", "")).strip() or None

        now = iso_now()
        launch = FlowLaunch(
            id=f"flow-launch-{uuid.uuid4().hex[:12]}",
            created_at=now,
            updated_at=now,
            flow_name=flow_name,
            summary=summary,
            project_path=state.project_path,
            conversation_id=state.conversation_id,
            source_turn_id=parent_turn.id,
            goal=goal,
            launch_context=launch_context,
            model=model,
            llm_provider=llm_provider,
            llm_profile=llm_profile,
            reasoning_effort=reasoning_effort,
            execution_container_image=execution_container_image,
        )
        state.flow_launches.append(launch)
        launch_segment: ConversationSegment | None = None
        if create_segment:
            launch_segment = ConversationSegment(
                id=f"segment-artifact-{launch.id}",
                turn_id=parent_turn.id,
                order=self._repository.next_turn_segment_order(state, parent_turn.id),
                kind="flow_launch",
                role="system",
                status="complete",
                timestamp=now,
                updated_at=now,
                artifact_id=launch.id,
                source=ConversationSegmentSource(),
            )
            launch.source_segment_id = launch_segment.id
            self._repository.upsert_segment(state, launch_segment)
        self._repository.append_event(state, f"Created direct flow launch {launch.id} for {flow_name}.")
        return launch, launch_segment

    def persist_proposed_plan_artifact(
        self,
        state: ConversationState,
        parent_turn: ConversationTurn,
        plan_segment: ConversationSegment,
    ) -> ProposedPlanArtifact:
        if plan_segment.kind != "plan":
            raise ValueError("Only plan segments can be persisted as proposed plan artifacts.")
        content = plan_segment.content.strip()
        if not content:
            raise ValueError("Proposed plan content is required.")
        artifact = next(
            (
                entry
                for entry in state.proposed_plans
                if (
                    (plan_segment.artifact_id and entry.id == plan_segment.artifact_id)
                    or entry.source_segment_id == plan_segment.id
                )
            ),
            None,
        )
        now = iso_now()
        title = _proposed_plan_title(content)
        if artifact is None:
            artifact = ProposedPlanArtifact(
                id=f"proposed-plan-{uuid.uuid4().hex[:12]}",
                created_at=now,
                updated_at=now,
                title=title,
                content=content,
                project_path=state.project_path,
                conversation_id=state.conversation_id,
                source_turn_id=parent_turn.id,
                source_segment_id=plan_segment.id,
            )
            state.proposed_plans.append(artifact)
            self._repository.append_event(state, f"Created proposed plan artifact {artifact.id}.")
        else:
            artifact.title = title
            artifact.content = content
            artifact.updated_at = now
        plan_segment.artifact_id = artifact.id
        return artifact

    def create_flow_launch(
        self,
        conversation_id: str,
        project_path: str,
        flow_launch_payload: dict[str, object],
    ) -> dict[str, object]:
        normalized_project_path = normalize_project_path_value(project_path)
        with self._repository._lock:
            state = self._repository.read_state(conversation_id, normalized_project_path)
            if state is None or state.project_path != normalized_project_path:
                raise FileNotFoundError(conversation_id)
            parent_turn = next(
                (
                    turn
                    for turn in reversed(state.turns)
                    if turn.role == "assistant" and turn.kind == "message"
                ),
                None,
            )
            if parent_turn is None:
                raise ValueError("Conversation has no assistant turn that can own a direct flow launch.")
            launch, launch_segment = self.persist_flow_launch(state, parent_turn, flow_launch_payload)
            if launch_segment is None:
                raise RuntimeError("Direct flow launch artifact did not create a segment.")
            self._repository.touch_conversation_state(state)
            self._repository.write_state(state)
            return {
                "conversation_id": conversation_id,
                "project_path": normalized_project_path,
                "turn_id": parent_turn.id,
                "flow_launch_id": launch.id,
                "segment_id": launch_segment.id,
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
    ) -> tuple[dict[str, object], FlowRunRequest]:
        normalized_project_path = normalize_project_path_value(project_path)
        trimmed_message = message.strip()
        if not trimmed_message:
            raise ValueError("Review message is required.")
        with self._repository._lock:
            state = self._repository.read_state(conversation_id, normalized_project_path)
            if state is None or state.project_path != normalized_project_path:
                raise ValueError("Conversation not found for project.")
            request = next((entry for entry in state.flow_run_requests if entry.id == request_id), None)
            if request is None:
                raise ValueError("Unknown flow run request.")
            if request.status not in {"pending", "approved", "launch_failed"}:
                raise ValueError(f"Flow run request is not reviewable in status '{request.status}'.")
            now = iso_now()
            if disposition == "approved":
                request.status = "approved"
                request.review_message = trimmed_message
                request.updated_at = now
                if flow_name:
                    request.flow_name = flow_name
                if model:
                    request.model = model
                if llm_provider is not None:
                    request.llm_provider = llm_provider.strip().lower() or None
                if llm_profile is not None:
                    request.llm_profile = llm_profile.strip() or None
                if reasoning_effort is not None:
                    request.reasoning_effort = reasoning_effort.strip().lower() or None
                if execution_container_image is not None:
                    request.execution_container_image = execution_container_image.strip() or None
                self._repository.append_event(state, f"Approved flow run request {request.id}.")
            else:
                request.status = "rejected"
                request.review_message = trimmed_message
                request.updated_at = now
                self._repository.append_event(state, f"Rejected flow run request {request.id}.")
            self._repository.touch_conversation_state(state, title_hint=trimmed_message)
            self._repository.write_state(state)
            return self._repository.serialize_conversation_state_for_ui(state), request

    def review_proposed_plan(
        self,
        conversation_id: str,
        project_path: str,
        plan_id: str,
        disposition: str,
        review_note: Optional[str],
    ) -> tuple[dict[str, object], ProposedPlanArtifact, Optional[FlowLaunch]]:
        normalized_project_path = normalize_project_path_value(project_path)
        if disposition not in {"approved", "rejected"}:
            raise ValueError("Proposed plan disposition must be approved or rejected.")
        normalized_review_note = review_note.strip() if isinstance(review_note, str) else ""
        with self._repository._lock:
            state = self._repository.read_state(conversation_id, normalized_project_path)
            if state is None or state.project_path != normalized_project_path:
                raise ValueError("Conversation not found for project.")
            proposed_plan = next((entry for entry in state.proposed_plans if entry.id == plan_id), None)
            if proposed_plan is None:
                raise ValueError("Unknown proposed plan artifact.")
            if proposed_plan.status != "pending_review":
                raise ValueError(f"Proposed plan is not reviewable in status '{proposed_plan.status}'.")

            now = iso_now()
            proposed_plan.review_note = normalized_review_note or None
            proposed_plan.updated_at = now

            if disposition == "rejected":
                proposed_plan.status = "rejected"
                self._repository.append_event(state, f"Rejected proposed plan {proposed_plan.id}.")
                self._repository.touch_conversation_state(state)
                self._repository.write_state(state)
                return self._repository.serialize_conversation_state_for_ui(state), proposed_plan, None

            source_turn = next((turn for turn in state.turns if turn.id == proposed_plan.source_turn_id), None)
            if source_turn is None:
                raise ValueError("Proposed plan is missing its source turn.")

            project_root = Path(normalized_project_path)
            if not project_root.exists() or not project_root.is_dir():
                raise ValueError("Project path is not available for writing the approved plan.")

            change_request_id, change_request_dir = _allocate_change_request_dir(
                project_root,
                proposed_plan.title,
                created_at=now,
            )
            request_path = change_request_dir / "request.md"
            request_path.write_text(proposed_plan.content.rstrip() + "\n", encoding="utf-8")
            relative_request_path = request_path.relative_to(project_root).as_posix()

            launch, _ = self.persist_flow_launch(
                state,
                source_turn,
                {
                    "flow_name": IMPLEMENT_CHANGE_REQUEST_FLOW,
                    "summary": f"Implement approved change request: {proposed_plan.title}",
                    "goal": f"Implement the approved change request written to {relative_request_path}.",
                    "launch_context": {
                        "context.request.change_request_id": change_request_id,
                        "context.request.change_request_path": relative_request_path,
                    },
                },
            )

            proposed_plan.status = "approved"
            proposed_plan.written_change_request_path = str(request_path.resolve(strict=False))
            proposed_plan.flow_launch_id = launch.id
            proposed_plan.run_id = None
            proposed_plan.launch_error = None
            proposed_plan.updated_at = iso_now()
            self._repository.append_event(
                state,
                f"Approved proposed plan {proposed_plan.id} and wrote {relative_request_path}.",
            )
            self._repository.touch_conversation_state(state)
            self._repository.write_state(state)
            return self._repository.serialize_conversation_state_for_ui(state), proposed_plan, launch

    def note_proposed_plan_launch_started(
        self,
        conversation_id: str,
        plan_id: str,
        run_id: str,
        flow_name: str,
    ) -> dict[str, object]:
        with self._repository._lock:
            state = self._repository.read_state(conversation_id)
            if state is None:
                raise ValueError("Conversation not found.")
            proposed_plan = next((entry for entry in state.proposed_plans if entry.id == plan_id), None)
            if proposed_plan is None:
                raise ValueError("Unknown proposed plan artifact.")
            proposed_plan.status = "approved"
            proposed_plan.run_id = run_id
            proposed_plan.launch_error = None
            proposed_plan.updated_at = iso_now()
            if proposed_plan.flow_launch_id is not None:
                launch = next((entry for entry in state.flow_launches if entry.id == proposed_plan.flow_launch_id), None)
                if launch is not None:
                    launch.status = "launched"
                    launch.updated_at = proposed_plan.updated_at
                    launch.run_id = run_id
                    launch.flow_name = flow_name
                    launch.launch_error = None
            self._repository.append_event(
                state,
                f"Launched proposed plan {proposed_plan.id} as run {run_id} using {flow_name}.",
            )
            self._repository.touch_conversation_state(state)
            self._repository.write_state(state)
            return self._repository.serialize_conversation_state_for_ui(state)

    def fail_proposed_plan_launch(
        self,
        conversation_id: str,
        plan_id: str,
        flow_name: str,
        error: str,
    ) -> dict[str, object]:
        launch_error = error.strip() or "Flow launch failed."
        with self._repository._lock:
            state = self._repository.read_state(conversation_id)
            if state is None:
                raise ValueError("Conversation not found.")
            proposed_plan = next((entry for entry in state.proposed_plans if entry.id == plan_id), None)
            if proposed_plan is None:
                raise ValueError("Unknown proposed plan artifact.")
            proposed_plan.status = "launch_failed"
            proposed_plan.launch_error = launch_error
            proposed_plan.updated_at = iso_now()
            if proposed_plan.flow_launch_id is not None:
                launch = next((entry for entry in state.flow_launches if entry.id == proposed_plan.flow_launch_id), None)
                if launch is not None:
                    launch.status = "launch_failed"
                    launch.updated_at = proposed_plan.updated_at
                    launch.flow_name = flow_name
                    launch.launch_error = launch_error
            self._repository.append_event(
                state,
                f"Approved proposed plan {proposed_plan.id} failed to launch {flow_name}: {launch_error}",
            )
            self._repository.touch_conversation_state(state)
            self._repository.write_state(state)
            return self._repository.serialize_conversation_state_for_ui(state)

    def note_flow_launch_started(
        self,
        conversation_id: str,
        launch_id: str,
        run_id: str,
        flow_name: str,
    ) -> dict[str, object]:
        with self._repository._lock:
            state = self._repository.read_state(conversation_id)
            if state is None:
                raise ValueError("Conversation not found.")
            launch = next((entry for entry in state.flow_launches if entry.id == launch_id), None)
            if launch is None:
                raise ValueError("Unknown flow launch.")
            launch.status = "launched"
            launch.updated_at = iso_now()
            launch.run_id = run_id
            launch.flow_name = flow_name
            launch.launch_error = None
            self._repository.append_event(state, f"Launched direct flow {launch.id} as run {run_id} using {flow_name}.")
            self._repository.touch_conversation_state(state)
            self._repository.write_state(state)
            return self._repository.serialize_conversation_state_for_ui(state)

    def fail_flow_launch(
        self,
        conversation_id: str,
        launch_id: str,
        flow_name: str,
        error: str,
    ) -> dict[str, object]:
        with self._repository._lock:
            state = self._repository.read_state(conversation_id)
            if state is None:
                raise ValueError("Conversation not found.")
            launch = next((entry for entry in state.flow_launches if entry.id == launch_id), None)
            if launch is None:
                raise ValueError("Unknown flow launch.")
            launch.status = "launch_failed"
            launch.updated_at = iso_now()
            launch.flow_name = flow_name
            launch.launch_error = error.strip() or "Flow launch failed."
            self._repository.append_event(state, f"Direct flow launch {launch.id} failed for {flow_name}: {launch.launch_error}")
            self._repository.touch_conversation_state(state)
            self._repository.write_state(state)
            return self._repository.serialize_conversation_state_for_ui(state)

    def note_flow_run_request_launched(
        self,
        conversation_id: str,
        request_id: str,
        run_id: str,
        flow_name: str,
    ) -> dict[str, object]:
        with self._repository._lock:
            state = self._repository.read_state(conversation_id)
            if state is None:
                raise ValueError("Unknown conversation.")
            request = next((entry for entry in state.flow_run_requests if entry.id == request_id), None)
            if request is None:
                raise ValueError("Unknown flow run request.")
            request.status = "launched"
            request.run_id = run_id
            request.flow_name = flow_name
            request.launch_error = None
            request.updated_at = iso_now()
            self._repository.append_event(
                state,
                f"Launched flow run request {request.id} as run {run_id} using {flow_name}.",
            )
            self._repository.touch_conversation_state(state)
            self._repository.write_state(state)
            return self._repository.serialize_conversation_state_for_ui(state)

    def fail_flow_run_request_launch(
        self,
        conversation_id: str,
        request_id: str,
        flow_name: str,
        error: str,
    ) -> dict[str, object]:
        with self._repository._lock:
            state = self._repository.read_state(conversation_id)
            if state is None:
                raise ValueError("Unknown conversation.")
            request = next((entry for entry in state.flow_run_requests if entry.id == request_id), None)
            if request is None:
                raise ValueError("Unknown flow run request.")
            request.status = "launch_failed"
            request.flow_name = flow_name
            request.launch_error = error
            request.updated_at = iso_now()
            self._repository.append_event(
                state,
                f"Flow run request {request.id} failed to launch: {error}",
            )
            self._repository.touch_conversation_state(state)
            self._repository.write_state(state)
            return self._repository.serialize_conversation_state_for_ui(state)
