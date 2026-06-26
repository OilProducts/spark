from __future__ import annotations

import os
import subprocess
import time
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any, Callable

import pytest
from fastapi.testclient import TestClient

import attractor.api.server as attractor_server
import spark.app as product_app
import spark.chat.service as project_chat
from spark.workspace.conversations.models import ToolCallRecord
from spark_common.turn_stream import TurnStreamEvent, TurnStreamSource
from tests.api._support import close_task_immediately, wait_for_pipeline_completion


REPO_ROOT = Path(__file__).resolve().parents[3]
WORKFLOW_DIR = Path(__file__).resolve().parent
HARNESS_INVOCATION = ("uv", "run", "pytest", "-q", "tests/acceptance/agent-workflows")

TERMINAL_STATUSES = {"completed", "failed", "validation_error", "canceled", "cancelled"}

RUNNABLE_FLOW = """
digraph AcceptanceRunnable {
  graph [
    label="Acceptance Runnable",
    goal="Exercise acceptance workflow without external provider calls",
    spark.title="Acceptance Runnable",
    spark.description="Deterministic no-provider acceptance flow"
  ];
  start [shape=Mdiamond];
  done [shape=Msquare];
  start -> done;
}
""".strip() + "\n"

RUNNABLE_FLOW_EDITED = """
digraph AcceptanceRunnable {
  graph [
    label="Acceptance Runnable Edited",
    goal="Exercise edited acceptance workflow without external provider calls",
    spark.title="Acceptance Runnable Edited",
    spark.description="Edited deterministic no-provider acceptance flow"
  ];
  start [shape=Mdiamond];
  done [shape=Msquare];
  start -> done [label="edited"];
}
""".strip() + "\n"


@dataclass(frozen=True)
class WorkflowCase:
    workflow_id: str
    markdown_asset: str
    runner_name: str
    coverage: tuple[str, ...]

    @property
    def markdown_path(self) -> Path:
        return WORKFLOW_DIR / self.markdown_asset


@dataclass
class WorkflowResult:
    workflow_id: str
    markdown_asset: str
    outcomes: set[str]
    evidence: dict[str, Any] = field(default_factory=dict)


@dataclass
class AcceptanceContext:
    product: TestClient
    attractor: TestClient
    tmp_path: Path
    monkeypatch: pytest.MonkeyPatch


WORKFLOW_CASES: tuple[WorkflowCase, ...] = (
    WorkflowCase(
        workflow_id="project-select-author-execute-inspect",
        markdown_asset="project-select-author-execute-inspect.md",
        runner_name="run_project_select_author_execute_inspect",
        coverage=(
            "project_registered",
            "active_project_context",
            "conversation_visible",
            "flow_saved_and_reopened",
            "run_launched_and_terminal",
            "run_detail_inspected",
            "followup_run_same_project",
        ),
    ),
    WorkflowCase(
        workflow_id="pipeline-author-workflow",
        markdown_asset="pipeline-author-workflow.md",
        runner_name="run_pipeline_author_workflow",
        coverage=(
            "structured_flow_saved",
            "validation_blocks_invalid_flow",
            "edited_flow_reopened",
        ),
    ),
    WorkflowCase(
        workflow_id="operator-run-workflow",
        markdown_asset="operator-run-workflow.md",
        runner_name="run_operator_run_workflow",
        coverage=(
            "valid_project_launch_available",
            "runtime_status_visible",
            "cancel_request_visible",
            "terminal_state_visible",
        ),
    ),
    WorkflowCase(
        workflow_id="reviewer-auditor-workflow",
        markdown_asset="reviewer-auditor-workflow.md",
        runner_name="run_reviewer_auditor_workflow",
        coverage=(
            "run_history_discoverable",
            "run_detail_stable",
            "events_checkpoint_context_visible",
            "artifacts_visible",
        ),
    ),
    WorkflowCase(
        workflow_id="project-owner-workflow",
        markdown_asset="project-owner-workflow.md",
        runner_name="run_project_owner_workflow",
        coverage=(
            "project_chat_scoped",
            "tool_activity_visible",
            "flow_run_request_reviewed",
            "approved_request_launched",
            "direct_launch_recorded",
            "workflow_events_separate",
        ),
    ),
)


