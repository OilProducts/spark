from __future__ import annotations

from pathlib import Path

import pytest
from fastapi.testclient import TestClient

import attractor.api.server as server
from attractor.engine import Checkpoint, Outcome, OutcomeStatus, save_checkpoint
from tests.api._support import (
    close_task_immediately as _close_task_immediately,
    start_pipeline as _start_pipeline,
)


def test_get_pipeline_context_returns_404_for_unknown_pipeline(
    attractor_api_client: TestClient,
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    server.configure_runtime_paths(runs_dir=tmp_path / "runs")

    response = attractor_api_client.get("/pipelines/missing-run/context")

    assert response.status_code == 404
    assert response.json()["detail"] == "Unknown pipeline"


def test_get_pipeline_context_returns_context_for_known_pipeline(
    attractor_api_client: TestClient,
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    runs_root = tmp_path / "runs"
    server.configure_runtime_paths(runs_dir=runs_root)
    monkeypatch.setattr(server.asyncio, "create_task", _close_task_immediately)

    start_payload = _start_pipeline(attractor_api_client, tmp_path / "work")
    run_id = str(start_payload["pipeline_id"])
    run_root = server._run_root(run_id)

    checkpoint = Checkpoint(
        timestamp="2026-01-01T00:00:00Z",
        current_node="implement",
        completed_nodes=["start", "plan"],
        context={
            "graph.goal": "Ship feature",
            "outcome": "success",
            "context.plan.ready": True,
        },
        retry_counts={"implement": 1},
        logs=["started", "implemented"],
    )
    save_checkpoint(run_root / "state.json", checkpoint)

    response = attractor_api_client.get(f"/pipelines/{run_id}/context")

    assert response.status_code == 200
    payload = response.json()
    assert payload == {
        "pipeline_id": run_id,
        "context": checkpoint.context,
    }


def test_start_pipeline_seeds_launch_context_into_initial_checkpoint(
    attractor_api_client: TestClient,
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    runs_root = tmp_path / "runs"
    server.configure_runtime_paths(runs_dir=runs_root)
    monkeypatch.setattr(server.asyncio, "create_task", _close_task_immediately)

    launch_context = {
        "context.request.summary": "Implement the approved scope.",
        "context.request.acceptance_criteria": ["Tests are updated."],
    }

    start_payload = _start_pipeline(
        attractor_api_client,
        tmp_path / "work",
        launch_context=launch_context,
    )
    run_id = str(start_payload["pipeline_id"])

    response = attractor_api_client.get(f"/pipelines/{run_id}/context")

    assert response.status_code == 200
    payload = response.json()
    assert payload["pipeline_id"] == run_id
    assert payload["context"]["context.request.summary"] == "Implement the approved scope."
    assert payload["context"]["context.request.acceptance_criteria"] == ["Tests are updated."]
    assert payload["context"]["internal.run_workdir"] == str((tmp_path / "work").resolve())


def test_start_pipeline_persists_and_seeds_execution_container_profile(
    attractor_api_client: TestClient,
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    class FakeContainerRunner:
        def __init__(self, *args, **kwargs) -> None:
            self.kwargs = kwargs
            self.transport = object()

        def __call__(self, node_id: str, prompt: str, context, *, emit_event=None):
            return Outcome(status=OutcomeStatus.SUCCESS)

        def set_logs_root(self, logs_root):
            self.logs_root = logs_root

        def set_control(self, control):
            self.control = control

        def close(self):
            self.closed = True

    runs_root = tmp_path / "runs"
    server.configure_runtime_paths(runs_dir=runs_root)
    monkeypatch.setattr(server.asyncio, "create_task", _close_task_immediately)
    monkeypatch.setattr(server, "ContainerizedHandlerRunner", FakeContainerRunner)

    response = attractor_api_client.post(
        "/pipelines",
        json={
            "flow_content": """
digraph G {
    start [shape=Mdiamond]
    work [shape=tool, tool.command="true"]
    done [shape=Msquare]
    start -> work -> done
}
""",
            "working_directory": str(tmp_path / "work"),
            "execution_container_image": "spark-exec:latest",
        },
    )

    assert response.status_code == 200
    start_payload = response.json()
    run_id = str(start_payload["run_id"])
    assert start_payload["execution_mode"] == "container"
    assert start_payload["execution_container_image"] == "spark-exec:latest"

    checkpoint = server.load_checkpoint(server._run_root(run_id) / "state.json")
    assert checkpoint is not None
    assert checkpoint.context["_attractor.runtime.execution_mode"] == "container"
    assert checkpoint.context["_attractor.runtime.execution_container_image"] == "spark-exec:latest"

    record = server._read_run_meta(server._run_root(run_id) / "run.json")
    assert record is not None
    assert record.execution_mode == "container"
    assert record.execution_container_image == "spark-exec:latest"


def test_start_pipeline_rejects_launch_context_outside_context_namespace(
    attractor_api_client: TestClient,
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    runs_root = tmp_path / "runs"
    server.configure_runtime_paths(runs_dir=runs_root)
    monkeypatch.setattr(server.asyncio, "create_task", _close_task_immediately)

    response = attractor_api_client.post(
        "/pipelines",
        json={
            "flow_content": """
digraph G {
    start [shape=Mdiamond]
    done [shape=Msquare]
    start -> done
}
""",
            "working_directory": str(tmp_path / "work"),
            "backend": "codex-app-server",
            "launch_context": {
                "graph.goal": "not allowed",
            },
        },
    )

    assert response.status_code == 200
    assert response.json() == {
        "status": "validation_error",
        "error": (
            "Attractor pipeline start launch_context key must use the context.* namespace: graph.goal"
        ),
    }
