from __future__ import annotations

import asyncio
import json
import uuid
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Callable, Optional, Protocol

from fastapi import APIRouter, HTTPException, Request
from fastapi.responses import PlainTextResponse, StreamingResponse
from pydantic import BaseModel, ConfigDict

from attractor.api.flow_sources import resolve_flow_path
from spark.chat.response_parsing import normalize_flow_run_request_payload
from spark.chat.service import ProjectChatService, TurnInProgressError
from spark.workspace.attractor_client import AttractorApiClient, AttractorApiError
from spark.workspace.conversations.artifacts import IMPLEMENT_CHANGE_REQUEST_FLOW
from spark.workspace.flow_catalog import (
    ALLOWED_LAUNCH_POLICIES,
    LAUNCH_POLICY_AGENT_REQUESTABLE,
    FlowDescription,
    FlowSummary,
    list_flow_summaries,
    normalize_launch_policy,
    read_flow_description,
    read_flow_launch_policy,
    read_flow_raw,
    set_flow_launch_policy,
)
from spark.workspace.storage import (
    delete_project_record,
    list_project_records,
    read_project_record,
    update_project_record,
)
from spark.workspace.triggers import (
    TriggerError,
    TriggerRuntime,
    create_trigger_definition,
    delete_trigger_definition,
    delete_trigger_state,
    list_trigger_definitions,
    load_trigger_state,
    read_trigger_definition,
    serialize_trigger,
    update_trigger_definition,
)
from spark_common.project_identity import normalize_project_path
from spark_common.runtime_path import resolve_runtime_workspace_path


class ConversationTurnRequest(BaseModel):
    project_path: str
    message: str
    provider: Optional[str] = None
    llm_profile: Optional[str] = None
    model: Optional[str] = None
    reasoning_effort: Optional[str] = None
    chat_mode: Optional[str] = None


class ConversationRequestUserInputAnswerRequest(BaseModel):
    model_config = ConfigDict(extra="forbid")
    project_path: str
    answers: dict[str, str]


class ConversationSettingsRequest(BaseModel):
    project_path: str
    chat_mode: Optional[str] = None
    provider: Optional[str] = None
    llm_profile: Optional[str] = None
    model: Optional[str] = None
    reasoning_effort: Optional[str] = None


class ProjectRegistrationRequest(BaseModel):
    project_path: str
    execution_container_image: Optional[str] = None


class ProjectStateUpdateRequest(BaseModel):
    project_path: str
    is_favorite: Optional[bool] = None
    last_accessed_at: Optional[str] = None
    active_conversation_id: Optional[str] = None
    execution_container_image: Optional[str] = None

class FlowRunRequestCreateByHandleRequest(BaseModel):
    model_config = ConfigDict(extra="forbid")
    flow_name: str
    summary: str
    goal: Optional[str] = None
    launch_context: Optional[dict[str, Any]] = None
    model: Optional[str] = None
    llm_provider: Optional[str] = None
    llm_profile: Optional[str] = None
    reasoning_effort: Optional[str] = None
    execution_container_image: Optional[str] = None


class FlowRunRequestReviewRequest(BaseModel):
    project_path: str
    disposition: str
    message: str
    flow_name: Optional[str] = None
    model: Optional[str] = None
    llm_provider: Optional[str] = None
    llm_profile: Optional[str] = None
    reasoning_effort: Optional[str] = None
    execution_container_image: Optional[str] = None


class ProposedPlanReviewRequest(BaseModel):
    project_path: str
    disposition: str
    review_note: Optional[str] = None


class RunLaunchRequest(BaseModel):
    model_config = ConfigDict(extra="ignore")
    flow_name: str
    summary: str
    conversation_handle: Optional[str] = None
    project_path: Optional[str] = None
    goal: Optional[str] = None
    launch_context: Optional[dict[str, Any]] = None
    model: Optional[str] = None
    llm_provider: Optional[str] = None
    llm_profile: Optional[str] = None
    reasoning_effort: Optional[str] = None
    execution_container_image: Optional[str] = None


class FlowLaunchPolicyUpdateRequest(BaseModel):
    launch_policy: str

class TriggerCreateRequest(BaseModel):
    model_config = ConfigDict(extra="forbid")
    name: str
    enabled: bool = True
    source_type: str
    action: dict[str, Any]
    source: dict[str, Any]


class TriggerUpdateRequest(BaseModel):
    model_config = ConfigDict(extra="forbid")
    name: Optional[str] = None
    enabled: Optional[bool] = None
    action: Optional[dict[str, Any]] = None
    source: Optional[dict[str, Any]] = None
    regenerate_webhook_secret: bool = False


class WorkspaceSettings(Protocol):
    data_dir: Path
    config_dir: Path
    flows_dir: Path
    project_roots: tuple[Path, ...]


@dataclass(frozen=True)
class WorkspaceApiDependencies:
    get_settings: Callable[[], WorkspaceSettings]
    get_project_chat: Callable[[], ProjectChatService]
    get_attractor_client: Callable[[], AttractorApiClient]
    resolve_project_git_branch: Callable[[Path], Optional[str]]
    resolve_project_git_commit: Callable[[Path], Optional[str]]
    get_trigger_runtime: Callable[[], TriggerRuntime]


