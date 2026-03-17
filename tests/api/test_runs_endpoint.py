from __future__ import annotations

import json
from pathlib import Path
import time

import pytest
from fastapi.testclient import TestClient

import workspace.project_chat as project_chat
import workspace.project_chat_models as project_chat_models
import attractor.api.server as server


def _write_flow(name: str, content: str = "digraph G { start [shape=Mdiamond]; done [shape=Msquare]; start -> done; }\n") -> None:
    flows_dir = server.get_settings().flows_dir
    flows_dir.mkdir(parents=True, exist_ok=True)
    (flows_dir / name).write_text(content, encoding="utf-8")


def test_list_runs_includes_project_and_git_metadata_fields(
    api_client: TestClient,
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    run_id = "run-with-project-metadata"

    server._write_run_meta(
        server.RunRecord(
            run_id=run_id,
            flow_name="Flow",
            status="success",
            result="success",
            working_directory=str(tmp_path / "work"),
            model="test-model",
            started_at="2026-01-01T00:00:00Z",
            ended_at="2026-01-01T00:01:00Z",
            project_path=str(tmp_path / "project"),
            git_branch="main",
            git_commit="abc123",
            last_error="",
            token_usage=42,
        )
    )

    response = api_client.get("/attractor/runs")

    assert response.status_code == 200
    payload = response.json()
    assert len(payload["runs"]) == 1
    run_payload = payload["runs"][0]
    assert run_payload["project_path"] == str(tmp_path / "project")
    assert run_payload["git_branch"] == "main"
    assert run_payload["git_commit"] == "abc123"


def test_list_runs_includes_spec_and_plan_artifact_links_when_available_item_9_6_03(
    api_client: TestClient,
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    run_id = "run-with-artifact-links"

    server._write_run_meta(
        server.RunRecord(
            run_id=run_id,
            flow_name="Flow",
            status="success",
            result="success",
            working_directory=str(tmp_path / "work"),
            model="test-model",
            started_at="2026-01-01T00:00:00Z",
            ended_at="2026-01-01T00:01:00Z",
            project_path=str(tmp_path / "project"),
            git_branch="main",
            git_commit="abc123",
            spec_id="spec-project-1700000000",
            plan_id="plan-project-1700000000",
        )
    )

    response = api_client.get("/attractor/runs")

    assert response.status_code == 200
    payload = response.json()
    assert len(payload["runs"]) == 1
    run_payload = payload["runs"][0]
    assert run_payload["spec_id"] == "spec-project-1700000000"
    assert run_payload["plan_id"] == "plan-project-1700000000"


def test_list_runs_filters_durable_history_by_project_item_9_6_01(
    api_client: TestClient,
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    server._write_run_meta(
        server.RunRecord(
            run_id="run-in-project-root",
            flow_name="Flow A",
            status="success",
            result="success",
            working_directory=str(tmp_path / "project-alpha"),
            model="test-model",
            started_at="2026-01-01T00:00:00Z",
            ended_at="2026-01-01T00:01:00Z",
            project_path=str(tmp_path / "project-alpha"),
        )
    )
    server._write_run_meta(
        server.RunRecord(
            run_id="run-in-project-child",
            flow_name="Flow B",
            status="success",
            result="success",
            working_directory=str(tmp_path / "project-alpha" / "nested"),
            model="test-model",
            started_at="2026-01-01T00:02:00Z",
            ended_at="2026-01-01T00:03:00Z",
            project_path="",
        )
    )
    server._write_run_meta(
        server.RunRecord(
            run_id="run-in-other-project",
            flow_name="Flow C",
            status="failed",
            result="failed",
            working_directory=str(tmp_path / "project-beta"),
            model="test-model",
            started_at="2026-01-01T00:04:00Z",
            ended_at="2026-01-01T00:05:00Z",
            project_path=str(tmp_path / "project-beta"),
        )
    )

    filtered_response = api_client.get(
        "/attractor/runs",
        params={"project_path": str(tmp_path / "project-alpha")},
    )
    assert filtered_response.status_code == 200
    filtered_payload = filtered_response.json()
    filtered_run_ids = {run["run_id"] for run in filtered_payload["runs"]}

    assert filtered_run_ids == {"run-in-project-root", "run-in-project-child"}


def test_list_runs_backfills_missing_timestamps_from_run_log_item_9_6_04(
    api_client: TestClient,
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    run_id = "run-with-partial-timestamps"
    server._write_run_meta(
        server.RunRecord(
            run_id=run_id,
            flow_name="Flow",
            status="success",
            result="success",
            working_directory=str(tmp_path / "project"),
            model="test-model",
            started_at="",
            ended_at=None,
            project_path=str(tmp_path / "project"),
        )
    )
    run_root = server._run_root(run_id)
    (run_root / "run.log").write_text(
        "\n".join(
            [
                "[2026-01-01 00:10:00 UTC] Starting run",
                "[2026-01-01 00:10:30 UTC] Pipeline success",
            ]
        )
        + "\n",
        encoding="utf-8",
    )

    response = api_client.get("/attractor/runs")

    assert response.status_code == 200
    payload = response.json()
    assert len(payload["runs"]) == 1
    run_payload = payload["runs"][0]
    assert run_payload["started_at"] == "2026-01-01T00:10:00Z"
    assert run_payload["ended_at"] == "2026-01-01T00:10:30Z"


def test_list_runs_reconstructs_timestamp_ordering_from_run_logs_item_9_6_04(
    api_client: TestClient,
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    older_id = "run-older"
    newer_id = "run-newer"

    for run_id, start_ts, end_ts in [
        (older_id, "2026-01-01 00:00:00", "2026-01-01 00:00:30"),
        (newer_id, "2026-01-01 00:01:00", "2026-01-01 00:01:30"),
    ]:
        server._write_run_meta(
            server.RunRecord(
                run_id=run_id,
                flow_name="Flow",
                status="success",
                result="success",
                working_directory=str(tmp_path / "project"),
                model="test-model",
                started_at="",
                ended_at=None,
                project_path=str(tmp_path / "project"),
            )
        )
        run_root = server._run_root(run_id)
        (run_root / "run.log").write_text(
            "\n".join(
                [
                    f"[{start_ts} UTC] Starting run",
                    f"[{end_ts} UTC] Pipeline success",
                ]
            )
            + "\n",
            encoding="utf-8",
        )

    response = api_client.get("/attractor/runs")
    assert response.status_code == 200
    payload = response.json()
    run_ids = [run["run_id"] for run in payload["runs"]]

    assert run_ids == [newer_id, older_id]


def test_execution_planning_approval_launches_real_pipeline_backed_run(
    api_client: TestClient,
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    project_path = str(tmp_path / "project")
    Path(project_path).mkdir(parents=True, exist_ok=True)
    flows_dir = server.get_settings().flows_dir
    flows_dir.mkdir(parents=True, exist_ok=True)
    (flows_dir / "plan-generation.dot").write_text(
        "\n".join(
            [
                "digraph plan_generation {",
                '  graph [goal=\"Generate a tracker-ready execution card JSON.\", label=\"Plan Generation\"];',
                '  start [label=\"Start\", shape=Mdiamond];',
                '  generate_execution_card [label=\"Generate Execution Card\", prompt=\"$goal\", shape=box];',
                '  done [label=\"Done\", shape=Msquare];',
                "  start -> generate_execution_card;",
                "  generate_execution_card -> done;",
                "}",
                "",
            ]
        ),
        encoding="utf-8",
    )

    class _Backend:
        def run(self, node_id, prompt, context, *, timeout=None):  # type: ignore[no-untyped-def]
            del node_id, prompt, context, timeout
            return json.dumps(
                {
                    "title": "Execution plan",
                    "summary": "Plan summary",
                    "objective": "Implement the approved spec edit.",
                    "work_items": [
                        {
                            "id": "work-1",
                            "title": "Update spec",
                            "description": "Apply the approved change.",
                            "acceptance_criteria": ["Spec updated"],
                            "depends_on": [],
                        }
                    ],
                }
            )

    monkeypatch.setattr(
        server,
        "_build_codergen_backend",
        lambda backend_name, working_dir, emit, model=None: _Backend(),
    )

    server.PROJECT_CHAT._write_state(
        project_chat.ConversationState(
            conversation_id="conversation-test",
            project_path=project_path,
            title="Workflow state test",
            created_at="2026-03-11T02:00:00Z",
            updated_at="2026-03-11T02:00:00Z",
            spec_edit_proposals=[
                project_chat.SpecEditProposal(
                    id="proposal-1",
                    created_at="2026-03-11T02:00:00Z",
                    summary="Summary",
                    changes=[project_chat_models.SpecEditProposalChange(path="specs/project.md", before="old", after="new")],
                    status="pending",
                )
            ],
        )
    )

    response = api_client.post(
        "/workspace/api/conversations/conversation-test/spec-edit-proposals/proposal-1/approve",
        json={
            "project_path": project_path,
            "flow_source": "plan-generation.dot",
        },
    )

    assert response.status_code == 200
    workflow_run_id = response.json()["execution_workflow"]["run_id"]

    terminal_status = "running"
    for _ in range(200):
        pipeline_response = api_client.get(f"/attractor/pipelines/{workflow_run_id}")
        assert pipeline_response.status_code == 200
        terminal_status = pipeline_response.json()["status"]
        if terminal_status != "running":
            break
        time.sleep(0.01)
    assert terminal_status == "success"

    snapshot = {}
    for _ in range(200):
        conversation_response = api_client.get(
            "/workspace/api/conversations/conversation-test",
            params={"project_path": project_path},
        )
        assert conversation_response.status_code == 200
        snapshot = conversation_response.json()
        if snapshot["execution_workflow"]["status"] == "idle" and snapshot["execution_cards"]:
            break
        time.sleep(0.01)

    assert snapshot["execution_workflow"]["status"] == "idle"
    assert snapshot["execution_cards"][0]["source_workflow_run_id"] == workflow_run_id

    record = server._read_run_meta(server._run_meta_path(workflow_run_id))
    assert record is not None
    assert record.flow_name == "plan-generation.dot"
    assert record.project_path == project_chat._normalize_project_path(project_path)
    assert record.status == "success"
    assert record.plan_id == snapshot["execution_cards"][0]["id"]

    checkpoint_response = api_client.get(f"/attractor/pipelines/{workflow_run_id}/checkpoint")
    assert checkpoint_response.status_code == 200
    context_response = api_client.get(f"/attractor/pipelines/{workflow_run_id}/context")
    assert context_response.status_code == 200
    artifacts_response = api_client.get(f"/attractor/pipelines/{workflow_run_id}/artifacts")
    assert artifacts_response.status_code == 200
    artifact_paths = [entry["path"] for entry in artifacts_response.json()["artifacts"]]
    assert "artifacts/graphviz/pipeline.dot" in artifact_paths
    assert f"logs/{server.EXECUTION_PLANNING_STAGE_ID}/response.md" in artifact_paths


def test_execution_planning_approval_uses_project_trigger_binding_when_present(
    api_client: TestClient,
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    project_path = str(tmp_path / "project")
    Path(project_path).mkdir(parents=True, exist_ok=True)
    flows_dir = server.get_settings().flows_dir
    flows_dir.mkdir(parents=True, exist_ok=True)
    (flows_dir / "custom-plan.dot").write_text(
        "\n".join(
            [
                "digraph custom_plan {",
                '  graph [goal=\"Generate a tracker-ready execution card JSON.\"];',
                '  start [shape=\"Mdiamond\"];',
                '  generate_execution_card [prompt=\"$goal\", shape=\"box\"];',
                '  done [shape=\"Msquare\"];',
                "  start -> generate_execution_card;",
                "  generate_execution_card -> done;",
                "}",
                "",
            ]
        ),
        encoding="utf-8",
    )

    class _Backend:
        def run(self, node_id, prompt, context, *, timeout=None):  # type: ignore[no-untyped-def]
            del node_id, prompt, context, timeout
            return json.dumps(
                {
                    "title": "Execution plan",
                    "summary": "Plan summary",
                    "objective": "Implement the approved spec edit.",
                    "work_items": [],
                }
            )

    monkeypatch.setattr(
        server,
        "_build_codergen_backend",
        lambda backend_name, working_dir, emit, model=None: _Backend(),
    )

    _write_flow("custom-plan.dot")
    api_client.post("/workspace/api/projects/register", json={"project_path": project_path})
    binding_response = api_client.put(
        "/workspace/api/projects/flow-bindings/spec_edit_approved",
        json={
            "project_path": project_path,
            "flow_name": "custom-plan.dot",
        },
    )
    assert binding_response.status_code == 200

    server.PROJECT_CHAT._write_state(
        project_chat.ConversationState(
            conversation_id="conversation-test",
            project_path=project_path,
            title="Workflow state test",
            created_at="2026-03-11T02:00:00Z",
            updated_at="2026-03-11T02:00:00Z",
            spec_edit_proposals=[
                project_chat.SpecEditProposal(
                    id="proposal-1",
                    created_at="2026-03-11T02:00:00Z",
                    summary="Summary",
                    changes=[project_chat_models.SpecEditProposalChange(path="specs/project.md", before="old", after="new")],
                    status="pending",
                )
            ],
        )
    )

    response = api_client.post(
        "/workspace/api/conversations/conversation-test/spec-edit-proposals/proposal-1/approve",
        json={
            "project_path": project_path,
        },
    )

    assert response.status_code == 200
    workflow_run_id = response.json()["execution_workflow"]["run_id"]
    record = server._read_run_meta(server._run_meta_path(workflow_run_id))
    assert record is not None
    assert record.flow_name == "custom-plan.dot"


def test_approved_execution_card_launches_selected_flow_run(
    api_client: TestClient,
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    project_path = str(tmp_path / "project")
    Path(project_path).mkdir(parents=True, exist_ok=True)
    flows_dir = server.get_settings().flows_dir
    flows_dir.mkdir(parents=True, exist_ok=True)
    (flows_dir / "implement-spec.dot").write_text(
        "\n".join(
            [
                "digraph implement_spec {",
                '  graph [goal="Implement the approved execution card."];',
                '  start [shape="Mdiamond"];',
                '  implement [prompt="Implement card", shape="box"];',
                '  done [shape="Msquare"];',
                "  start -> implement;",
                "  implement -> done;",
                "}",
                "",
            ]
        ),
        encoding="utf-8",
    )

    class _Backend:
        def run(self, node_id, prompt, context, *, timeout=None):  # type: ignore[no-untyped-def]
            del node_id, prompt, context, timeout
            return "implemented"

    monkeypatch.setattr(
        server,
        "_build_codergen_backend",
        lambda backend_name, working_dir, emit, model=None: _Backend(),
    )

    server.PROJECT_CHAT._write_state(
        project_chat.ConversationState(
            conversation_id="conversation-test",
            project_path=project_path,
            title="Execution approval test",
            created_at="2026-03-11T02:00:00Z",
            updated_at="2026-03-11T02:00:00Z",
            execution_cards=[
                project_chat.ExecutionCard(
                    id="execution-card-1",
                    title="Execution plan",
                    summary="Plan summary",
                    objective="Implement the approved spec edit.",
                    source_spec_edit_id="spec-edit-1",
                    source_workflow_run_id="workflow-plan-1",
                    created_at="2026-03-11T02:00:00Z",
                    updated_at="2026-03-11T02:00:00Z",
                    status="draft",
                    flow_source="plan-generation.dot",
                    work_items=[],
                    review_feedback=[],
                )
            ],
        )
    )

    response = api_client.post(
        "/workspace/api/conversations/conversation-test/execution-cards/execution-card-1/review",
        json={
            "project_path": project_path,
            "disposition": "approved",
            "message": "Approved for dispatch.",
            "flow_source": "implement-spec.dot",
        },
    )

    assert response.status_code == 200

    runs_response = api_client.get("/attractor/runs", params={"project_path": project_path})
    assert runs_response.status_code == 200
    runs_payload = runs_response.json()["runs"]
    matching_runs = [run for run in runs_payload if run["plan_id"] == "execution-card-1"]
    assert len(matching_runs) == 1
    launched_run = matching_runs[0]
    assert launched_run["flow_name"] == "implement-spec.dot"
    assert launched_run["spec_id"] == "spec-edit-1"

    terminal_status = "running"
    for _ in range(200):
        pipeline_response = api_client.get(f"/attractor/pipelines/{launched_run['run_id']}")
        assert pipeline_response.status_code == 200
        terminal_status = pipeline_response.json()["status"]
        if terminal_status != "running":
            break
        time.sleep(0.01)
    assert terminal_status == "success"

    snapshot_response = api_client.get(
        "/workspace/api/conversations/conversation-test",
        params={"project_path": project_path},
    )
    assert snapshot_response.status_code == 200
    snapshot = snapshot_response.json()
    assert snapshot["execution_cards"][0]["status"] == "approved"
    assert snapshot["event_log"][-1]["message"] == (
        f"Dispatched execution card execution-card-1 as run {launched_run['run_id']} using implement-spec.dot."
    )


def test_approved_execution_card_defaults_legacy_planning_flow_to_dispatch_flow(
    api_client: TestClient,
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    project_path = str(tmp_path / "project")
    Path(project_path).mkdir(parents=True, exist_ok=True)
    flows_dir = server.get_settings().flows_dir
    flows_dir.mkdir(parents=True, exist_ok=True)
    (flows_dir / "implement-spec.dot").write_text(
        "\n".join(
            [
                "digraph implement_spec {",
                '  graph [goal="Implement the approved execution card."];',
                '  start [shape="Mdiamond"];',
                '  implement [prompt="Implement card", shape="box"];',
                '  done [shape="Msquare"];',
                "  start -> implement;",
                "  implement -> done;",
                "}",
                "",
            ]
        ),
        encoding="utf-8",
    )

    class _Backend:
        def run(self, node_id, prompt, context, *, timeout=None):  # type: ignore[no-untyped-def]
            del node_id, prompt, context, timeout
            return "implemented"

    monkeypatch.setattr(
        server,
        "_build_codergen_backend",
        lambda backend_name, working_dir, emit, model=None: _Backend(),
    )

    server.PROJECT_CHAT._write_state(
        project_chat.ConversationState(
            conversation_id="conversation-test",
            project_path=project_path,
            title="Execution approval test",
            created_at="2026-03-11T02:00:00Z",
            updated_at="2026-03-11T02:00:00Z",
            execution_cards=[
                project_chat.ExecutionCard(
                    id="execution-card-1",
                    title="Execution plan",
                    summary="Plan summary",
                    objective="Implement the approved spec edit.",
                    source_spec_edit_id="spec-edit-1",
                    source_workflow_run_id="workflow-plan-1",
                    created_at="2026-03-11T02:00:00Z",
                    updated_at="2026-03-11T02:00:00Z",
                    status="draft",
                    flow_source="plan-generation.dot",
                    work_items=[],
                    review_feedback=[],
                )
            ],
        )
    )

    response = api_client.post(
        "/workspace/api/conversations/conversation-test/execution-cards/execution-card-1/review",
        json={
            "project_path": project_path,
            "disposition": "approved",
            "message": "Approved for dispatch.",
        },
    )

    assert response.status_code == 200

    runs_response = api_client.get("/attractor/runs", params={"project_path": project_path})
    assert runs_response.status_code == 200
    runs_payload = runs_response.json()["runs"]
    matching_runs = [run for run in runs_payload if run["plan_id"] == "execution-card-1"]
    assert len(matching_runs) == 1
    launched_run = matching_runs[0]
    assert launched_run["flow_name"] == "implement-spec.dot"


def test_approved_execution_card_uses_project_trigger_binding_when_present(
    api_client: TestClient,
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    project_path = str(tmp_path / "project")
    Path(project_path).mkdir(parents=True, exist_ok=True)
    flows_dir = server.get_settings().flows_dir
    flows_dir.mkdir(parents=True, exist_ok=True)
    (flows_dir / "custom-implement.dot").write_text(
        "\n".join(
            [
                "digraph custom_implement {",
                '  graph [goal=\"Implement the approved execution card.\"];',
                '  start [shape=\"Mdiamond\"];',
                '  implement [prompt=\"Implement card\", shape=\"box\"];',
                '  done [shape=\"Msquare\"];',
                "  start -> implement;",
                "  implement -> done;",
                "}",
                "",
            ]
        ),
        encoding="utf-8",
    )

    class _Backend:
        def run(self, node_id, prompt, context, *, timeout=None):  # type: ignore[no-untyped-def]
            del node_id, prompt, context, timeout
            return "implemented"

    monkeypatch.setattr(
        server,
        "_build_codergen_backend",
        lambda backend_name, working_dir, emit, model=None: _Backend(),
    )

    _write_flow("custom-implement.dot")
    api_client.post("/workspace/api/projects/register", json={"project_path": project_path})
    binding_response = api_client.put(
        "/workspace/api/projects/flow-bindings/execution_card_approved",
        json={
            "project_path": project_path,
            "flow_name": "custom-implement.dot",
        },
    )
    assert binding_response.status_code == 200

    server.PROJECT_CHAT._write_state(
        project_chat.ConversationState(
            conversation_id="conversation-test",
            project_path=project_path,
            title="Execution approval test",
            created_at="2026-03-11T02:00:00Z",
            updated_at="2026-03-11T02:00:00Z",
            execution_cards=[
                project_chat.ExecutionCard(
                    id="execution-card-1",
                    title="Execution plan",
                    summary="Plan summary",
                    objective="Implement the approved spec edit.",
                    source_spec_edit_id="spec-edit-1",
                    source_workflow_run_id="workflow-plan-1",
                    created_at="2026-03-11T02:00:00Z",
                    updated_at="2026-03-11T02:00:00Z",
                    status="draft",
                    flow_source="plan-generation.dot",
                    work_items=[],
                    review_feedback=[],
                )
            ],
        )
    )

    response = api_client.post(
        "/workspace/api/conversations/conversation-test/execution-cards/execution-card-1/review",
        json={
            "project_path": project_path,
            "disposition": "approved",
            "message": "Approved for dispatch.",
        },
    )

    assert response.status_code == 200

    runs_response = api_client.get("/attractor/runs", params={"project_path": project_path})
    assert runs_response.status_code == 200
    matching_runs = [run for run in runs_response.json()["runs"] if run["plan_id"] == "execution-card-1"]
    assert len(matching_runs) == 1
    assert matching_runs[0]["flow_name"] == "custom-implement.dot"