def reset_acceptance_runtime(monkeypatch: pytest.MonkeyPatch, tmp_path: Path):
    for name in (
        "SPARK_HOME",
        "SPARK_FLOWS_DIR",
        "SPARK_PROJECT_ROOTS",
        "SPARK_UI_DIR",
        "CODEX_HOME",
    ):
        monkeypatch.delenv(name, raising=False)
    monkeypatch.setenv("OPENAI_API_KEY", "test-openai-key")
    monkeypatch.setenv("ANTHROPIC_API_KEY", "test-anthropic-key")
    monkeypatch.setenv("GEMINI_API_KEY", "test-gemini-key")

    codex_runtime_root = tmp_path / "codex-runtime"
    codex_home = codex_runtime_root / ".codex"
    codex_home.mkdir(parents=True, exist_ok=True)
    (codex_home / "auth.json").write_text("{}", encoding="utf-8")
    monkeypatch.setenv("ATTRACTOR_CODEX_RUNTIME_ROOT", str(codex_runtime_root))

    attractor_server.shutdown_attractor_runtime()
    product_app.configure_settings(
        data_dir=tmp_path / ".spark",
        flows_dir=tmp_path / "flows",
        ui_dir=None,
    )
    with attractor_server.ACTIVE_RUNS_LOCK:
        attractor_server.ACTIVE_RUNS.clear()
    attractor_server.HUMAN_BROKER = attractor_server.HumanGateBroker()
    attractor_server.EVENT_HUB = attractor_server.PipelineEventHub()
    attractor_server.RUNTIME.status = "idle"
    attractor_server.RUNTIME.outcome = None
    attractor_server.RUNTIME.outcome_reason_code = None
    attractor_server.RUNTIME.outcome_reason_message = None
    attractor_server.RUNTIME.last_error = ""
    attractor_server.RUNTIME.last_working_directory = ""
    attractor_server.RUNTIME.last_model = ""
    attractor_server.RUNTIME.last_completed_nodes = []
    attractor_server.RUNTIME.last_flow_name = ""
    attractor_server.clear_registered_transforms()
    yield
    attractor_server.shutdown_attractor_runtime()
    with attractor_server.ACTIVE_RUNS_LOCK:
        attractor_server.ACTIVE_RUNS.clear()
    attractor_server.clear_registered_transforms()


def workflow_case_runner(case: WorkflowCase) -> Callable[[AcceptanceContext, WorkflowCase], WorkflowResult]:
    try:
        return CASE_RUNNERS[case.runner_name]
    except KeyError as exc:
        raise AssertionError(f"Unknown workflow runner: {case.runner_name}") from exc


def run_project_select_author_execute_inspect(ctx: AcceptanceContext, case: WorkflowCase) -> WorkflowResult:
    project_path = register_project(ctx, "full-journey-project")
    conversation = send_project_chat_turn(
        ctx,
        conversation_id="conversation-full-journey",
        project_path=project_path,
        message="Prepare a project-scoped build run.",
        assistant="Project context is active and ready for execution.",
    )
    set_active_conversation(ctx, project_path, conversation["conversation_id"])

    save_flow(ctx, "acceptance/full-journey.dot", RUNNABLE_FLOW)
    validate_payload = validate_flow(ctx, "acceptance/full-journey.dot")
    reopened = get_flow_raw(ctx, "acceptance/full-journey.dot")

    launch = launch_workspace_run(
        ctx,
        flow_name="acceptance/full-journey.dot",
        project_path=project_path,
        conversation_handle=conversation["conversation_handle"],
        summary="Launch the full acceptance journey.",
    )
    completed = wait_for_pipeline_completion(ctx.attractor, launch["run_id"])
    inspected = inspect_run(ctx, launch["run_id"])

    save_flow(ctx, "acceptance/full-journey.dot", RUNNABLE_FLOW_EDITED)
    edited_raw = get_flow_raw(ctx, "acceptance/full-journey.dot")
    followup = launch_workspace_run(
        ctx,
        flow_name="acceptance/full-journey.dot",
        project_path=project_path,
        summary="Launch follow-up after an edit.",
    )
    followup_completed = wait_for_pipeline_completion(ctx.attractor, followup["run_id"])

    assert validate_payload["graph"]["nodes"], validate_payload
    assert "Acceptance Runnable" in reopened
    assert "Acceptance Runnable Edited" in edited_raw
    assert completed["status"] == "completed"
    assert completed["project_path"] == project_path
    assert inspected["status"]["project_path"] == project_path
    assert followup_completed["status"] == "completed"
    assert followup_completed["project_path"] == project_path

    return result_for(
        case,
        case.coverage,
        project_path=project_path,
        conversation_id=conversation["conversation_id"],
        run_ids=[launch["run_id"], followup["run_id"]],
    )