def create_workspace_router(deps: WorkspaceApiDependencies) -> APIRouter:
    router = APIRouter()
    terminal_pipeline_statuses = {
        "completed",
        "failed",
        "validation_error",
        "canceled",
        "cancelled",
    }

    def _serialize_project_record(project: Any) -> dict[str, object]:
        return {
            "project_id": project.project_id,
            "project_path": project.project_path,
            "display_name": project.display_name,
            "created_at": project.created_at,
            "last_opened_at": project.last_opened_at,
            "last_accessed_at": project.last_accessed_at,
            "is_favorite": project.is_favorite,
            "active_conversation_id": project.active_conversation_id,
            "execution_container_image": project.execution_container_image,
        }

    def _serialize_deleted_project_record(project: Any) -> dict[str, object]:
        return {
            "status": "deleted",
            "project_id": project.project_id,
            "project_path": project.project_path,
            "display_name": project.display_name,
        }

    def _normalize_project_path_or_400(project_path: str) -> str:
        normalized_project_path = normalize_project_path(project_path)
        if not normalized_project_path:
            raise HTTPException(status_code=400, detail="Project path is required.")
        return normalized_project_path

    def _normalize_browse_path_or_400(requested_path: str | None) -> Path:
        if requested_path is None:
            project_roots = deps.get_settings().project_roots
            if project_roots:
                return project_roots[0]
            return Path(normalize_project_path(str(Path.home())))

        trimmed_path = requested_path.strip()
        if not trimmed_path:
            raise HTTPException(status_code=400, detail="Browse path is required.")

        raw_path = Path(trimmed_path).expanduser()
        if not raw_path.is_absolute():
            raise HTTPException(status_code=400, detail="Browse path must be absolute.")

        normalized_path = Path(normalize_project_path(str(raw_path)))
        if not normalized_path.is_absolute():
            raise HTTPException(status_code=400, detail="Browse path must be absolute.")
        return normalized_path

    def _validate_flow_surface(surface: Optional[str]) -> str:
        normalized_surface = str(surface or "human").strip().lower()
        if normalized_surface not in {"human", "agent"}:
            raise HTTPException(status_code=400, detail="Flow surface must be 'human' or 'agent'.")
        return normalized_surface

    async def _ensure_flow_exists(flow_name: str) -> None:
        flow_path = resolve_flow_path(deps.get_settings().flows_dir, flow_name)
        if not flow_path.exists():
            raise HTTPException(status_code=404, detail=f"Unknown flow: {flow_name}")

    def _serialize_flow_summary(flow: FlowSummary) -> dict[str, object]:
        return {
            "name": flow.name,
            "title": flow.title,
            "description": flow.description,
            "launch_policy": flow.launch_policy,
            "effective_launch_policy": flow.effective_launch_policy,
            "graph_label": flow.graph_label,
            "graph_goal": flow.graph_goal,
        }

    def _serialize_flow_description(flow: FlowDescription) -> dict[str, object]:
        return {
            **_serialize_flow_summary(flow),
            "node_count": flow.node_count,
            "edge_count": flow.edge_count,
            "features": {
                "has_human_gate": flow.features.has_human_gate,
                "has_manager_loop": flow.features.has_manager_loop,
            },
        }

    def _filter_flow_surface_or_404(flow: FlowSummary | FlowDescription, surface: str) -> None:
        if surface == "agent" and flow.effective_launch_policy != LAUNCH_POLICY_AGENT_REQUESTABLE:
            raise HTTPException(status_code=404, detail=f"Unknown flow: {flow.name}")

    async def _wait_for_pipeline_terminal_status(run_id: str) -> dict[str, Any]:
        client = deps.get_attractor_client()
        while True:
            payload = await client.get_pipeline(run_id)
            status = str(payload.get("status", "")).strip().lower()
            if status in terminal_pipeline_statuses:
                return payload
            await asyncio.sleep(1.0)

    async def _launch_flow_run_request_pipeline(
        *,
        conversation_id: str,
        request_id: str,
        project_path: str,
        flow_name: str,
        goal: Optional[str],
        launch_context: Optional[dict[str, Any]],
        model: Optional[str],
        llm_provider: Optional[str],
        llm_profile: Optional[str],
        reasoning_effort: Optional[str],
        execution_container_image: Optional[str],
    ) -> str | None:
        project_chat = deps.get_project_chat()
        launch_kwargs: dict[str, Any] = {}
        if llm_provider is not None:
            launch_kwargs["llm_provider"] = llm_provider
        if llm_profile is not None:
            launch_kwargs["llm_profile"] = llm_profile
        if reasoning_effort is not None:
            launch_kwargs["reasoning_effort"] = reasoning_effort
        effective_execution_container_image = execution_container_image
        if effective_execution_container_image is None:
            record = await asyncio.to_thread(read_project_record, deps.get_settings().data_dir, project_path)
            effective_execution_container_image = record.execution_container_image if record is not None else None
        if effective_execution_container_image is not None:
            launch_kwargs["execution_container_image"] = effective_execution_container_image
        try:
            launch_payload = await deps.get_attractor_client().start_pipeline(
                run_id=None,
                flow_name=flow_name,
                working_directory=project_path,
                model=model,
                goal=goal,
                launch_context=launch_context,
                **launch_kwargs,
            )
        except AttractorApiError as exc:
            await asyncio.to_thread(
                project_chat.fail_flow_run_request_launch,
                conversation_id,
                request_id,
                flow_name,
                str(exc),
            )
            await project_chat.publish_snapshot(conversation_id)
            return None

        if launch_payload.get("status") != "started":
            error = str(launch_payload.get("error") or "Flow run could not be started.")
            await asyncio.to_thread(
                project_chat.fail_flow_run_request_launch,
                conversation_id,
                request_id,
                flow_name,
                error,
            )
            await project_chat.publish_snapshot(conversation_id)
            return None

        run_id = str(launch_payload.get("run_id") or "")
        if not run_id:
            await asyncio.to_thread(
                project_chat.fail_flow_run_request_launch,
                conversation_id,
                request_id,
                flow_name,
                "Flow run did not return a run id.",
            )
            await project_chat.publish_snapshot(conversation_id)
            return None

        await asyncio.to_thread(
            project_chat.note_flow_run_request_launched,
            conversation_id,
            request_id,
            run_id,
            flow_name,
        )
        await deps.get_trigger_runtime().observe_run(
            run_id=run_id,
            flow_name=flow_name,
            project_path=project_path,
        )
        await project_chat.publish_snapshot(conversation_id)
        return run_id

    async def _launch_direct_flow(
        *,
        flow_name: str,
        project_path: str,
        goal: Optional[str],
        launch_context: Optional[dict[str, Any]],
        model: Optional[str],
        llm_provider: Optional[str],
        llm_profile: Optional[str],
        reasoning_effort: Optional[str],
        execution_container_image: Optional[str] = None,
    ) -> str:
        launch_kwargs: dict[str, Any] = {}
        if llm_provider is not None:
            launch_kwargs["llm_provider"] = llm_provider
        if llm_profile is not None:
            launch_kwargs["llm_profile"] = llm_profile
        if reasoning_effort is not None:
            launch_kwargs["reasoning_effort"] = reasoning_effort
        effective_execution_container_image = execution_container_image
        if effective_execution_container_image is None:
            record = await asyncio.to_thread(read_project_record, deps.get_settings().data_dir, project_path)
            effective_execution_container_image = record.execution_container_image if record is not None else None
        if effective_execution_container_image is not None:
            launch_kwargs["execution_container_image"] = effective_execution_container_image
        try:
            launch_payload = await deps.get_attractor_client().start_pipeline(
                run_id=None,
                flow_name=flow_name,
                working_directory=project_path,
                model=model,
                goal=goal,
                launch_context=launch_context,
                **launch_kwargs,
            )
        except AttractorApiError as exc:
            raise HTTPException(status_code=exc.status_code, detail=str(exc)) from exc

        if launch_payload.get("status") != "started":
            error = str(launch_payload.get("error") or "Flow launch could not be started.")
            raise HTTPException(status_code=500, detail=error)

        run_id = str(launch_payload.get("run_id") or "")
        if not run_id:
            raise HTTPException(status_code=500, detail="Flow launch did not return a run id.")

        await deps.get_trigger_runtime().observe_run(
            run_id=run_id,
            flow_name=flow_name,
            project_path=project_path,
        )
        return run_id

    @router.get("/api/conversations/{conversation_id}")
    async def get_project_conversation(conversation_id: str, project_path: Optional[str] = None):
        try:
            return await asyncio.to_thread(deps.get_project_chat().get_snapshot, conversation_id, project_path)
        except FileNotFoundError as exc:
            raise HTTPException(status_code=404, detail=f"Unknown conversation: {conversation_id}") from exc
        except ValueError as exc:
            raise HTTPException(status_code=400, detail=str(exc)) from exc

    @router.get("/api/conversations/{conversation_id}/segments/{segment_id}/tool-output")
    async def get_project_conversation_segment_tool_output(
        conversation_id: str,
        segment_id: str,
        project_path: Optional[str] = None,
    ):
        try:
            return await asyncio.to_thread(
                deps.get_project_chat().get_segment_tool_output,
                conversation_id,
                segment_id,
                project_path,
            )
        except FileNotFoundError as exc:
            raise HTTPException(status_code=404, detail="Unknown conversation segment tool output.") from exc
        except ValueError as exc:
            raise HTTPException(status_code=400, detail=str(exc)) from exc

    @router.put("/api/conversations/{conversation_id}/settings")
    async def update_project_conversation_settings(conversation_id: str, req: ConversationSettingsRequest):
        try:
            return await asyncio.to_thread(
                deps.get_project_chat().update_conversation_settings,
                conversation_id,
                req.project_path,
                chat_mode=req.chat_mode,
                provider=req.provider,
                llm_profile=req.llm_profile,
                model=req.model,
                reasoning_effort=req.reasoning_effort,
            )
        except ValueError as exc:
            raise HTTPException(status_code=400, detail=str(exc)) from exc

    @router.delete("/api/conversations/{conversation_id}")
    async def delete_project_conversation(conversation_id: str, project_path: str):
        try:
            return await asyncio.to_thread(deps.get_project_chat().delete_conversation, conversation_id, project_path)
        except FileNotFoundError as exc:
            raise HTTPException(status_code=404, detail=f"Unknown conversation: {conversation_id}") from exc
        except ValueError as exc:
            raise HTTPException(status_code=400, detail=str(exc)) from exc

    @router.get("/api/projects")
    async def list_projects():
        projects = await asyncio.to_thread(list_project_records, deps.get_settings().data_dir)
        return [_serialize_project_record(project) for project in projects]

    @router.get("/api/flows")
    async def list_workspace_flows(surface: Optional[str] = None):
        normalized_surface = _validate_flow_surface(surface)
        flows = await asyncio.to_thread(
            list_flow_summaries,
            deps.get_settings().flows_dir,
            deps.get_settings().config_dir,
        )
        if normalized_surface == "agent":
            flows = [flow for flow in flows if flow.effective_launch_policy == LAUNCH_POLICY_AGENT_REQUESTABLE]
        return [_serialize_flow_summary(flow) for flow in flows]

    @router.get("/api/flows/{flow_name:path}/raw")
    async def get_workspace_flow_raw(flow_name: str, surface: Optional[str] = None):
        normalized_surface = _validate_flow_surface(surface)
        try:
            policy_state = await asyncio.to_thread(read_flow_launch_policy, deps.get_settings().config_dir, flow_name)
        except ValueError as exc:
            raise HTTPException(status_code=400, detail=str(exc)) from exc
        _filter_flow_surface_or_404(policy_state, normalized_surface)
        try:
            resolved_flow_name, raw_content = await asyncio.to_thread(
                read_flow_raw,
                deps.get_settings().flows_dir,
                flow_name,
            )
        except FileNotFoundError as exc:
            raise HTTPException(status_code=404, detail=f"Unknown flow: {flow_name}") from exc
        return PlainTextResponse(raw_content, media_type="text/vnd.graphviz", headers={"X-Spark-Flow-Name": resolved_flow_name})

    @router.get("/api/flows/{flow_name:path}/validate")
    async def validate_workspace_flow(flow_name: str):
        try:
            resolved_flow_name, raw_content = await asyncio.to_thread(
                read_flow_raw,
                deps.get_settings().flows_dir,
                flow_name,
            )
        except FileNotFoundError as exc:
            raise HTTPException(status_code=404, detail=f"Unknown flow: {flow_name}") from exc
        try:
            preview_payload = await deps.get_attractor_client().preview_flow(raw_content)
        except AttractorApiError as exc:
            raise HTTPException(status_code=exc.status_code, detail=str(exc)) from exc
        flow_path = resolve_flow_path(deps.get_settings().flows_dir, resolved_flow_name)
        return {
            "name": resolved_flow_name,
            "path": str(flow_path.resolve(strict=False)),
            **preview_payload,
        }

    @router.put("/api/flows/{flow_name:path}/launch-policy")
    async def put_workspace_flow_launch_policy(flow_name: str, req: FlowLaunchPolicyUpdateRequest):
        try:
            normalized_launch_policy = normalize_launch_policy(req.launch_policy)
        except ValueError as exc:
            raise HTTPException(status_code=400, detail=str(exc)) from exc
        await _ensure_flow_exists(flow_name)
        policy_state = await asyncio.to_thread(
            set_flow_launch_policy,
            deps.get_settings().config_dir,
            flow_name,
            normalized_launch_policy,
        )
        return {
            "name": policy_state.name,
            "launch_policy": policy_state.launch_policy,
            "effective_launch_policy": policy_state.effective_launch_policy,
            "allowed_launch_policies": sorted(ALLOWED_LAUNCH_POLICIES),
        }

    @router.get("/api/flows/{flow_name:path}")
    async def get_workspace_flow(flow_name: str, surface: Optional[str] = None):
        normalized_surface = _validate_flow_surface(surface)
        try:
            flow = await asyncio.to_thread(
                read_flow_description,
                deps.get_settings().flows_dir,
                deps.get_settings().config_dir,
                flow_name,
            )
        except FileNotFoundError as exc:
            raise HTTPException(status_code=404, detail=f"Unknown flow: {flow_name}") from exc
        except RuntimeError as exc:
            raise HTTPException(status_code=500, detail=str(exc)) from exc
        _filter_flow_surface_or_404(flow, normalized_surface)
        return _serialize_flow_description(flow)

    @router.post("/api/projects/register")
    async def register_project(req: ProjectRegistrationRequest):
        normalized_project_path = _normalize_project_path_or_400(req.project_path)
        try:
            project = await asyncio.to_thread(read_project_record, deps.get_settings().data_dir, normalized_project_path)
            if project is None:
                raise ValueError("Unable to register project.")
            if req.execution_container_image is not None:
                project = await asyncio.to_thread(
                    update_project_record,
                    deps.get_settings().data_dir,
                    normalized_project_path,
                    execution_container_image=req.execution_container_image,
                )
            return _serialize_project_record(project)
        except ValueError as exc:
            raise HTTPException(status_code=400, detail=str(exc)) from exc

    @router.patch("/api/projects/state")
    async def update_project_state(req: ProjectStateUpdateRequest):
        normalized_project_path = _normalize_project_path_or_400(req.project_path)
        try:
            project = await asyncio.to_thread(
                update_project_record,
                deps.get_settings().data_dir,
                normalized_project_path,
                last_accessed_at=req.last_accessed_at,
                is_favorite=req.is_favorite,
                active_conversation_id=req.active_conversation_id,
                execution_container_image=req.execution_container_image,
            )
            return _serialize_project_record(project)
        except ValueError as exc:
            raise HTTPException(status_code=400, detail=str(exc)) from exc

    @router.delete("/api/projects")
    async def delete_project(project_path: str):
        normalized_project_path = _normalize_project_path_or_400(project_path)
        try:
            deleted = await asyncio.to_thread(
                delete_project_record,
                deps.get_settings().data_dir,
                normalized_project_path,
            )
            return _serialize_deleted_project_record(deleted)
        except ValueError as exc:
            raise HTTPException(status_code=404, detail=str(exc)) from exc

    @router.get("/api/triggers")
    async def list_triggers():
        return await deps.get_trigger_runtime().list_triggers()

    @router.post("/api/triggers")
    async def create_trigger(req: TriggerCreateRequest):
        flow_name = req.action.get("flow_name")
        if not isinstance(flow_name, str) or not flow_name.strip():
            raise HTTPException(status_code=400, detail="Trigger action requires a flow_name.")
        await _ensure_flow_exists(flow_name.strip())
        try:
            definition, webhook_secret = await asyncio.to_thread(
                create_trigger_definition,
                deps.get_settings().config_dir,
                name=req.name,
                enabled=req.enabled,
                source_type=req.source_type,
                action=req.action,
                source=req.source,
                protected=False,
            )
        except TriggerError as exc:
            raise HTTPException(status_code=400, detail=str(exc)) from exc
        await deps.get_trigger_runtime().reload()
        state = load_trigger_state(deps.get_settings().data_dir, definition.id)
        return serialize_trigger(definition, state, webhook_secret=webhook_secret)

    @router.get("/api/triggers/{trigger_id}")
    async def get_trigger(trigger_id: str):
        payload = await deps.get_trigger_runtime().get_trigger(trigger_id)
        if payload is None:
            raise HTTPException(status_code=404, detail="Unknown trigger.")
        return payload

    @router.patch("/api/triggers/{trigger_id}")
    async def patch_trigger(trigger_id: str, req: TriggerUpdateRequest):
        next_action = req.action
        if next_action is not None:
            flow_name = next_action.get("flow_name")
            if isinstance(flow_name, str) and flow_name.strip():
                await _ensure_flow_exists(flow_name.strip())
        try:
            definition, webhook_secret = await asyncio.to_thread(
                update_trigger_definition,
                deps.get_settings().config_dir,
                trigger_id,
                name=req.name,
                enabled=req.enabled,
                action=next_action,
                source=req.source,
                regenerate_webhook_secret=req.regenerate_webhook_secret,
            )
        except TriggerError as exc:
            status_code = 404 if str(exc) == "Unknown trigger." else 400
            raise HTTPException(status_code=status_code, detail=str(exc)) from exc
        await deps.get_trigger_runtime().reload()
        state = load_trigger_state(deps.get_settings().data_dir, definition.id)
        return serialize_trigger(definition, state, webhook_secret=webhook_secret)

    @router.delete("/api/triggers/{trigger_id}")
    async def remove_trigger(trigger_id: str):
        definition = read_trigger_definition(deps.get_settings().config_dir, trigger_id)
        if definition is None:
            raise HTTPException(status_code=404, detail="Unknown trigger.")
        if definition.protected:
            raise HTTPException(status_code=400, detail="Protected triggers cannot be deleted.")
        await asyncio.to_thread(delete_trigger_definition, deps.get_settings().config_dir, trigger_id)
        await asyncio.to_thread(delete_trigger_state, deps.get_settings().data_dir, trigger_id)
        await deps.get_trigger_runtime().reload()
        return {"status": "deleted", "id": trigger_id}

    @router.post("/api/webhooks")
    async def post_trigger_webhook(request: Request):
        webhook_key = request.headers.get("X-Spark-Webhook-Key", "").strip()
        webhook_secret = request.headers.get("X-Spark-Webhook-Secret", "").strip()
        request_id = request.headers.get("X-Spark-Webhook-Request-Id", "").strip() or None
        if not webhook_key or not webhook_secret:
            raise HTTPException(status_code=401, detail="Webhook key and secret headers are required.")
        try:
            payload = await request.json()
        except Exception as exc:  # noqa: BLE001
            raise HTTPException(status_code=400, detail="Webhook payload must be valid JSON.") from exc
        if not isinstance(payload, dict):
            raise HTTPException(status_code=400, detail="Webhook payload must be a JSON object.")
        try:
            return await deps.get_trigger_runtime().handle_webhook(
                webhook_key=webhook_key,
                webhook_secret=webhook_secret,
                payload=payload,
                request_id=request_id,
            )
        except TriggerError as exc:
            detail = str(exc)
            status_code = 403 if "secret" in detail.lower() else 404 if "Unknown" in detail else 400
            raise HTTPException(status_code=status_code, detail=detail) from exc

    @router.get("/api/projects/conversations")
    async def list_project_conversations(project_path: str):
        try:
            return await asyncio.to_thread(deps.get_project_chat().list_conversations, project_path)
        except ValueError as exc:
            raise HTTPException(status_code=400, detail=str(exc)) from exc

    @router.get("/api/projects/chat-models")
    async def get_project_chat_models(project_path: str):
        normalized_project_path = _normalize_project_path_or_400(project_path)
        try:
            return await asyncio.to_thread(deps.get_project_chat().list_chat_models, normalized_project_path)
        except RuntimeError as exc:
            raise HTTPException(status_code=503, detail=str(exc)) from exc
        except ValueError as exc:
            raise HTTPException(status_code=400, detail=str(exc)) from exc

    @router.get("/api/conversations/{conversation_id}/events")
    async def project_conversation_events(conversation_id: str, request: Request, project_path: Optional[str] = None):
        project_chat = deps.get_project_chat()
        try:
            await asyncio.to_thread(project_chat.get_snapshot, conversation_id, project_path)
        except FileNotFoundError as exc:
            raise HTTPException(status_code=404, detail=f"Unknown conversation: {conversation_id}") from exc
        except ValueError as exc:
            raise HTTPException(status_code=400, detail=str(exc)) from exc

        queue = project_chat.events().subscribe(conversation_id)

        async def stream():
            try:
                while True:
                    if await request.is_disconnected():
                        break
                    try:
                        event = await asyncio.wait_for(queue.get(), timeout=15.0)
                        yield f"data: {json.dumps(event)}\n\n"
                    except asyncio.TimeoutError:
                        yield ": keepalive\n\n"
            finally:
                project_chat.events().unsubscribe(conversation_id, queue)

        return StreamingResponse(
            stream(),
            media_type="text/event-stream",
            headers={
                "Cache-Control": "no-cache",
                "Connection": "keep-alive",
            },
        )

    @router.post("/api/conversations/{conversation_id}/turns")
    async def send_project_conversation_turn(conversation_id: str, req: ConversationTurnRequest):
        project_chat = deps.get_project_chat()

        def publish_progress_event(event: dict[str, Any]) -> None:
            project_chat.events().publish_nowait(conversation_id, event)

        try:
            snapshot = await asyncio.to_thread(
                project_chat.start_turn,
                conversation_id,
                req.project_path,
                req.message,
                model=req.model,
                chat_mode=req.chat_mode,
                reasoning_effort=req.reasoning_effort,
                progress_callback=publish_progress_event,
                provider=req.provider,
                llm_profile=req.llm_profile,
            )
        except ValueError as exc:
            raise HTTPException(status_code=400, detail=str(exc)) from exc
        except TurnInProgressError as exc:
            raise HTTPException(status_code=409, detail=str(exc)) from exc
        except RuntimeError as exc:
            raise HTTPException(status_code=503, detail=str(exc)) from exc
        return snapshot

    @router.post("/api/conversations/{conversation_id}/request-user-input/{request_id}/answer")
    async def answer_project_conversation_request_user_input(
        conversation_id: str,
        request_id: str,
        req: ConversationRequestUserInputAnswerRequest,
    ):
        project_chat = deps.get_project_chat()

        def publish_progress_event(event: dict[str, Any]) -> None:
            project_chat.events().publish_nowait(conversation_id, event)

        try:
            snapshot = await asyncio.to_thread(
                project_chat.submit_request_user_input_answer,
                conversation_id,
                req.project_path,
                request_id,
                req.answers,
                publish_progress_event,
            )
        except FileNotFoundError as exc:
            raise HTTPException(status_code=404, detail=f"Unknown conversation input request: {request_id}") from exc
        except ValueError as exc:
            detail = str(exc)
            status_code = 409 if ("already answered" in detail or "expired" in detail) else 400
            raise HTTPException(status_code=status_code, detail=detail) from exc
        except RuntimeError as exc:
            raise HTTPException(status_code=409, detail=str(exc)) from exc
        return snapshot

    @router.post("/api/conversations/by-handle/{conversation_handle}/flow-run-requests")
    async def create_flow_run_request_by_handle(
        conversation_handle: str,
        req: FlowRunRequestCreateByHandleRequest,
    ):
        await _ensure_flow_exists(req.flow_name)
        try:
            result = await asyncio.to_thread(
                deps.get_project_chat().create_flow_run_request_by_handle,
                conversation_handle,
                req.model_dump(),
            )
        except FileNotFoundError as exc:
            raise HTTPException(
                status_code=404,
                detail=(
                    f"Unknown conversation handle: {conversation_handle}. "
                    "Verify the handle shown in the thread UI and try again."
                ),
            ) from exc
        except ValueError as exc:
            detail = str(exc)
            status_code = 409 if "identical request already exists" in detail else 400
            raise HTTPException(status_code=status_code, detail=detail) from exc
        await deps.get_project_chat().publish_snapshot(str(result["conversation_id"]))
        return {
            "ok": True,
            **result,
        }

    @router.post("/api/runs/launch")
    async def launch_workspace_run(req: RunLaunchRequest):
        normalized_payload = normalize_flow_run_request_payload(
            req.model_dump(),
            source_name="spark run launch",
        )
        await _ensure_flow_exists(normalized_payload["flow_name"])

        conversation_handle = str(req.conversation_handle or "").strip() or None
        explicit_project_path = str(req.project_path or "").strip() or None
        conversation_id: str | None = None
        normalized_project_path: str
        artifact_result: dict[str, object] | None = None

        if conversation_handle:
            try:
                conversation_id, resolved_project_path = await asyncio.to_thread(
                    deps.get_project_chat().resolve_conversation_handle,
                    conversation_handle,
                )
            except FileNotFoundError as exc:
                raise HTTPException(
                    status_code=404,
                    detail=(
                        f"Unknown conversation handle: {conversation_handle}. "
                        "Verify the handle shown in the thread UI and try again."
                    ),
                ) from exc
            if explicit_project_path:
                normalized_explicit_project_path = _normalize_project_path_or_400(explicit_project_path)
                if normalized_explicit_project_path != resolved_project_path:
                    raise HTTPException(
                        status_code=400,
                        detail=(
                            "Explicit --project path does not match the project bound to the conversation handle."
                        ),
                    )
            normalized_project_path = resolved_project_path
            try:
                artifact_result = await asyncio.to_thread(
                    deps.get_project_chat().create_flow_launch,
                    conversation_id,
                    normalized_project_path,
                    normalized_payload,
                )
            except FileNotFoundError as exc:
                raise HTTPException(status_code=404, detail=f"Unknown conversation: {conversation_id}") from exc
            except ValueError as exc:
                raise HTTPException(status_code=400, detail=str(exc)) from exc
            await deps.get_project_chat().publish_snapshot(conversation_id)
        else:
            if not explicit_project_path:
                raise HTTPException(
                    status_code=400,
                    detail="Project path is required when conversation_handle is omitted.",
                )
            normalized_project_path = _normalize_project_path_or_400(explicit_project_path)

        try:
            run_id = await _launch_direct_flow(
                flow_name=str(normalized_payload["flow_name"]),
                project_path=normalized_project_path,
                goal=normalized_payload.get("goal") if isinstance(normalized_payload.get("goal"), str) else None,
                launch_context=normalized_payload.get("launch_context") if isinstance(normalized_payload.get("launch_context"), dict) else None,
                model=normalized_payload.get("model") if isinstance(normalized_payload.get("model"), str) else None,
                llm_provider=normalized_payload.get("llm_provider") if isinstance(normalized_payload.get("llm_provider"), str) else None,
                llm_profile=normalized_payload.get("llm_profile") if isinstance(normalized_payload.get("llm_profile"), str) else None,
                reasoning_effort=normalized_payload.get("reasoning_effort") if isinstance(normalized_payload.get("reasoning_effort"), str) else None,
                execution_container_image=(
                    normalized_payload.get("execution_container_image")
                    if isinstance(normalized_payload.get("execution_container_image"), str)
                    else None
                ),
            )
        except HTTPException as exc:
            if conversation_id and artifact_result and isinstance(artifact_result.get("flow_launch_id"), str):
                await asyncio.to_thread(
                    deps.get_project_chat().fail_flow_launch,
                    conversation_id,
                    str(artifact_result["flow_launch_id"]),
                    str(normalized_payload["flow_name"]),
                    str(exc.detail),
                )
                await deps.get_project_chat().publish_snapshot(conversation_id)
            raise

        if conversation_id and artifact_result and isinstance(artifact_result.get("flow_launch_id"), str):
            await asyncio.to_thread(
                deps.get_project_chat().note_flow_launch_started,
                conversation_id,
                str(artifact_result["flow_launch_id"]),
                run_id,
                str(normalized_payload["flow_name"]),
            )
            await deps.get_project_chat().publish_snapshot(conversation_id)

        response_payload: dict[str, object] = {
            "ok": True,
            "status": "started",
            "run_id": run_id,
            "flow_name": str(normalized_payload["flow_name"]),
            "project_path": normalized_project_path,
        }
        if conversation_handle:
            response_payload["conversation_handle"] = conversation_handle
        if conversation_id:
            response_payload["conversation_id"] = conversation_id
        if artifact_result:
            response_payload.update(artifact_result)
        return response_payload

    @router.post("/api/conversations/{conversation_id}/flow-run-requests/{request_id}/review")
    async def review_flow_run_request(
        conversation_id: str,
        request_id: str,
        req: FlowRunRequestReviewRequest,
    ):
        if req.disposition not in {"approved", "rejected"}:
            raise HTTPException(status_code=400, detail="Flow run request disposition must be approved or rejected.")
        normalized_project_path = _normalize_project_path_or_400(req.project_path)
        effective_flow_name = (req.flow_name or "").strip() or None
        if req.disposition == "approved":
            if effective_flow_name:
                await _ensure_flow_exists(effective_flow_name)
            else:
                snapshot = await asyncio.to_thread(
                    deps.get_project_chat().get_snapshot,
                    conversation_id,
                    normalized_project_path,
                )
                existing_request = next(
                    (
                        entry
                        for entry in snapshot.get("flow_run_requests", [])
                        if isinstance(entry, dict) and str(entry.get("id", "")) == request_id
                    ),
                    None,
                )
                if existing_request is None:
                    raise HTTPException(status_code=404, detail="Unknown flow run request.")
                await _ensure_flow_exists(str(existing_request.get("flow_name", "")).strip())
        try:
            snapshot, flow_run_request = await asyncio.to_thread(
                deps.get_project_chat().review_flow_run_request,
                conversation_id,
                normalized_project_path,
                request_id,
                req.disposition,
                req.message,
                effective_flow_name,
                req.model,
                req.llm_provider,
                req.llm_profile,
                req.reasoning_effort,
                req.execution_container_image,
            )
        except ValueError as exc:
            raise HTTPException(status_code=400, detail=str(exc)) from exc
        await deps.get_project_chat().publish_snapshot(conversation_id)
        if req.disposition == "approved":
            approved_flow_name = effective_flow_name or flow_run_request.flow_name
            await _ensure_flow_exists(approved_flow_name)
            await _launch_flow_run_request_pipeline(
                conversation_id=conversation_id,
                request_id=flow_run_request.id,
                project_path=normalized_project_path,
                flow_name=approved_flow_name,
                goal=flow_run_request.goal,
                launch_context=flow_run_request.launch_context,
                model=req.model or flow_run_request.model,
                llm_provider=req.llm_provider if req.llm_provider is not None else flow_run_request.llm_provider,
                llm_profile=req.llm_profile if req.llm_profile is not None else flow_run_request.llm_profile,
                reasoning_effort=(
                    req.reasoning_effort
                    if req.reasoning_effort is not None
                    else flow_run_request.reasoning_effort
                ),
                execution_container_image=(
                    req.execution_container_image
                    if req.execution_container_image is not None
                    else flow_run_request.execution_container_image
                ),
            )
            return await asyncio.to_thread(
                deps.get_project_chat().get_snapshot,
                conversation_id,
                normalized_project_path,
            )
        return snapshot

    @router.post("/api/conversations/{conversation_id}/proposed-plans/{plan_id}/review")
    async def review_proposed_plan(
        conversation_id: str,
        plan_id: str,
        req: ProposedPlanReviewRequest,
    ):
        if req.disposition not in {"approved", "rejected"}:
            raise HTTPException(status_code=400, detail="Proposed plan disposition must be approved or rejected.")
        normalized_project_path = _normalize_project_path_or_400(req.project_path)
        if req.disposition == "approved":
            await _ensure_flow_exists(IMPLEMENT_CHANGE_REQUEST_FLOW)
        try:
            snapshot, proposed_plan, flow_launch = await asyncio.to_thread(
                deps.get_project_chat().review_proposed_plan,
                conversation_id,
                normalized_project_path,
                plan_id,
                req.disposition,
                req.review_note,
            )
        except ValueError as exc:
            raise HTTPException(status_code=400, detail=str(exc)) from exc
        await deps.get_project_chat().publish_snapshot(conversation_id)
        if req.disposition == "approved" and flow_launch is not None:
            try:
                run_id = await _launch_direct_flow(
                    flow_name=flow_launch.flow_name,
                    project_path=normalized_project_path,
                    goal=flow_launch.goal,
                    launch_context=flow_launch.launch_context,
                    model=flow_launch.model,
                    llm_provider=flow_launch.llm_provider,
                    llm_profile=flow_launch.llm_profile,
                    reasoning_effort=flow_launch.reasoning_effort,
                    execution_container_image=flow_launch.execution_container_image,
                )
            except HTTPException as exc:
                await asyncio.to_thread(
                    deps.get_project_chat().fail_proposed_plan_launch,
                    proposed_plan.conversation_id,
                    proposed_plan.id,
                    flow_launch.flow_name,
                    str(exc.detail),
                )
                await deps.get_project_chat().publish_snapshot(proposed_plan.conversation_id)
                return await asyncio.to_thread(
                    deps.get_project_chat().get_snapshot,
                    proposed_plan.conversation_id,
                    normalized_project_path,
                )
            await asyncio.to_thread(
                deps.get_project_chat().note_proposed_plan_launch_started,
                proposed_plan.conversation_id,
                proposed_plan.id,
                run_id,
                flow_launch.flow_name,
            )
            await deps.get_project_chat().publish_snapshot(proposed_plan.conversation_id)
            return await asyncio.to_thread(
                deps.get_project_chat().get_snapshot,
                proposed_plan.conversation_id,
                normalized_project_path,
            )
        return snapshot

    @router.get("/api/projects/metadata")
    async def get_project_metadata(directory: str):
        requested_path = directory.strip()
        if not requested_path:
            raise HTTPException(status_code=400, detail="Project directory path is required.")

        project_path = Path(requested_path).expanduser()
        if not project_path.is_absolute():
            raise HTTPException(status_code=400, detail="Project directory path must be absolute.")

        normalized_path = project_path.resolve(strict=False)
        runtime_path = Path(resolve_runtime_workspace_path(str(normalized_path)))
        return {
            "name": normalized_path.name or str(normalized_path),
            "directory": str(normalized_path),
            "branch": deps.resolve_project_git_branch(runtime_path),
            "commit": deps.resolve_project_git_commit(runtime_path),
        }

    @router.get("/api/projects/browse")
    async def browse_project_directories(path: str | None = None):
        current_path = _normalize_browse_path_or_400(path)
        browse_roots = [normalize_project_path(str(root)) for root in deps.get_settings().project_roots]
        if not current_path.exists():
            raise HTTPException(status_code=404, detail=f"Browse path does not exist: {current_path}")
        if not current_path.is_dir():
            raise HTTPException(status_code=400, detail=f"Browse path is not a directory: {current_path}")

        try:
            entries = sorted(
                (
                    {
                        "name": entry.name,
                        "path": normalize_project_path(str(entry)),
                        "is_dir": True,
                    }
                    for entry in current_path.iterdir()
                    if entry.is_dir()
                ),
                key=lambda entry: (entry["name"].casefold(), entry["name"]),
            )
        except PermissionError as exc:
            raise HTTPException(status_code=403, detail=f"Browse path is not accessible: {current_path}") from exc

        parent_path = current_path.parent
        return {
            "current_path": str(current_path),
            "parent_path": None if parent_path == current_path else str(parent_path),
            "roots": browse_roots,
            "entries": entries,
        }

    return router
