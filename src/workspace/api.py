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
from spark_common.runtime import normalize_project_path, resolve_runtime_workspace_path
from workspace.attractor_client import AttractorApiClient, AttractorApiError
from workspace.flow_catalog import (
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
from workspace.project_chat_common import normalize_flow_run_request_payload
from workspace.project_chat import ProjectChatService, TurnInProgressError
from workspace.storage import (
    delete_project_record,
    list_project_records,
    read_project_record,
    update_project_record,
)
from workspace.triggers import (
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


class ConversationTurnRequest(BaseModel):
    project_path: str
    message: str
    model: Optional[str] = None


class ProjectRegistrationRequest(BaseModel):
    project_path: str


class ProjectStateUpdateRequest(BaseModel):
    project_path: str
    is_favorite: Optional[bool] = None
    last_accessed_at: Optional[str] = None
    active_conversation_id: Optional[str] = None


class SpecEditApprovalRequest(BaseModel):
    project_path: str
    model: Optional[str] = None
    flow_source: Optional[str] = None


class SpecEditProposalChangeRequest(BaseModel):
    path: str
    before: str
    after: str


class SpecEditProposalCreateByHandleRequest(BaseModel):
    model_config = ConfigDict(extra="forbid")
    summary: str
    changes: list[SpecEditProposalChangeRequest]
    rationale: Optional[str] = None


class SpecEditRejectionRequest(BaseModel):
    project_path: str


class FlowRunRequestCreateByHandleRequest(BaseModel):
    model_config = ConfigDict(extra="forbid")
    flow_name: str
    summary: str
    goal: Optional[str] = None
    launch_context: Optional[dict[str, Any]] = None
    model: Optional[str] = None


class FlowRunRequestReviewRequest(BaseModel):
    project_path: str
    disposition: str
    message: str
    flow_name: Optional[str] = None
    model: Optional[str] = None


class RunLaunchRequest(BaseModel):
    model_config = ConfigDict(extra="forbid")
    flow_name: str
    summary: str
    conversation_handle: Optional[str] = None
    project_path: Optional[str] = None
    goal: Optional[str] = None
    launch_context: Optional[dict[str, Any]] = None
    model: Optional[str] = None


class FlowLaunchPolicyUpdateRequest(BaseModel):
    launch_policy: str


class ExecutionCardReviewRequest(BaseModel):
    project_path: str
    disposition: str
    message: str
    model: Optional[str] = None
    flow_source: Optional[str] = None


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


@dataclass(frozen=True)
class WorkspaceApiDependencies:
    get_settings: Callable[[], WorkspaceSettings]
    get_project_chat: Callable[[], ProjectChatService]
    get_attractor_client: Callable[[], AttractorApiClient]
    resolve_project_git_branch: Callable[[Path], Optional[str]]
    resolve_project_git_commit: Callable[[Path], Optional[str]]
    pick_project_directory: Callable[[], Optional[Path]]
    get_trigger_runtime: Callable[[], TriggerRuntime]
    default_execution_planning_flow: str
    default_execution_dispatch_flow: str


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

    def _validate_flow_surface(surface: Optional[str]) -> str:
        normalized_surface = str(surface or "human").strip().lower()
        if normalized_surface not in {"human", "agent"}:
            raise HTTPException(status_code=400, detail="Flow surface must be 'human' or 'agent'.")
        return normalized_surface

    def _find_workspace_event_trigger(project_path: str, event_name: str) -> str | None:
        matching_flow: str | None = None
        for definition in list_trigger_definitions(deps.get_settings().config_dir):
            if definition.source_type != "workspace_event" or not definition.enabled:
                continue
            if str(definition.source.get("event_name") or "").strip() != event_name:
                continue
            configured_project = (definition.action.project_path or "").strip()
            if configured_project and configured_project != project_path:
                continue
            if matching_flow is None or configured_project == project_path:
                matching_flow = definition.action.flow_name
        return matching_flow

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

    async def _read_stage_response(run_id: str, stage_id: str) -> str:
        client = deps.get_attractor_client()
        for artifact_path in (f"logs/{stage_id}/response.md", f"{stage_id}/response.md"):
            try:
                text = await client.get_artifact_text(run_id, artifact_path)
            except AttractorApiError:
                continue
            if text.strip():
                return text
        raise RuntimeError(f"Run {run_id} completed without a response artifact for stage {stage_id}.")

    async def _launch_execution_planning_pipeline(
        *,
        conversation_id: str,
        proposal_id: str,
        workflow_run_id: str,
        flow_source: str,
        execution_flow_source: str,
        model: Optional[str],
        review_feedback: Optional[str],
    ) -> None:
        project_chat = deps.get_project_chat()
        client = deps.get_attractor_client()
        launch_spec = await asyncio.to_thread(
            project_chat.prepare_execution_workflow_launch,
            conversation_id,
            proposal_id,
            review_feedback,
        )
        try:
            launch_payload = await client.start_pipeline(
                run_id=workflow_run_id,
                flow_name=flow_source,
                working_directory=launch_spec.project_path,
                model=model,
                goal=launch_spec.prompt,
                spec_id=launch_spec.spec_id,
            )
        except AttractorApiError as exc:
            await asyncio.to_thread(
                project_chat.fail_execution_workflow,
                conversation_id,
                workflow_run_id,
                flow_source,
                str(exc),
            )
            await project_chat.publish_snapshot(conversation_id)
            raise HTTPException(status_code=exc.status_code, detail=str(exc)) from exc

        if launch_payload.get("status") != "started":
            error = str(launch_payload.get("error") or "Execution planning flow could not be started.")
            await asyncio.to_thread(
                project_chat.fail_execution_workflow,
                conversation_id,
                workflow_run_id,
                flow_source,
                error,
            )
            await project_chat.publish_snapshot(conversation_id)
            raise HTTPException(status_code=500, detail=error)

        await deps.get_trigger_runtime().observe_run(
            run_id=workflow_run_id,
            flow_name=flow_source,
            project_path=launch_spec.project_path,
        )

        async def monitor() -> None:
            try:
                payload = await _wait_for_pipeline_terminal_status(workflow_run_id)
                completed_status = str(payload.get("status", "")).strip().lower()
                completed_outcome = str(payload.get("outcome", "")).strip().lower()
                if completed_status != "completed" or completed_outcome != "success":
                    error = str(
                        payload.get("last_error")
                        or payload.get("outcome_reason_message")
                        or f"Execution planning pipeline ended with status '{completed_status}' and outcome '{completed_outcome or '—'}'."
                    ).strip()
                    await asyncio.to_thread(
                        project_chat.fail_execution_workflow,
                        conversation_id,
                        workflow_run_id,
                        flow_source,
                        error,
                    )
                    await project_chat.publish_snapshot(conversation_id)
                    return

                raw_response = await _read_stage_response(workflow_run_id, "generate_execution_card")
                execution_card = await asyncio.to_thread(
                    project_chat.complete_execution_workflow,
                    conversation_id,
                    proposal_id,
                    flow_source,
                    execution_flow_source,
                    workflow_run_id,
                    raw_response,
                )
                await client.update_pipeline_metadata(
                    workflow_run_id,
                    plan_id=execution_card.id,
                )
                await project_chat.publish_snapshot(conversation_id)
            except Exception as exc:  # noqa: BLE001
                await asyncio.to_thread(
                    project_chat.fail_execution_workflow,
                    conversation_id,
                    workflow_run_id,
                    flow_source,
                    str(exc),
                )
                await project_chat.publish_snapshot(conversation_id)

        asyncio.create_task(monitor())

    async def _launch_execution_card_pipeline(
        *,
        conversation_id: str,
        execution_card_id: str,
        project_path: str,
        flow_source: str,
        model: Optional[str],
        spec_id: str,
        plan_id: str,
    ) -> str:
        project_chat = deps.get_project_chat()
        try:
            launch_payload = await deps.get_attractor_client().start_pipeline(
                run_id=None,
                flow_name=flow_source,
                working_directory=project_path,
                model=model,
                spec_id=spec_id,
                plan_id=plan_id,
            )
        except AttractorApiError as exc:
            raise HTTPException(status_code=exc.status_code, detail=str(exc)) from exc

        if launch_payload.get("status") != "started":
            error = str(launch_payload.get("error") or "Execution flow could not be started.")
            raise HTTPException(status_code=500, detail=error)

        run_id = str(launch_payload.get("run_id") or "")
        if not run_id:
            raise HTTPException(status_code=500, detail="Execution flow did not return a run id.")

        await deps.get_trigger_runtime().observe_run(
            run_id=run_id,
            flow_name=flow_source,
            project_path=project_path,
        )
        await asyncio.to_thread(
            project_chat.note_execution_card_dispatched,
            conversation_id,
            execution_card_id,
            run_id,
            flow_source,
        )
        await project_chat.publish_snapshot(conversation_id)
        return run_id

    async def _launch_flow_run_request_pipeline(
        *,
        conversation_id: str,
        request_id: str,
        project_path: str,
        flow_name: str,
        goal: Optional[str],
        launch_context: Optional[dict[str, Any]],
        model: Optional[str],
    ) -> str | None:
        project_chat = deps.get_project_chat()
        try:
            launch_payload = await deps.get_attractor_client().start_pipeline(
                run_id=None,
                flow_name=flow_name,
                working_directory=project_path,
                model=model,
                goal=goal,
                launch_context=launch_context,
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
    ) -> str:
        try:
            launch_payload = await deps.get_attractor_client().start_pipeline(
                run_id=None,
                flow_name=flow_name,
                working_directory=project_path,
                model=model,
                goal=goal,
                launch_context=launch_context,
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

    @router.get("/api/conversations/{conversation_id}/events")
    async def project_conversation_events(conversation_id: str, request: Request, project_path: Optional[str] = None):
        project_chat = deps.get_project_chat()
        try:
            snapshot = await asyncio.to_thread(project_chat.get_snapshot, conversation_id, project_path)
        except FileNotFoundError as exc:
            raise HTTPException(status_code=404, detail=f"Unknown conversation: {conversation_id}") from exc
        except ValueError as exc:
            raise HTTPException(status_code=400, detail=str(exc)) from exc

        queue = project_chat.events().subscribe(conversation_id)

        async def stream():
            try:
                yield f"data: {json.dumps({'type': 'conversation_snapshot', 'state': snapshot})}\n\n"
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
        loop = asyncio.get_running_loop()
        project_chat = deps.get_project_chat()

        def publish_progress_event(event: dict[str, Any]) -> None:
            asyncio.run_coroutine_threadsafe(
                project_chat.events().publish(
                    conversation_id,
                    event,
                ),
                loop,
            )

        try:
            snapshot = await asyncio.to_thread(
                project_chat.start_turn,
                conversation_id,
                req.project_path,
                req.message,
                req.model,
                publish_progress_event,
            )
        except ValueError as exc:
            raise HTTPException(status_code=400, detail=str(exc)) from exc
        except TurnInProgressError as exc:
            raise HTTPException(status_code=409, detail=str(exc)) from exc
        except RuntimeError as exc:
            raise HTTPException(status_code=503, detail=str(exc)) from exc
        return snapshot

    @router.post("/api/conversations/by-handle/{conversation_handle}/spec-edit-proposals")
    async def create_project_spec_edit_proposal_by_handle(
        conversation_handle: str,
        req: SpecEditProposalCreateByHandleRequest,
    ):
        try:
            result = await asyncio.to_thread(
                deps.get_project_chat().create_spec_edit_proposal_by_handle,
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
            status_code = 409 if "identical proposal already exists" in detail else 400
            raise HTTPException(status_code=status_code, detail=detail) from exc
        await deps.get_project_chat().publish_snapshot(str(result["conversation_id"]))
        return {
            "ok": True,
            **result,
        }

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

    @router.post("/api/conversations/{conversation_id}/spec-edit-proposals/{proposal_id}/approve")
    async def approve_project_spec_edit_proposal(
        conversation_id: str,
        proposal_id: str,
        req: SpecEditApprovalRequest,
    ):
        normalized_project_path = _normalize_project_path_or_400(req.project_path)
        effective_flow_source = (
            (req.flow_source or "").strip()
            or _find_workspace_event_trigger(normalized_project_path, "spec_edit_approved")
            or deps.default_execution_planning_flow
        )
        await _ensure_flow_exists(effective_flow_source)
        try:
            snapshot, proposal = await asyncio.to_thread(
                deps.get_project_chat().approve_spec_edit,
                conversation_id,
                normalized_project_path,
                proposal_id,
            )
            workflow_run_id = f"workflow-{uuid.uuid4().hex[:12]}"
            snapshot = await asyncio.to_thread(
                deps.get_project_chat().mark_execution_workflow_started,
                conversation_id,
                workflow_run_id,
                effective_flow_source,
            )
        except ValueError as exc:
            raise HTTPException(status_code=400, detail=str(exc)) from exc

        await _launch_execution_planning_pipeline(
            conversation_id=conversation_id,
            proposal_id=proposal.id,
            workflow_run_id=workflow_run_id,
            flow_source=effective_flow_source,
            execution_flow_source=deps.default_execution_dispatch_flow,
            model=req.model,
            review_feedback=None,
        )
        await deps.get_project_chat().publish_snapshot(conversation_id)
        return snapshot

    @router.post("/api/conversations/{conversation_id}/spec-edit-proposals/{proposal_id}/reject")
    async def reject_project_spec_edit_proposal(
        conversation_id: str,
        proposal_id: str,
        req: SpecEditRejectionRequest,
    ):
        normalized_project_path = _normalize_project_path_or_400(req.project_path)
        try:
            snapshot = await asyncio.to_thread(
                deps.get_project_chat().reject_spec_edit,
                conversation_id,
                normalized_project_path,
                proposal_id,
            )
        except ValueError as exc:
            raise HTTPException(status_code=400, detail=str(exc)) from exc
        await deps.get_project_chat().publish_snapshot(conversation_id)
        return snapshot

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
            )
            return await asyncio.to_thread(
                deps.get_project_chat().get_snapshot,
                conversation_id,
                normalized_project_path,
            )
        return snapshot

    @router.post("/api/conversations/{conversation_id}/execution-cards/{execution_card_id}/review")
    async def review_project_execution_card(
        conversation_id: str,
        execution_card_id: str,
        req: ExecutionCardReviewRequest,
    ):
        if req.disposition not in {"approved", "rejected", "revision_requested"}:
            raise HTTPException(status_code=400, detail="Execution card disposition must be approved, rejected, or revision_requested.")
        normalized_project_path = _normalize_project_path_or_400(req.project_path)
        review_flow_source = None
        if req.disposition != "approved":
            planning_trigger = (
                "execution_card_revision_requested"
                if req.disposition == "revision_requested"
                else "execution_card_rejected"
            )
            review_flow_source = (
                _find_workspace_event_trigger(normalized_project_path, planning_trigger)
                or deps.default_execution_planning_flow
            )
        try:
            snapshot, execution_card, proposal_id, workflow_run_id = await asyncio.to_thread(
                deps.get_project_chat().review_execution_card,
                conversation_id,
                normalized_project_path,
                execution_card_id,
                req.disposition,
                req.message,
                review_flow_source,
            )
        except ValueError as exc:
            raise HTTPException(status_code=400, detail=str(exc)) from exc
        await deps.get_project_chat().publish_snapshot(conversation_id)
        if req.disposition == "approved":
            persisted_execution_flow = (execution_card.flow_source or "").strip()
            effective_flow_source = (
                (req.flow_source or "").strip()
                or _find_workspace_event_trigger(normalized_project_path, "execution_card_approved")
                or persisted_execution_flow
                or deps.default_execution_dispatch_flow
            )
            await _ensure_flow_exists(effective_flow_source)
            await _launch_execution_card_pipeline(
                conversation_id=conversation_id,
                execution_card_id=execution_card.id,
                project_path=normalized_project_path,
                flow_source=effective_flow_source,
                model=req.model,
                spec_id=execution_card.source_spec_edit_id,
                plan_id=execution_card.id,
            )
        elif proposal_id and workflow_run_id:
            resolved_review_flow = review_flow_source or deps.default_execution_planning_flow
            await _ensure_flow_exists(resolved_review_flow)
            await _launch_execution_planning_pipeline(
                conversation_id=conversation_id,
                proposal_id=proposal_id,
                workflow_run_id=workflow_run_id,
                flow_source=resolved_review_flow,
                execution_flow_source=deps.default_execution_dispatch_flow,
                model=req.model,
                review_feedback=req.message,
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

    @router.post("/api/projects/pick-directory")
    async def pick_project_directory():
        try:
            selected_directory = await asyncio.to_thread(deps.pick_project_directory)
        except RuntimeError as exc:
            raise HTTPException(status_code=503, detail=str(exc)) from exc
        if selected_directory is None:
            return {"status": "canceled"}
        if not selected_directory.is_absolute():
            raise HTTPException(status_code=500, detail="Directory picker returned a non-absolute path.")
        return {
            "status": "selected",
            "directory_path": str(selected_directory),
        }

    return router