def run_pipeline_author_workflow(ctx: AcceptanceContext, case: WorkflowCase) -> WorkflowResult:
    save_flow(ctx, "acceptance/authoring.dot", RUNNABLE_FLOW)
    flow_description = describe_flow(ctx, "acceptance/authoring.dot")
    invalid_response = ctx.attractor.post(
        "/api/flows",
        json={
            "name": "acceptance/invalid-authoring.dot",
            "content": "digraph Broken { start [shape=Mdiamond] start -> missing }",
        },
    )

    save_flow(ctx, "acceptance/authoring.dot", RUNNABLE_FLOW_EDITED)
    reopened = get_flow_raw(ctx, "acceptance/authoring.dot")
    validate_payload = validate_flow(ctx, "acceptance/authoring.dot")

    assert flow_description["node_count"] >= 2
    assert flow_description["edge_count"] >= 1
    assert invalid_response.status_code == 422
    assert invalid_response.json()["detail"]["errors"]
    assert "Acceptance Runnable Edited" in reopened
    assert validate_payload["status"] == "ok"
    assert validate_payload["errors"] == []

    return result_for(case, case.coverage, flow_name="acceptance/authoring.dot")


def run_operator_run_workflow(ctx: AcceptanceContext, case: WorkflowCase) -> WorkflowResult:
    project_path = register_project(ctx, "operator-project")
    save_flow(ctx, "acceptance/operator.dot", RUNNABLE_FLOW)

    terminal_launch = launch_workspace_run(
        ctx,
        flow_name="acceptance/operator.dot",
        project_path=project_path,
        summary="Launch operator terminal-state check.",
    )
    terminal_payload = wait_for_pipeline_completion(ctx.attractor, terminal_launch["run_id"])

    ctx.monkeypatch.setattr(attractor_server.asyncio, "create_task", close_task_immediately)
    running_payload = ctx.attractor.post(
        "/pipelines",
        json={
            "flow_content": RUNNABLE_FLOW,
            "working_directory": project_path,
        },
    )
    assert running_payload.status_code == 200
    running_run_id = running_payload.json()["run_id"]
    status_payload = ctx.attractor.get(f"/pipelines/{running_run_id}").json()
    cancel_response = ctx.attractor.post(f"/pipelines/{running_run_id}/cancel")
    canceled_status = ctx.attractor.get(f"/pipelines/{running_run_id}").json()

    assert terminal_payload["status"] == "completed"
    assert terminal_payload["project_path"] == project_path
    assert status_payload["status"] == "running"
    assert cancel_response.status_code == 200
    assert cancel_response.json()["status"] == "cancel_requested"
    assert canceled_status["status"] == "cancel_requested"
    assert canceled_status["last_error"] == "cancel_requested_by_user"

    return result_for(
        case,
        case.coverage,
        project_path=project_path,
        terminal_run_id=terminal_launch["run_id"],
        canceled_run_id=running_run_id,
    )


def run_reviewer_auditor_workflow(ctx: AcceptanceContext, case: WorkflowCase) -> WorkflowResult:
    project_path = register_project(ctx, "reviewer-project")
    save_flow(ctx, "acceptance/reviewer.dot", RUNNABLE_FLOW)
    launch = launch_workspace_run(
        ctx,
        flow_name="acceptance/reviewer.dot",
        project_path=project_path,
        summary="Launch reviewer inspection run.",
    )
    completed = wait_for_pipeline_completion(ctx.attractor, launch["run_id"])
    run_root = attractor_server._run_root(launch["run_id"])
    artifact = run_root / "artifacts" / "review" / "summary.txt"
    artifact.parent.mkdir(parents=True, exist_ok=True)
    artifact.write_text("review evidence", encoding="utf-8")

    runs_payload = ctx.product.get("/attractor/runs", params={"project_path": project_path}).json()
    inspected = inspect_run(ctx, launch["run_id"])
    artifacts = ctx.attractor.get(f"/pipelines/{launch['run_id']}/artifacts").json()
    artifact_response = ctx.attractor.get(f"/pipelines/{launch['run_id']}/artifacts/artifacts/review/summary.txt")

    assert completed["status"] == "completed"
    assert any(run["run_id"] == launch["run_id"] for run in runs_payload["runs"])
    assert inspected["status"]["status"] == "completed"
    assert inspected["context"]["pipeline_id"] == launch["run_id"]
    assert inspected["checkpoint"]["pipeline_id"] == launch["run_id"]
    assert artifacts["artifacts"]
    assert any(item["path"] == "artifacts/review/summary.txt" for item in artifacts["artifacts"])
    assert artifact_response.status_code == 200
    assert artifact_response.text == "review evidence"

    return result_for(case, case.coverage, project_path=project_path, run_id=launch["run_id"])


def run_project_owner_workflow(ctx: AcceptanceContext, case: WorkflowCase) -> WorkflowResult:
    project_path = register_project(ctx, "owner-project")
    save_flow(ctx, "acceptance/owner.dot", RUNNABLE_FLOW)
    conversation = send_project_chat_turn(
        ctx,
        conversation_id="conversation-owner",
        project_path=project_path,
        message="Please prepare a flow run request for this project.",
        assistant="I prepared the project run request.",
    )

    create_request = ctx.product.post(
        f"/workspace/api/conversations/by-handle/{conversation['conversation_handle']}/flow-run-requests",
        json={
            "flow_name": "acceptance/owner.dot",
            "summary": "Run the owner-approved acceptance flow.",
            "goal": "Exercise owner approval launch.",
            "launch_context": {"context.acceptance.workflow": case.workflow_id},
        },
    )
    assert create_request.status_code == 200
    request_id = create_request.json()["flow_run_request_id"]

    review = ctx.product.post(
        f"/workspace/api/conversations/{conversation['conversation_id']}/flow-run-requests/{request_id}/review",
        json={
            "project_path": project_path,
            "disposition": "approved",
            "message": "Approved for acceptance execution.",
        },
    )
    assert review.status_code == 200
    reviewed_snapshot = review.json()
    request_payload = next(entry for entry in reviewed_snapshot["flow_run_requests"] if entry["id"] == request_id)

    direct_launch = launch_workspace_run(
        ctx,
        flow_name="acceptance/owner.dot",
        project_path=project_path,
        conversation_handle=conversation["conversation_handle"],
        summary="Direct owner follow-up launch.",
    )
    final_snapshot = get_conversation(ctx, conversation["conversation_id"], project_path)
    flow_launch = next(
        entry for entry in final_snapshot["flow_launches"] if entry["id"] == direct_launch["flow_launch_id"]
    )
    tool_segments = [segment for segment in final_snapshot["segments"] if segment["kind"] == "tool_call"]

    assert final_snapshot["project_path"] == project_path
    assert tool_segments
    assert tool_segments[0]["tool_call"]["status"] == "completed"
    assert request_payload["status"] == "launched"
    assert request_payload["run_id"]
    assert flow_launch["status"] == "launched"
    assert flow_launch["run_id"] == direct_launch["run_id"]
    event_messages = [event["message"] for event in final_snapshot["event_log"]]
    assert any("Created" in message for message in event_messages)
    assert any("Launched" in message for message in event_messages)

    return result_for(
        case,
        case.coverage,
        project_path=project_path,
        conversation_id=conversation["conversation_id"],
        request_run_id=request_payload["run_id"],
        direct_run_id=direct_launch["run_id"],
    )


CASE_RUNNERS: dict[str, Callable[[AcceptanceContext, WorkflowCase], WorkflowResult]] = {
    "run_project_select_author_execute_inspect": run_project_select_author_execute_inspect,
    "run_pipeline_author_workflow": run_pipeline_author_workflow,
    "run_operator_run_workflow": run_operator_run_workflow,
    "run_reviewer_auditor_workflow": run_reviewer_auditor_workflow,
    "run_project_owner_workflow": run_project_owner_workflow,
}


def result_for(
    case: WorkflowCase,
    outcomes: tuple[str, ...] | set[str],
    **evidence: Any,
) -> WorkflowResult:
    return WorkflowResult(
        workflow_id=case.workflow_id,
        markdown_asset=case.markdown_asset,
        outcomes=set(outcomes),
        evidence=evidence,
    )


def register_project(ctx: AcceptanceContext, name: str) -> str:
    project_dir = (ctx.tmp_path / name).resolve()
    project_dir.mkdir(parents=True, exist_ok=True)
    response = ctx.product.post("/workspace/api/projects/register", json={"project_path": str(project_dir)})
    assert response.status_code == 200
    payload = response.json()
    assert payload["project_path"] == str(project_dir)
    assert payload["display_name"] == name

    projects = ctx.product.get("/workspace/api/projects")
    assert projects.status_code == 200
    assert any(project["project_path"] == str(project_dir) for project in projects.json())
    return str(project_dir)


def set_active_conversation(ctx: AcceptanceContext, project_path: str, conversation_id: str) -> None:
    response = ctx.product.patch(
        "/workspace/api/projects/state",
        json={
            "project_path": project_path,
            "active_conversation_id": conversation_id,
            "last_accessed_at": "2026-06-26T00:00:00Z",
        },
    )
    assert response.status_code == 200
    payload = response.json()
    assert payload["active_conversation_id"] == conversation_id
    assert payload["project_path"] == project_path


def send_project_chat_turn(
    ctx: AcceptanceContext,
    *,
    conversation_id: str,
    project_path: str,
    message: str,
    assistant: str,
) -> dict[str, Any]:
    class ScriptedSession:
        def turn(self, prompt: str, model: str | None, **kwargs: Any) -> project_chat.ChatTurnResult:
            assert project_path in prompt
            assert model == "gpt-acceptance"
            on_event = kwargs.get("on_event")
            if on_event is not None:
                on_event(
                    TurnStreamEvent(
                        kind="content_completed",
                        channel="reasoning",
                        content_delta="Checked the active project context.",
                        source=TurnStreamSource(app_turn_id="acceptance-turn", item_id="reasoning"),
                    )
                )
                on_event(
                    TurnStreamEvent(
                        kind="tool_call_started",
                        source=TurnStreamSource(app_turn_id="acceptance-turn", item_id="tool-context"),
                        tool_call=ToolCallRecord(
                            id="tool-context",
                            kind="command_execution",
                            status="running",
                            title="Inspect project context",
                            command="pwd",
                        ),
                    )
                )
                on_event(
                    TurnStreamEvent(
                        kind="tool_call_completed",
                        source=TurnStreamSource(app_turn_id="acceptance-turn", item_id="tool-context"),
                        tool_call=ToolCallRecord(
                            id="tool-context",
                            kind="command_execution",
                            status="completed",
                            title="Inspect project context",
                            command="pwd",
                            output=project_path,
                        ),
                    )
                )
                on_event(
                    TurnStreamEvent(
                        kind="content_completed",
                        channel="assistant",
                        content_delta=assistant,
                        phase="final_answer",
                        source=TurnStreamSource(app_turn_id="acceptance-turn", item_id="final"),
                    )
                )
            return project_chat.ChatTurnResult(assistant_message=assistant)

    service = product_app.get_project_chat()
    ctx.monkeypatch.setattr(service, "_build_session", lambda *args, **kwargs: ScriptedSession())
    response = ctx.product.post(
        f"/workspace/api/conversations/{conversation_id}/turns",
        json={
            "project_path": project_path,
            "message": message,
            "model": "gpt-acceptance",
        },
    )
    assert response.status_code == 200
    snapshot = wait_for_conversation_complete(ctx, conversation_id, project_path)
    assert snapshot["project_path"] == project_path
    assert snapshot["turns"][-1]["status"] == "complete"
    assert snapshot["turns"][-1]["content"] == assistant
    return snapshot


def wait_for_conversation_complete(
    ctx: AcceptanceContext,
    conversation_id: str,
    project_path: str,
    *,
    timeout_seconds: float = 2.0,
) -> dict[str, Any]:
    deadline = time.time() + timeout_seconds
    last_snapshot: dict[str, Any] | None = None
    while time.time() < deadline:
        snapshot = get_conversation(ctx, conversation_id, project_path)
        last_snapshot = snapshot
        if snapshot["turns"] and snapshot["turns"][-1]["status"] == "complete":
            return snapshot
        time.sleep(0.02)
    raise AssertionError(f"conversation did not complete: {last_snapshot}")


def get_conversation(ctx: AcceptanceContext, conversation_id: str, project_path: str) -> dict[str, Any]:
    response = ctx.product.get(
        f"/workspace/api/conversations/{conversation_id}",
        params={"project_path": project_path},
    )
    assert response.status_code == 200
    return response.json()


def save_flow(ctx: AcceptanceContext, flow_name: str, content: str) -> dict[str, Any]:
    response = ctx.attractor.post("/api/flows", json={"name": flow_name, "content": content})
    assert response.status_code == 200
    payload = response.json()
    assert payload["status"] == "saved"
    assert payload["name"] == flow_name
    return payload


def describe_flow(ctx: AcceptanceContext, flow_name: str) -> dict[str, Any]:
    response = ctx.product.get(f"/workspace/api/flows/{flow_name}", params={"surface": "human"})
    assert response.status_code == 200
    return response.json()


def validate_flow(ctx: AcceptanceContext, flow_name: str) -> dict[str, Any]:
    response = ctx.product.get(f"/workspace/api/flows/{flow_name}/validate")
    assert response.status_code == 200
    payload = response.json()
    assert payload["name"] == flow_name
    return payload


def get_flow_raw(ctx: AcceptanceContext, flow_name: str) -> str:
    response = ctx.product.get(f"/workspace/api/flows/{flow_name}/raw", params={"surface": "human"})
    assert response.status_code == 200
    assert response.headers["X-Spark-Flow-Name"] == flow_name
    return response.text


def launch_workspace_run(
    ctx: AcceptanceContext,
    *,
    flow_name: str,
    project_path: str,
    summary: str,
    conversation_handle: str | None = None,
) -> dict[str, Any]:
    payload: dict[str, object] = {
        "flow_name": flow_name,
        "summary": summary,
        "project_path": project_path,
        "launch_context": {
            "context.acceptance.flow_name": flow_name,
            "context.acceptance.summary": summary,
        },
    }
    if conversation_handle is not None:
        payload["conversation_handle"] = conversation_handle
    response = ctx.product.post("/workspace/api/runs/launch", json=payload)
    assert response.status_code == 200
    launch = response.json()
    assert launch["ok"] is True
    assert launch["status"] == "started"
    assert launch["flow_name"] == flow_name
    assert launch["project_path"] == project_path
    assert launch["run_id"]
    return launch


def inspect_run(ctx: AcceptanceContext, run_id: str) -> dict[str, Any]:
    status = ctx.attractor.get(f"/pipelines/{run_id}")
    context = ctx.attractor.get(f"/pipelines/{run_id}/context")
    checkpoint = ctx.attractor.get(f"/pipelines/{run_id}/checkpoint")
    journal = ctx.attractor.get(f"/pipelines/{run_id}/journal")
    artifacts = ctx.attractor.get(f"/pipelines/{run_id}/artifacts")
    assert status.status_code == 200
    assert context.status_code == 200
    assert checkpoint.status_code == 200
    assert journal.status_code == 200
    assert artifacts.status_code == 200
    return {
        "status": status.json(),
        "context": context.json(),
        "checkpoint": checkpoint.json(),
        "journal": journal.json(),
        "artifacts": artifacts.json(),
    }


def collect_harness_with_uv() -> subprocess.CompletedProcess[str]:
    env = os.environ.copy()
    env.pop("PYTEST_CURRENT_TEST", None)
    return subprocess.run(
        ("uv", "run", "pytest", "--collect-only", "-q", "tests/acceptance/agent-workflows"),
        cwd=REPO_ROOT,
        env=env,
        capture_output=True,
        text=True,
        check=False,
    )
