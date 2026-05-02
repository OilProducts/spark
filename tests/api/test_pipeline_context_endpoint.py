from __future__ import annotations

from datetime import datetime, timezone
from pathlib import Path
import socket
import sys
import threading
import time
from typing import Any

import pytest
from fastapi.testclient import TestClient
import uvicorn

import attractor.api.server as server
import attractor.execution.metadata as execution_metadata
from attractor.engine import Checkpoint, Outcome, OutcomeStatus, save_checkpoint
from attractor.execution import (
    EXECUTION_PROFILE_ID_CONTEXT_KEY,
    EXECUTION_PROFILE_SELECTION_SOURCE_CONTEXT_KEY,
    ExecutionProtocolError,
    WorkerAPIError,
    WorkerEvent,
    WorkerHealthResponse,
    WorkerInfoResponse,
    WorkerCallbackResponse,
    WorkerCancelResponse,
    WorkerCleanupResponse,
    WorkerNodeAcceptedResponse,
    WorkerNodeRequest,
    WorkerRunAdmissionResponse,
)
from attractor.execution.worker_app import create_worker_app
from attractor.execution.worker_runtime import LocalProcessWorkerRuntime
from tests.api._support import (
    close_task_immediately as _close_task_immediately,
    start_pipeline as _start_pipeline,
    wait_for_pipeline_completion,
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
    config_dir = server.get_runtime_paths().config_dir
    config_dir.mkdir(parents=True, exist_ok=True)
    (config_dir / "execution-profiles.toml").write_text(
        """
        [profiles.local-dev]
        mode = "local_container"
        label = "Local Dev"
        image = "spark-exec:latest"
        """,
        encoding="utf-8",
    )
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
            "execution_profile_id": "local-dev",
        },
    )

    assert response.status_code == 200
    start_payload = response.json()
    run_id = str(start_payload["run_id"])
    assert start_payload["execution_mode"] == "local_container"
    assert start_payload["execution_profile_id"] == "local-dev"
    assert start_payload["execution_container_image"] == "spark-exec:latest"

    checkpoint = server.load_checkpoint(server._run_root(run_id) / "state.json")
    assert checkpoint is not None
    assert checkpoint.context["_attractor.runtime.execution_mode"] == "local_container"
    assert checkpoint.context["_attractor.runtime.execution_profile_id"] == "local-dev"
    assert checkpoint.context["_attractor.runtime.execution_container_image"] == "spark-exec:latest"

    record = server._read_run_meta(server._run_root(run_id) / "run.json")
    assert record is not None
    assert record.execution_mode == "local_container"
    assert record.execution_profile_id == "local-dev"
    assert record.execution_container_image == "spark-exec:latest"

    (config_dir / "execution-profiles.toml").write_text(
        """
        [profiles.local-dev]
        mode = "local_container"
        label = "Local Dev Mutated"
        image = "spark-exec:mutated"
        """,
        encoding="utf-8",
    )

    status_response = attractor_api_client.get(f"/pipelines/{run_id}")
    assert status_response.status_code == 200
    assert status_response.json()["execution_container_image"] == "spark-exec:latest"

    runs_response = attractor_api_client.get("/runs")
    assert runs_response.status_code == 200
    run_payload = next(item for item in runs_response.json()["runs"] if item["run_id"] == run_id)
    assert run_payload["execution_mode"] == "local_container"
    assert run_payload["execution_container_image"] == "spark-exec:latest"

    context_response = attractor_api_client.get(f"/pipelines/{run_id}/context")
    assert context_response.status_code == 200
    context = context_response.json()["context"]
    assert context["execution_container_image"] == "spark-exec:latest"
    assert context["_attractor.runtime.execution_container_image"] == "spark-exec:latest"


def test_pipeline_cleanup_failure_is_visible_without_rewriting_terminal_outcome(
    attractor_api_client: TestClient,
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    class CleanupFailingContainerRunner:
        def __init__(self, *args, **kwargs) -> None:
            self.transport = object()

        def __call__(self, node_id: str, prompt: str, context, *, emit_event=None):
            return Outcome(status=OutcomeStatus.SUCCESS)

        def set_logs_root(self, logs_root):
            self.logs_root = logs_root

        def set_control(self, control):
            self.control = control

        def close(self):
            raise RuntimeError("container cleanup failed")

    server.configure_runtime_paths(runs_dir=tmp_path / "runs")
    config_dir = server.get_runtime_paths().config_dir
    config_dir.mkdir(parents=True, exist_ok=True)
    (config_dir / "execution-profiles.toml").write_text(
        """
        [profiles.local-dev]
        mode = "local_container"
        label = "Local Dev"
        image = "spark-exec:latest"
        """,
        encoding="utf-8",
    )
    monkeypatch.setattr(server, "ContainerizedHandlerRunner", CleanupFailingContainerRunner)

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
            "execution_profile_id": "local-dev",
        },
    )

    assert response.status_code == 200
    run_id = str(response.json()["run_id"])
    completion = wait_for_pipeline_completion(attractor_api_client, run_id)

    assert completion["status"] == "completed"
    assert completion["outcome"] == "success"
    assert completion["last_error"] in {"", None}
    assert completion["cleanup_error"] == "Run cleanup failed: container cleanup failed"

    record = server._read_run_meta(server._run_root(run_id) / "run.json")
    assert record is not None
    assert record.status == "completed"
    assert record.outcome == "success"
    assert record.cleanup_error == "Run cleanup failed: container cleanup failed"

    log_response = attractor_api_client.get(f"/pipelines/{run_id}/artifacts/run.log")
    assert log_response.status_code == 200
    assert "Run cleanup failed: container cleanup failed" in log_response.text


def test_start_pipeline_rejects_missing_execution_profile_id_before_execution(
    attractor_api_client: TestClient,
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    class UnexpectedContainerRunner:
        def __init__(self, *args, **kwargs) -> None:  # pragma: no cover - assertion path
            raise AssertionError("execution should not start for missing profile")

    server.configure_runtime_paths(runs_dir=tmp_path / "runs")
    config_dir = server.get_runtime_paths().config_dir
    config_dir.mkdir(parents=True, exist_ok=True)
    (config_dir / "execution-profiles.toml").write_text(
        """
        [profiles.native-dev]
        mode = "native"
        label = "Native Dev"
        """,
        encoding="utf-8",
    )
    monkeypatch.setattr(server.asyncio, "create_task", _close_task_immediately)
    monkeypatch.setattr(server, "ContainerizedHandlerRunner", UnexpectedContainerRunner)

    response = attractor_api_client.post(
        "/pipelines",
        json={
            "flow_content": "digraph G { start [shape=Mdiamond] done [shape=Msquare] start -> done }",
            "working_directory": str(tmp_path / "work"),
            "execution_profile_id": "missing",
        },
    )

    assert response.status_code == 200
    payload = response.json()
    assert payload["status"] == "validation_error"
    assert "missing" in payload["error"]


def test_start_pipeline_rejects_remote_project_outside_control_root_before_execution(
    attractor_api_client: TestClient,
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    def unexpected_backend(*args, **kwargs):  # pragma: no cover - assertion path
        raise AssertionError("execution should not start for an unmappable remote project")

    server.configure_runtime_paths(runs_dir=tmp_path / "runs")
    config_dir = server.get_runtime_paths().config_dir
    config_dir.mkdir(parents=True, exist_ok=True)
    control_root = tmp_path / "control"
    (config_dir / "execution-profiles.toml").write_text(
        f"""
        [workers.worker-a]
        label = "Worker A"
        enabled = true
        base_url = "https://worker.example"
        auth_token_env = "SPARK_WORKER_TOKEN"

        [profiles.remote-fast]
        mode = "remote_worker"
        label = "Remote Fast"
        worker = "worker-a"
        image = "spark-worker:latest"
        control_project_root = "{control_root}"
        worker_project_root = "/srv/projects"
        worker_runtime_root = "/srv/runtime"
        capabilities = ["shell"]
        """,
        encoding="utf-8",
    )
    monkeypatch.setattr(server, "_build_codergen_backend", unexpected_backend)
    monkeypatch.setattr(server.asyncio, "create_task", _close_task_immediately)

    response = attractor_api_client.post(
        "/pipelines",
        json={
            "flow_content": "digraph G { start [shape=Mdiamond] done [shape=Msquare] start -> done }",
            "working_directory": str(tmp_path / "control-other" / "project"),
            "execution_profile_id": "remote-fast",
        },
    )

    assert response.status_code == 200
    payload = response.json()
    assert payload["status"] == "validation_error"
    assert "outside remote control_project_root" in payload["error"]


def test_start_pipeline_admits_valid_remote_worker_profile_without_native_execution(
    attractor_api_client: TestClient,
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    def unexpected_backend(*args, **kwargs):  # pragma: no cover - assertion path
        raise AssertionError("remote_worker profile should not run through the native backend")

    server.configure_runtime_paths(runs_dir=tmp_path / "runs")
    config_dir = server.get_runtime_paths().config_dir
    config_dir.mkdir(parents=True, exist_ok=True)
    control_root = tmp_path / "control"
    (config_dir / "execution-profiles.toml").write_text(
        f"""
        [workers.worker-a]
        label = "Worker A"
        enabled = true
        base_url = "https://worker.example"
        auth_token_env = "SPARK_WORKER_TOKEN"

        [profiles.remote-fast]
        mode = "remote_worker"
        label = "Remote Fast"
        worker = "worker-a"
        image = "spark-worker:latest"
        control_project_root = "{control_root}"
        worker_project_root = "/srv/projects"
        worker_runtime_root = "/srv/runtime"
        capabilities = ["shell"]
        """,
        encoding="utf-8",
    )
    admission_requests = []

    class FakeRemoteClient:
        def __init__(self, worker):
            self.worker = worker

        def __enter__(self):
            return self

        def __exit__(self, *_exc_info):
            return None

        def health(self):
            return WorkerHealthResponse(
                worker_id="worker-a",
                worker_version="1.2.3",
                protocol_version="v1",
                status="ok",
                capabilities={"shell": True},
            )

        def worker_info(self):
            return WorkerInfoResponse(
                worker_id="worker-a",
                worker_version="1.2.3",
                protocol_version="v1",
                status="ok",
                capabilities={"shell": True},
            )

        def admit_run(self, request):
            admission_requests.append(request)
            return WorkerRunAdmissionResponse(
                run_id=request.run_id,
                worker_id="worker-a",
                status="preparing",
                event_url=f"/v1/runs/{request.run_id}/events",
                last_sequence=0,
                accepted=True,
            )

    monkeypatch.setattr(server, "_build_codergen_backend", unexpected_backend)
    monkeypatch.setattr(execution_metadata, "RemoteWorkerClient", FakeRemoteClient)
    monkeypatch.setattr(server.asyncio, "create_task", _close_task_immediately)

    response = attractor_api_client.post(
        "/pipelines",
        json={
            "flow_content": "digraph G { start [shape=Mdiamond] done [shape=Msquare] start -> done }",
            "working_directory": str(control_root / "project"),
            "execution_profile_id": "remote-fast",
        },
    )

    assert response.status_code == 200
    payload = response.json()
    assert payload["status"] == "started"
    assert payload["execution_mode"] == "remote_worker"
    assert payload["execution_profile_id"] == "remote-fast"
    assert payload["execution_container_image"] == "spark-worker:latest"
    assert payload["execution_mapped_project_path"] == "/srv/projects/project"
    assert payload["execution_worker_runtime_root"] == "/srv/runtime"
    assert payload["execution_worker_id"] == "worker-a"
    assert payload["execution_worker_label"] == "Worker A"
    assert payload["execution_worker_base_url"] == "https://worker.example"
    assert payload["execution_worker_version"] == "1.2.3"
    assert payload["execution_worker_capabilities"] == {"shell": True}
    assert payload["execution_profile_capabilities"] == ["shell"]
    assert len(admission_requests) == 1
    assert admission_requests[0].mapped_project_path == "/srv/projects/project"
    assert admission_requests[0].worker_runtime_root == "/srv/runtime"

    run_id = str(payload["run_id"])
    record = server._read_run_meta(server._run_root(run_id) / "run.json")
    assert record is not None
    record_payload = record.to_dict()
    for key, value in {
        "execution_mode": "remote_worker",
        "execution_profile_id": "remote-fast",
        "execution_worker_id": "worker-a",
        "execution_worker_label": "Worker A",
        "execution_worker_base_url": "https://worker.example",
        "execution_container_image": "spark-worker:latest",
        "execution_mapped_project_path": "/srv/projects/project",
        "execution_worker_runtime_root": "/srv/runtime",
        "execution_worker_version": "1.2.3",
        "execution_worker_capabilities": {"shell": True},
        "execution_profile_capabilities": ["shell"],
    }.items():
        assert record_payload[key] == value

    runs_response = attractor_api_client.get("/runs")
    assert runs_response.status_code == 200
    runs_payload = runs_response.json()["runs"]
    run_payload = next(item for item in runs_payload if item["run_id"] == run_id)
    assert run_payload["execution_worker_label"] == "Worker A"
    assert run_payload["execution_worker_base_url"] == "https://worker.example"
    assert run_payload["execution_worker_capabilities"] == {"shell": True}

    context_response = attractor_api_client.get(f"/pipelines/{run_id}/context")
    assert context_response.status_code == 200
    context = context_response.json()["context"]
    assert context["execution_worker_id"] == "worker-a"
    assert context["execution_worker_label"] == "Worker A"
    assert context["execution_worker_base_url"] == "https://worker.example"
    assert context["execution_mapped_project_path"] == "/srv/projects/project"
    assert context["execution_worker_runtime_root"] == "/srv/runtime"
    assert context["execution_worker_version"] == "1.2.3"
    assert context["execution_worker_capabilities"] == {"shell": True}
    assert context["execution_profile_capabilities"] == ["shell"]
    assert context["_attractor.runtime.execution_worker_id"] == "worker-a"

    (config_dir / "execution-profiles.toml").write_text(
        f"""
        [workers.worker-a]
        label = "Worker A Mutated"
        enabled = true
        base_url = "https://mutated-worker.example"
        auth_token_env = "SPARK_WORKER_TOKEN"

        [profiles.remote-fast]
        mode = "remote_worker"
        label = "Remote Fast Mutated"
        worker = "worker-a"
        image = "spark-worker:mutated"
        control_project_root = "{control_root}"
        worker_project_root = "/srv/mutated-projects"
        worker_runtime_root = "/srv/mutated-runtime"
        capabilities = ["shell", "git"]
        """,
        encoding="utf-8",
    )

    status_response = attractor_api_client.get(f"/pipelines/{run_id}")
    assert status_response.status_code == 200
    status_payload = status_response.json()
    for key, value in {
        "execution_mode": "remote_worker",
        "execution_profile_id": "remote-fast",
        "execution_worker_id": "worker-a",
        "execution_worker_label": "Worker A",
        "execution_worker_base_url": "https://worker.example",
        "execution_container_image": "spark-worker:latest",
        "execution_mapped_project_path": "/srv/projects/project",
        "execution_worker_runtime_root": "/srv/runtime",
        "execution_worker_version": "1.2.3",
        "execution_worker_capabilities": {"shell": True},
        "execution_profile_capabilities": ["shell"],
    }.items():
        assert status_payload[key] == value

    runs_response = attractor_api_client.get("/runs")
    assert runs_response.status_code == 200
    run_payload = next(item for item in runs_response.json()["runs"] if item["run_id"] == run_id)
    for key, value in {
        "execution_worker_label": "Worker A",
        "execution_worker_base_url": "https://worker.example",
        "execution_container_image": "spark-worker:latest",
        "execution_mapped_project_path": "/srv/projects/project",
        "execution_worker_runtime_root": "/srv/runtime",
        "execution_worker_version": "1.2.3",
        "execution_worker_capabilities": {"shell": True},
        "execution_profile_capabilities": ["shell"],
    }.items():
        assert run_payload[key] == value

    context_response = attractor_api_client.get(f"/pipelines/{run_id}/context")
    assert context_response.status_code == 200
    context = context_response.json()["context"]
    for key, value in {
        "execution_worker_label": "Worker A",
        "execution_worker_base_url": "https://worker.example",
        "execution_container_image": "spark-worker:latest",
        "execution_mapped_project_path": "/srv/projects/project",
        "execution_worker_runtime_root": "/srv/runtime",
        "execution_worker_version": "1.2.3",
        "execution_worker_capabilities": {"shell": True},
        "execution_profile_capabilities": ["shell"],
    }.items():
        assert context[key] == value
        assert context[f"_attractor.runtime.{key}"] == value

    stored_record = server._read_run_meta(server._run_root(run_id) / "run.json")
    assert stored_record is not None
    assert stored_record.execution_worker_label == "Worker A"
    assert stored_record.execution_worker_base_url == "https://worker.example"
    assert stored_record.execution_container_image == "spark-worker:latest"
    assert stored_record.execution_mapped_project_path == "/srv/projects/project"
    assert stored_record.execution_worker_runtime_root == "/srv/runtime"


def test_remote_worker_node_result_applies_to_canonical_run_state(
    attractor_api_client: TestClient,
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    def unexpected_backend(*args, **kwargs):  # pragma: no cover - assertion path
        raise AssertionError("remote_worker profile should not run through the native backend")

    server.configure_runtime_paths(runs_dir=tmp_path / "runs")
    _write_remote_profile_config(tmp_path)
    controller = _RemoteController(
        outcome={
            "status": "success",
            "context_updates": {"context.remote.answer": 42},
            "notes": "remote ok",
        }
    )

    monkeypatch.setattr(server, "_build_codergen_backend", unexpected_backend)
    monkeypatch.setattr(execution_metadata, "RemoteWorkerClient", controller.factory)

    response = attractor_api_client.post(
        "/pipelines",
        json={
            "flow_content": (
                "digraph G { "
                'start [shape=Mdiamond, label="Start", spark.writes_context="[\\"context.remote.answer\\"]"] '
                "done [shape=Msquare] start -> done }"
            ),
            "working_directory": str(tmp_path / "control" / "project"),
            "execution_profile_id": "remote-fast",
        },
    )

    assert response.status_code == 200
    payload = response.json()
    assert payload["status"] == "started"
    run_id = str(payload["run_id"])
    completion = wait_for_pipeline_completion(attractor_api_client, run_id)

    assert completion["status"] == "completed", completion
    assert completion["outcome"] == "success"
    assert len(controller.node_requests) == 1
    node_request = controller.node_requests[0]
    assert node_request.node_execution_id == f"{run_id}:start:1"
    assert node_request.payload["node_metadata"]["node_id"] == "start"
    assert node_request.payload["node_metadata"]["attrs"]["shape"] == "Mdiamond"
    assert node_request.payload["runtime_references"]["run_id"] == run_id
    assert node_request.payload["runtime_references"]["execution_profile_id"] == "remote-fast"
    assert node_request.payload["options"] == {"max_attempts": 0}

    checkpoint = server.load_checkpoint(server._run_root(run_id) / "state.json")
    assert checkpoint is not None
    assert checkpoint.completed_nodes == ["start"]
    assert checkpoint.context["context.remote.answer"] == 42

    record = server._read_run_meta(server._run_root(run_id) / "run.json")
    assert record is not None
    assert record.status == "completed"
    assert record.execution_worker_id == "worker-a"

    persisted_events = server._read_persisted_run_events(run_id)
    event_types = [event["type"] for event in persisted_events]
    assert "remote_worker_event" in event_types
    remote_events = [event for event in persisted_events if event["type"] == "remote_worker_event"]
    assert [event["event"] for event in remote_events] == ["run_ready", "node_started", "node_result"]
    result_payload = remote_events[-1]["payload"]
    assert result_payload["outcome"]["status"] == "success"
    assert result_payload["outcome"]["context_updates"] == {"context.remote.answer": 42}
    assert result_payload["context"] == {"context.remote.answer": 42}
    assert result_payload["runtime_metadata"] == {"runtime_id": "runtime-run"}


def test_remote_worker_node_result_context_payload_drives_canonical_routing_and_public_state(
    attractor_api_client: TestClient,
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    def unexpected_backend(*args, **kwargs):  # pragma: no cover - assertion path
        raise AssertionError("remote_worker profile should not run through the native backend")

    server.configure_runtime_paths(runs_dir=tmp_path / "runs")
    _write_remote_profile_config(tmp_path)
    controller = _RemoteController(
        outcome={"status": "success", "context_updates": {}},
        outcomes_by_node={"route": {"status": "success", "context_updates": {}}},
        event_context={"context.remote.route": "branch-only"},
    )

    monkeypatch.setattr(server, "_build_codergen_backend", unexpected_backend)
    monkeypatch.setattr(execution_metadata, "RemoteWorkerClient", controller.factory)

    response = attractor_api_client.post(
        "/pipelines",
        json={
            "flow_content": (
                "digraph G { "
                "start [shape=Mdiamond] "
                "route [shape=box] "
                "fallback [shape=box] "
                "done [shape=Msquare] "
                'start -> route [condition="context.remote.route=branch-only"] '
                "start -> fallback "
                "route -> done "
                "fallback -> done }"
            ),
            "working_directory": str(tmp_path / "control" / "project"),
            "execution_profile_id": "remote-fast",
        },
    )

    assert response.status_code == 200, response.text
    run_id = str(response.json()["run_id"])
    completion = wait_for_pipeline_completion(attractor_api_client, run_id)

    assert completion["status"] == "completed", completion
    assert [request.node_id for request in controller.node_requests] == ["start", "route"]

    checkpoint = server.load_checkpoint(server._run_root(run_id) / "state.json")
    assert checkpoint is not None
    assert checkpoint.current_node == "done"
    assert checkpoint.completed_nodes == ["start", "route"]
    assert checkpoint.context["context.remote.route"] == "branch-only"

    context_response = attractor_api_client.get(f"/pipelines/{run_id}/context")
    assert context_response.status_code == 200
    assert context_response.json()["context"]["context.remote.route"] == "branch-only"

    persisted_events = server._read_persisted_run_events(run_id)
    remote_events = [event for event in persisted_events if event["type"] == "remote_worker_event"]
    result_events = [event for event in remote_events if event["event"] == "node_result"]
    assert result_events[0]["payload"]["outcome"]["context_updates"] == {}
    assert result_events[0]["payload"]["context"] == {"context.remote.route": "branch-only"}


def test_remote_worker_human_gate_request_surfaces_public_question_and_posts_callback(
    attractor_api_client: TestClient,
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    server.configure_runtime_paths(runs_dir=tmp_path / "runs")
    _write_remote_profile_config(tmp_path)
    controller = _RemoteController(outcome={"status": "success"})
    controller.request_events = [
        (
            "human_gate_request",
            {
                "gate_id": "gate-1",
                "question": {
                    "text": "Continue remote work?",
                    "type": "YES_NO",
                    "metadata": {"node_id": "start"},
                },
            },
        )
    ]

    monkeypatch.setattr(execution_metadata, "RemoteWorkerClient", controller.factory)

    response = attractor_api_client.post(
        "/pipelines",
        json={
            "flow_content": 'digraph G { start [shape=Mdiamond, label="Start"] done [shape=Msquare] start -> done }',
            "working_directory": str(tmp_path / "control" / "project"),
            "execution_profile_id": "remote-fast",
        },
    )
    assert response.status_code == 200
    run_id = str(response.json()["run_id"])

    question = _wait_for_question(attractor_api_client, run_id)
    assert question["prompt"] == "Continue remote work?"
    assert question["node_id"] == "start"

    answer_response = attractor_api_client.post(
        f"/pipelines/{run_id}/questions/{question['question_id']}/answer",
        json={"selected_value": "YES"},
    )
    assert answer_response.status_code == 200

    completion = wait_for_pipeline_completion(attractor_api_client, run_id)
    assert completion["status"] == "completed", completion
    assert controller.callbacks == [
        (
            "human_gate",
            run_id,
            "gate-1",
            {"value": "YES", "text": "", "selected_values": ["YES"]},
        )
    ]

    persisted_events = server._read_persisted_run_events(run_id)
    remote_events = [event for event in persisted_events if event["type"] == "remote_worker_event"]
    assert [event["event"] for event in remote_events] == [
        "run_ready",
        "node_started",
        "human_gate_request",
        "node_result",
    ]


def test_remote_worker_child_run_and_status_requests_use_public_child_coordination(
    attractor_api_client: TestClient,
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    from attractor.dsl import parse_dot
    from attractor.handlers.execution_container import graph_to_payload

    class ChildBackend:
        def run(self, node_id: str, prompt: str, context, **kwargs) -> Outcome:
            del node_id, prompt, context, kwargs
            return Outcome(
                status=OutcomeStatus.SUCCESS,
                context_updates={"context.child.observable": "done"},
                notes="child completed",
            )

    child_dot_path = tmp_path / "child.dot"
    child_dot = """
    digraph Child {
        start [shape=Mdiamond]
        task [shape=box, spark.writes_context="[\\"context.child.observable\\"]"]
        done [shape=Msquare]
        start -> task
        task -> done
    }
    """
    child_dot_path.write_text(child_dot, encoding="utf-8")

    server.configure_runtime_paths(runs_dir=tmp_path / "runs")
    _write_remote_profile_config(tmp_path)
    controller = _RemoteController(outcome={"status": "success"})
    controller.complete_after_callback_types = {"child_status"}
    controller.request_events = [
        (
            "child_run_request",
            {
                "request_id": "child-request-1",
                "child_run_id": "worker-child-1",
                "child_graph": graph_to_payload(parse_dot(child_dot)),
                "child_flow_name": "Worker Child",
                "child_flow_path": str(child_dot_path),
                "child_workdir": str(tmp_path / "child-work"),
                "parent_context": {},
                "parent_run_id": "__RUN_ID__",
                "parent_node_id": "start",
                "root_run_id": "__RUN_ID__",
            },
        ),
        ("child_status_request", {"request_id": "status-request-1", "run_id": "worker-child-1"}),
    ]

    monkeypatch.setattr(server, "_build_codergen_backend", lambda *args, **kwargs: ChildBackend())
    monkeypatch.setattr(execution_metadata, "RemoteWorkerClient", controller.factory)

    response = attractor_api_client.post(
        "/pipelines",
        json={
            "flow_content": 'digraph G { start [shape=Mdiamond, label="Start"] done [shape=Msquare] start -> done }',
            "working_directory": str(tmp_path / "control" / "project"),
            "execution_profile_id": "remote-fast",
        },
    )
    assert response.status_code == 200, response.text
    parent_run_id = str(response.json()["run_id"])
    completion = wait_for_pipeline_completion(attractor_api_client, parent_run_id)

    assert completion["status"] == "completed", completion
    assert controller.callbacks == [
        (
            "child_run",
            parent_run_id,
            "child-request-1",
            {
                "run_id": "worker-child-1",
                "status": "completed",
                "outcome": "success",
                "outcome_reason_code": None,
                "outcome_reason_message": None,
                "current_node": "done",
                "completed_nodes": ["start", "task"],
                "route_trace": ["start", "task", "done"],
                "failure_reason": "",
            },
        ),
        (
            "child_status",
            parent_run_id,
            "status-request-1",
            {
                "run_id": "worker-child-1",
                "status": "completed",
                "outcome": "success",
                "outcome_reason_code": None,
                "outcome_reason_message": None,
                "current_node": "done",
                "completed_nodes": ["start", "task"],
                "route_trace": [],
                "failure_reason": "",
            },
        ),
    ]

    child_record = server._read_run_meta(server._run_meta_path("worker-child-1"))
    assert child_record is not None
    assert child_record.status == "completed"
    assert child_record.parent_run_id == parent_run_id
    assert child_record.parent_node_id == "start"
    assert child_record.root_run_id == parent_run_id
    assert child_record.child_invocation_index == 1

    child_checkpoint = server.load_checkpoint(server._run_root("worker-child-1") / "state.json")
    assert child_checkpoint is not None
    assert child_checkpoint.context["context.child.observable"] == "done"
    assert child_checkpoint.context["internal.parent_run_id"] == parent_run_id
    assert child_checkpoint.context["internal.parent_node_id"] == "start"
    assert child_checkpoint.context["internal.root_run_id"] == parent_run_id

    parent_events = server._read_persisted_run_events(parent_run_id)
    remote_events = [event for event in parent_events if event["type"] == "remote_worker_event"]
    assert [event["event"] for event in remote_events] == [
        "run_ready",
        "node_started",
        "child_run_request",
        "child_status_request",
        "node_result",
    ]
    assert set(controller.callbacks[0][3]) == {
        "run_id",
        "status",
        "outcome",
        "outcome_reason_code",
        "outcome_reason_message",
        "current_node",
        "completed_nodes",
        "route_trace",
        "failure_reason",
    }


def test_remote_worker_retryable_callback_exhaustion_records_failed_run_state_and_history(
    attractor_api_client: TestClient,
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    def unexpected_backend(*args, **kwargs):  # pragma: no cover - assertion path
        raise AssertionError("remote_worker profile should not run through the native backend")

    server.configure_runtime_paths(runs_dir=tmp_path / "runs")
    _write_remote_profile_config(tmp_path)
    controller = _RemoteController(outcome={"status": "success"})
    controller.request_events = [
        (
            "human_gate_request",
            {
                "gate_id": "gate-1",
                "question": {"text": "Continue remote work?", "type": "YES_NO", "metadata": {"node_id": "start"}},
            },
        )
    ]
    controller.callback_errors = [
        WorkerAPIError("temporarily_unavailable", "worker callback unavailable", 503, retryable=True),
        WorkerAPIError("temporarily_unavailable", "worker callback unavailable", 503, retryable=True),
        WorkerAPIError("temporarily_unavailable", "worker callback unavailable", 503, retryable=True),
    ]

    monkeypatch.setattr(server, "_build_codergen_backend", unexpected_backend)
    monkeypatch.setattr(execution_metadata, "RemoteWorkerClient", controller.factory)

    response = attractor_api_client.post(
        "/pipelines",
        json={
            "flow_content": 'digraph G { start [shape=Mdiamond, label="Start"] done [shape=Msquare] start -> done }',
            "working_directory": str(tmp_path / "control" / "project"),
            "execution_profile_id": "remote-fast",
        },
    )
    assert response.status_code == 200, response.text
    run_id = str(response.json()["run_id"])

    question = _wait_for_question(attractor_api_client, run_id)
    answer_response = attractor_api_client.post(
        f"/pipelines/{run_id}/questions/{question['question_id']}/answer",
        json={"selected_value": "YES"},
    )
    assert answer_response.status_code == 200
    completion = wait_for_pipeline_completion(attractor_api_client, run_id)

    assert completion["status"] == "failed"
    assert "worker callback unavailable" in completion["last_error"]
    assert [
        callback
        for callback in controller.callbacks
        if callback[:3] == ("human_gate", run_id, "gate-1")
    ] == [
        ("human_gate", run_id, "gate-1", {"value": "YES", "text": "", "selected_values": ["YES"]}),
        ("human_gate", run_id, "gate-1", {"value": "YES", "text": "", "selected_values": ["YES"]}),
        ("human_gate", run_id, "gate-1", {"value": "YES", "text": "", "selected_values": ["YES"]}),
    ]

    record = server._read_run_meta(server._run_root(run_id) / "run.json")
    assert record is not None
    assert record.status == "failed"
    assert "worker callback unavailable" in record.last_error

    persisted_events = server._read_persisted_run_events(run_id)
    remote_events = [event for event in persisted_events if event["type"] == "remote_worker_event"]
    assert [event["event"] for event in remote_events] == [
        "run_ready",
        "node_started",
        "human_gate_request",
        "run_failed",
    ]
    assert remote_events[-1]["payload"]["code"] == "worker_callback_delivery_failed"
    assert remote_events[-1]["payload"]["request_event"] == "human_gate_request"
    assert remote_events[-1]["payload"]["request_id"] == "gate-1"
    runtime_events = [event for event in persisted_events if event["type"] == "runtime"]
    assert runtime_events[-1]["status"] == "failed"
    assert "worker callback unavailable" in runtime_events[-1]["last_error"]


def test_remote_worker_post_runs_through_worker_api_process_bridge_and_canonical_state(
    attractor_api_client: TestClient,
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    worker_project = tmp_path / "control" / "project"
    worker_runtime_root = tmp_path / "worker-runtime"
    worker_project.mkdir(parents=True)
    worker_runtime_root.mkdir()
    server.configure_runtime_paths(runs_dir=tmp_path / "runs")
    monkeypatch.setenv("SPARK_WORKER_TOKEN", "test-token")
    monkeypatch.setenv("SPARK_CONFIG_DIR", str(tmp_path / "config"))

    worker_app = create_worker_app(
        token="test-token",
        worker_id="worker-a",
        worker_version="1.2.3",
        capabilities={"shell": True},
        supported_images=["spark-worker:latest"],
        runtime=LocalProcessWorkerRuntime(command=_worker_run_node_command()),
    )
    worker_server, worker_thread, worker_base_url = _start_worker_server(worker_app)
    try:
        _write_remote_profile_config(
            tmp_path,
            base_url=worker_base_url,
            worker_project_root=str(tmp_path / "control"),
            worker_runtime_root=str(worker_runtime_root),
        )

        response = attractor_api_client.post(
            "/pipelines",
            json={
                "flow_content": (
                    "digraph G { "
                    'start [shape=box, prompt="Set remote answer", '
                    'spark.writes_context="[\\"context.remote.answer\\"]"] '
                    "done [shape=Msquare] start -> done }"
                ),
                "working_directory": str(worker_project),
                "execution_profile_id": "remote-fast",
            },
        )

        assert response.status_code == 200, response.text
        run_id = str(response.json()["run_id"])
        completion = wait_for_pipeline_completion(attractor_api_client, run_id)

        assert completion["status"] == "completed", completion
        checkpoint = server.load_checkpoint(server._run_root(run_id) / "state.json")
        assert checkpoint is not None
        assert checkpoint.context["context.remote.answer"] == 42
        assert checkpoint.context["_attractor.node_outcomes"]["start"] == "success"

        record = server._read_run_meta(server._run_root(run_id) / "run.json")
        assert record is not None
        assert record.status == "completed"
        assert record.execution_worker_id == "worker-a"

        persisted_events = server._read_persisted_run_events(run_id)
        remote_events = [event for event in persisted_events if event["type"] == "remote_worker_event"]
        node_events = [event for event in remote_events if event["event"] in {"node_started", "node_result"}]
        assert [event["event"] for event in node_events] == ["node_started", "node_result"]
        started_payload = node_events[0]["payload"]["payload"]
        assert started_payload["graph"]["nodes"]["start"]["attrs"]["prompt"]["value"] == "Set remote answer"
        assert started_payload["working_dir"] == str(worker_project)
        assert started_payload["logs_root"] == str(worker_runtime_root / "runs" / run_id / "logs")
        assert started_payload["context"]["internal.run_id"] == run_id
        assert started_payload["runtime_references"]["run_id"] == run_id
        assert started_payload["node_metadata"]["node_id"] == "start"
        result_payload = node_events[-1]["payload"]
        assert result_payload["outcome"]["status"] == "success"
        assert result_payload["outcome"]["context_updates"]["context.remote.answer"] == 42
        assert result_payload["context"]["internal.run_id"] == run_id
    finally:
        _stop_worker_server(worker_server, worker_thread)


def test_remote_worker_modeled_fail_node_result_applies_through_canonical_run_state(
    attractor_api_client: TestClient,
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    def unexpected_backend(*args, **kwargs):  # pragma: no cover - assertion path
        raise AssertionError("remote_worker profile should not run through the native backend")

    server.configure_runtime_paths(runs_dir=tmp_path / "runs")
    _write_remote_profile_config(tmp_path)
    controller = _RemoteController(
        outcome={"status": "success"},
        outcomes_by_node={
            "work": {
                "status": "fail",
                "failure_reason": "remote modeled failure",
                "context_updates": {"context.remote.failure_seen": True},
                "notes": "worker completed with a modeled failure outcome",
            }
        },
        event_context={"context.remote.failure_seen": True},
        runtime_metadata={"runtime_id": "runtime-fail", "exit_code": 0},
    )

    monkeypatch.setattr(server, "_build_codergen_backend", unexpected_backend)
    monkeypatch.setattr(execution_metadata, "RemoteWorkerClient", controller.factory)

    response = attractor_api_client.post(
        "/pipelines",
        json={
            "flow_content": (
                "digraph G { "
                "start [shape=Mdiamond] "
                'work [shape=box, prompt="Remote work", max_retries=0, retry_target="done", '
                'spark.writes_context="[\\"context.remote.failure_seen\\"]"] '
                "done [shape=Msquare] "
                'start -> work work -> done [condition="outcome=success"] }'
            ),
            "working_directory": str(tmp_path / "control" / "project"),
            "execution_profile_id": "remote-fast",
        },
    )

    assert response.status_code == 200, response.text
    run_id = str(response.json()["run_id"])
    completion = wait_for_pipeline_completion(attractor_api_client, run_id)

    assert completion["status"] == "completed"
    assert completion["last_error"] in {"", None}
    assert len(controller.node_requests) == 2
    assert [request.node_id for request in controller.node_requests] == ["start", "work"]

    checkpoint = server.load_checkpoint(server._run_root(run_id) / "state.json")
    assert checkpoint is not None
    assert checkpoint.current_node == "done"
    assert checkpoint.completed_nodes == ["start", "work"]
    assert checkpoint.context["context.remote.failure_seen"] is True
    assert checkpoint.context["_attractor.node_outcomes"]["start"] == "success"
    assert checkpoint.context["_attractor.node_outcomes"]["work"] == "fail"

    context_response = attractor_api_client.get(f"/pipelines/{run_id}/context")
    assert context_response.status_code == 200
    assert context_response.json()["context"]["context.remote.failure_seen"] is True

    record = server._read_run_meta(server._run_root(run_id) / "run.json")
    assert record is not None
    assert record.status == "completed"
    assert record.last_error == ""

    persisted_events = server._read_persisted_run_events(run_id)
    remote_events = [event for event in persisted_events if event["type"] == "remote_worker_event"]
    assert [event["event"] for event in remote_events] == [
        "run_ready",
        "node_started",
        "node_result",
        "node_started",
        "node_result",
    ]
    assert "node_failed" not in [event["event"] for event in remote_events]
    result_payload = remote_events[4]["payload"]
    assert result_payload["outcome"]["status"] == "fail"
    assert result_payload["outcome"]["failure_reason"] == "remote modeled failure"
    assert result_payload["outcome"]["context_updates"] == {"context.remote.failure_seen": True}
    assert result_payload["context"] == {"context.remote.failure_seen": True}
    assert result_payload["runtime_metadata"] == {"runtime_id": "runtime-fail", "exit_code": 0}

    runtime_events = [event for event in persisted_events if event["type"] == "runtime"]
    assert runtime_events[-1]["status"] == "completed"
    assert runtime_events[-1]["last_error"] in {"", None}


def test_start_pipeline_rejects_dot_authored_execution_profile_context_selection(
    attractor_api_client: TestClient,
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    child_dot_path = tmp_path / "child.dot"
    child_dot_path.write_text(
        """
        digraph Child {
            start [shape=Mdiamond, spark.writes_context="[\\"context.remote.child_answer\\"]"]
            done [shape=Msquare]
            start -> done
        }
        """,
        encoding="utf-8",
    )

    server.configure_runtime_paths(runs_dir=tmp_path / "runs")
    monkeypatch.setattr(server.asyncio, "create_task", _close_task_immediately)

    response = attractor_api_client.post(
        "/pipelines",
        json={
            "flow_content": f"""
            digraph Parent {{
                graph [stack.child_dotfile="{child_dot_path}"]
                start [shape=Mdiamond]
                select_remote [
                    shape=box,
                    prompt="Select remote child execution",
                    spark.writes_context="[\\"_attractor.runtime.execution_profile_id\\", \\"_attractor.runtime.execution_profile_selection_source\\"]"
                ]
                manager [shape=house, type="stack.manager_loop", manager.actions="", manager.max_cycles=1]
                done [shape=Msquare]

                start -> select_remote
                select_remote -> manager
                manager -> done
            }}
            """,
            "working_directory": str(tmp_path / "control" / "project"),
        },
    )

    assert response.status_code == 200, response.text
    payload = response.json()
    assert payload["status"] == "validation_error"
    errors = payload["errors"]
    assert len(errors) == 1
    assert errors[0]["rule_id"] == "execution_placement_context_non_authoritative"
    assert errors[0]["severity"] == "error"
    assert errors[0]["node"] == "select_remote"
    assert "_attractor.runtime.execution_profile_id" in errors[0]["message"]
    assert "_attractor.runtime.execution_profile_selection_source" in errors[0]["message"]


def test_remote_worker_node_failed_records_active_run_failure(
    attractor_api_client: TestClient,
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    def unexpected_backend(*args, **kwargs):  # pragma: no cover - assertion path
        raise AssertionError("remote_worker profile should not run through the native backend")

    server.configure_runtime_paths(runs_dir=tmp_path / "runs")
    _write_remote_profile_config(tmp_path)
    controller = _RemoteController(
        outcome={"status": "success"},
        node_failure={"code": "process_crashed", "message": "worker process crashed", "retryable": True},
        failure_node="work",
    )

    monkeypatch.setattr(server, "_build_codergen_backend", unexpected_backend)
    monkeypatch.setattr(execution_metadata, "RemoteWorkerClient", controller.factory)

    response = attractor_api_client.post(
        "/pipelines",
        json={
            "flow_content": (
                "digraph G { "
                "start [shape=Mdiamond] "
                'work [shape=box, prompt="Remote work", max_retries=0, retry_target="fix"] '
                'fix [shape=box, prompt="Should not run"] done [shape=Msquare] '
                'start -> work work -> done [condition="outcome=success"] '
                'work -> fix [condition="outcome=fail"] fix -> done }'
            ),
            "working_directory": str(tmp_path / "control" / "project"),
            "execution_profile_id": "remote-fast",
        },
    )

    assert response.status_code == 200
    payload = response.json()
    assert payload["status"] == "started", payload
    run_id = str(payload["run_id"])
    completion = wait_for_pipeline_completion(attractor_api_client, run_id)

    assert completion["status"] == "failed"
    assert "worker process crashed" in completion["last_error"]
    assert len(controller.node_requests) == 2
    assert [request.node_id for request in controller.node_requests] == ["start", "work"]
    checkpoint = server.load_checkpoint(server._run_root(run_id) / "state.json")
    assert checkpoint is not None
    assert checkpoint.current_node == "work"
    assert checkpoint.completed_nodes == ["start"]
    assert checkpoint.context.get("_attractor.node_outcomes", {}) == {"start": "success"}
    record = server._read_run_meta(server._run_root(run_id) / "run.json")
    assert record is not None
    assert record.status == "failed"
    assert "worker process crashed" in record.last_error
    persisted_events = server._read_persisted_run_events(run_id)
    remote_events = [event for event in persisted_events if event["type"] == "remote_worker_event"]
    assert [event["event"] for event in remote_events] == [
        "run_ready",
        "node_started",
        "node_result",
        "node_started",
        "node_failed",
    ]
    work_stage_events = [
        event for event in persisted_events if event.get("node_id") == "work" and event["type"].startswith("Stage")
    ]
    assert [event["type"] for event in work_stage_events] == ["StageStarted"]
    runtime_events = [event for event in persisted_events if event["type"] == "runtime"]
    assert runtime_events[-1]["status"] == "failed"
    assert "worker process crashed" in runtime_events[-1]["last_error"]


def test_remote_worker_run_failed_after_ready_records_active_run_failure(
    attractor_api_client: TestClient,
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    def unexpected_backend(*args, **kwargs):  # pragma: no cover - assertion path
        raise AssertionError("remote_worker profile should not run through the native backend")

    server.configure_runtime_paths(runs_dir=tmp_path / "runs")
    _write_remote_profile_config(tmp_path)
    controller = _RemoteController(
        outcome={"status": "success"},
        run_failure={"code": "runtime_lost", "message": "worker lost runtime after readiness", "retryable": False},
        failure_node="work",
    )

    monkeypatch.setattr(server, "_build_codergen_backend", unexpected_backend)
    monkeypatch.setattr(execution_metadata, "RemoteWorkerClient", controller.factory)

    response = attractor_api_client.post(
        "/pipelines",
        json={
            "flow_content": (
                "digraph G { "
                "start [shape=Mdiamond] "
                'work [shape=box, prompt="Remote work", max_retries=0, retry_target="fix"] '
                'fix [shape=box, prompt="Should not run"] done [shape=Msquare] '
                'start -> work work -> done [condition="outcome=success"] '
                'work -> fix [condition="outcome=fail"] fix -> done }'
            ),
            "working_directory": str(tmp_path / "control" / "project"),
            "execution_profile_id": "remote-fast",
        },
    )

    assert response.status_code == 200, response.text
    run_id = str(response.json()["run_id"])
    completion = wait_for_pipeline_completion(attractor_api_client, run_id)

    assert completion["status"] == "failed"
    assert "worker lost runtime after readiness" in completion["last_error"]
    assert len(controller.node_requests) == 2
    assert [request.node_id for request in controller.node_requests] == ["start", "work"]

    record = server._read_run_meta(server._run_root(run_id) / "run.json")
    assert record is not None
    assert record.status == "failed"
    assert "worker lost runtime after readiness" in record.last_error

    checkpoint = server.load_checkpoint(server._run_root(run_id) / "state.json")
    assert checkpoint is not None
    assert checkpoint.current_node == "work"
    assert checkpoint.completed_nodes == ["start"]
    assert checkpoint.context.get("_attractor.node_outcomes", {}) == {"start": "success"}

    persisted_events = server._read_persisted_run_events(run_id)
    remote_events = [event for event in persisted_events if event["type"] == "remote_worker_event"]
    assert [event["event"] for event in remote_events] == [
        "run_ready",
        "node_started",
        "node_result",
        "node_started",
        "run_failed",
    ]
    work_stage_events = [
        event for event in persisted_events if event.get("node_id") == "work" and event["type"].startswith("Stage")
    ]
    assert [event["type"] for event in work_stage_events] == ["StageStarted"]
    assert remote_events[-1]["payload"] == {
        "code": "runtime_lost",
        "message": "worker lost runtime after readiness",
        "retryable": False,
    }
    runtime_events = [event for event in persisted_events if event["type"] == "runtime"]
    assert runtime_events[-1]["status"] == "failed"
    assert "worker lost runtime after readiness" in runtime_events[-1]["last_error"]


def test_remote_worker_run_failed_before_ready_records_preparation_failure_without_node_result(
    attractor_api_client: TestClient,
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    def unexpected_backend(*args, **kwargs):  # pragma: no cover - assertion path
        raise AssertionError("remote_worker profile should not run through the native backend")

    server.configure_runtime_paths(runs_dir=tmp_path / "runs")
    _write_remote_profile_config(tmp_path)
    controller = _RemoteController(
        outcome={"status": "success"},
        admission_status="preparing",
        preparation_failure={
            "code": "runtime_prepare_failed",
            "message": "worker path does not exist",
            "retryable": False,
            "details": {"missing": "/srv/projects/project"},
        },
    )

    monkeypatch.setattr(server, "_build_codergen_backend", unexpected_backend)
    monkeypatch.setattr(execution_metadata, "RemoteWorkerClient", controller.factory)

    response = attractor_api_client.post(
        "/pipelines",
        json={
            "flow_content": "digraph G { start [shape=Mdiamond] done [shape=Msquare] start -> done }",
            "working_directory": str(tmp_path / "control" / "project"),
            "execution_profile_id": "remote-fast",
        },
    )

    assert response.status_code == 200, response.text
    run_id = str(response.json()["run_id"])
    completion = wait_for_pipeline_completion(attractor_api_client, run_id)

    assert completion["status"] == "failed"
    assert "worker path does not exist" in completion["last_error"]
    assert controller.node_requests == []

    record = server._read_run_meta(server._run_root(run_id) / "run.json")
    assert record is not None
    assert record.status == "failed"
    assert "worker path does not exist" in record.last_error

    checkpoint = server.load_checkpoint(server._run_root(run_id) / "state.json")
    assert checkpoint is not None
    assert checkpoint.current_node == "start"
    assert checkpoint.completed_nodes == []
    assert checkpoint.context.get("_attractor.node_outcomes", {}) == {}

    persisted_events = server._read_persisted_run_events(run_id)
    remote_events = [event for event in persisted_events if event["type"] == "remote_worker_event"]
    assert [event["event"] for event in remote_events] == ["run_failed"]
    assert remote_events[-1]["payload"] == {
        "code": "runtime_prepare_failed",
        "message": "worker path does not exist",
        "retryable": False,
        "details": {"missing": "/srv/projects/project"},
    }
    assert "StageCompleted" not in [event["type"] for event in persisted_events]
    runtime_events = [event for event in persisted_events if event["type"] == "runtime"]
    assert runtime_events[-1]["status"] == "failed"
    assert "worker path does not exist" in runtime_events[-1]["last_error"]


def test_remote_worker_stream_close_before_ready_records_active_run_failure(
    attractor_api_client: TestClient,
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    def unexpected_backend(*args, **kwargs):  # pragma: no cover - assertion path
        raise AssertionError("remote_worker profile should not run through the native backend")

    server.configure_runtime_paths(runs_dir=tmp_path / "runs")
    _write_remote_profile_config(tmp_path)
    controller = _RemoteController(
        outcome={"status": "success"},
        admission_status="preparing",
        close_stream_before_ready=True,
    )

    monkeypatch.setattr(server, "_build_codergen_backend", unexpected_backend)
    monkeypatch.setattr(execution_metadata, "RemoteWorkerClient", controller.factory)

    response = attractor_api_client.post(
        "/pipelines",
        json={
            "flow_content": "digraph G { start [shape=Mdiamond] done [shape=Msquare] start -> done }",
            "working_directory": str(tmp_path / "control" / "project"),
            "execution_profile_id": "remote-fast",
        },
    )

    assert response.status_code == 200, response.text
    run_id = str(response.json()["run_id"])
    completion = wait_for_pipeline_completion(attractor_api_client, run_id)

    assert completion["status"] == "failed"
    assert "closed before run_ready" in completion["last_error"]
    assert controller.node_requests == []

    record = server._read_run_meta(server._run_root(run_id) / "run.json")
    assert record is not None
    assert record.status == "failed"
    assert "closed before run_ready" in record.last_error

    persisted_events = server._read_persisted_run_events(run_id)
    remote_events = [event for event in persisted_events if event["type"] == "remote_worker_event"]
    assert [event["event"] for event in remote_events] == ["run_failed"]
    assert remote_events[-1]["payload"]["code"] == "worker_event_stream_closed"
    runtime_events = [event for event in persisted_events if event["type"] == "runtime"]
    assert runtime_events[-1]["status"] == "failed"
    assert "closed before run_ready" in runtime_events[-1]["last_error"]


def test_remote_worker_stream_close_after_node_post_records_active_run_failure(
    attractor_api_client: TestClient,
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    def unexpected_backend(*args, **kwargs):  # pragma: no cover - assertion path
        raise AssertionError("remote_worker profile should not run through the native backend")

    server.configure_runtime_paths(runs_dir=tmp_path / "runs")
    _write_remote_profile_config(tmp_path)
    controller = _RemoteController(
        outcome={"status": "success"},
        close_stream_after_node_started=True,
    )

    monkeypatch.setattr(server, "_build_codergen_backend", unexpected_backend)
    monkeypatch.setattr(execution_metadata, "RemoteWorkerClient", controller.factory)

    response = attractor_api_client.post(
        "/pipelines",
        json={
            "flow_content": "digraph G { start [shape=Mdiamond] done [shape=Msquare] start -> done }",
            "working_directory": str(tmp_path / "control" / "project"),
            "execution_profile_id": "remote-fast",
        },
    )

    assert response.status_code == 200, response.text
    run_id = str(response.json()["run_id"])
    completion = wait_for_pipeline_completion(attractor_api_client, run_id)

    assert completion["status"] == "failed"
    assert "closed before required node result" in completion["last_error"]
    assert len(controller.node_requests) == 1

    record = server._read_run_meta(server._run_root(run_id) / "run.json")
    assert record is not None
    assert record.status == "failed"
    assert "closed before required node result" in record.last_error

    persisted_events = server._read_persisted_run_events(run_id)
    remote_events = [event for event in persisted_events if event["type"] == "remote_worker_event"]
    assert [event["event"] for event in remote_events] == ["run_ready", "node_started", "run_failed"]
    assert remote_events[-1]["payload"]["code"] == "worker_event_stream_closed"
    assert controller.node_requests[0].node_execution_id in remote_events[-1]["payload"]["message"]
    runtime_events = [event for event in persisted_events if event["type"] == "runtime"]
    assert runtime_events[-1]["status"] == "failed"
    assert "closed before required node result" in runtime_events[-1]["last_error"]


def test_start_pipeline_preserves_structured_remote_admission_rejection(
    attractor_api_client: TestClient,
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    def unexpected_backend(*args, **kwargs):  # pragma: no cover - assertion path
        raise AssertionError("remote admission rejection should stop before execution")

    server.configure_runtime_paths(runs_dir=tmp_path / "runs")
    _write_remote_profile_config(tmp_path)
    admission_requests = []

    class RejectingRemoteClient:
        def __init__(self, worker):
            self.worker = worker

        def __enter__(self):
            return self

        def __exit__(self, *_exc_info):
            return None

        def health(self):
            return WorkerHealthResponse(
                worker_id="worker-a",
                worker_version="1.2.3",
                protocol_version="v1",
                status="ok",
                capabilities={"shell": True},
            )

        def worker_info(self):
            return WorkerInfoResponse(
                worker_id="worker-a",
                worker_version="1.2.3",
                protocol_version="v1",
                status="ok",
                capabilities={"shell": True},
            )

        def admit_run(self, request):
            admission_requests.append(request)
            raise WorkerAPIError(
                "worker_path_missing",
                "mapped project path does not exist",
                422,
                retryable=False,
                details={"mapped_project_path": "/srv/projects/project"},
            )

    monkeypatch.setattr(server, "_build_codergen_backend", unexpected_backend)
    monkeypatch.setattr(execution_metadata, "RemoteWorkerClient", RejectingRemoteClient)
    monkeypatch.setattr(server.asyncio, "create_task", _close_task_immediately)

    response = attractor_api_client.post(
        "/pipelines",
        json={
            "flow_content": "digraph G { start [shape=Mdiamond] done [shape=Msquare] start -> done }",
            "working_directory": str(tmp_path / "control" / "project"),
            "execution_profile_id": "remote-fast",
        },
    )

    assert response.status_code == 200
    payload = response.json()
    assert payload["status"] == "validation_error"
    assert "mapped project path does not exist" in payload["error"]
    assert payload["error_code"] == "worker_path_missing"
    assert payload["retryable"] is False
    assert payload["details"] == {"mapped_project_path": "/srv/projects/project"}
    assert len(admission_requests) == 1


def _write_remote_profile_config(
    tmp_path: Path,
    *,
    base_url: str = "https://worker.example",
    worker_project_root: str = "/srv/projects",
    worker_runtime_root: str = "/srv/runtime",
) -> None:
    config_dir = server.get_runtime_paths().config_dir
    config_dir.mkdir(parents=True, exist_ok=True)
    (config_dir / "execution-profiles.toml").write_text(
        f"""
        [workers.worker-a]
        label = "Worker A"
        enabled = true
        base_url = "{base_url}"
        auth_token_env = "SPARK_WORKER_TOKEN"

        [profiles.remote-fast]
        mode = "remote_worker"
        label = "Remote Fast"
        worker = "worker-a"
        image = "spark-worker:latest"
        control_project_root = "{tmp_path / "control"}"
        worker_project_root = "{worker_project_root}"
        worker_runtime_root = "{worker_runtime_root}"
        capabilities = ["shell"]
        """,
        encoding="utf-8",
    )


def _worker_run_node_command() -> list[str]:
    script = """
from attractor.engine import Outcome, OutcomeStatus
import attractor.api.codex_backends as codex_backends

class Backend:
    def run(self, node_id, prompt, context, **kwargs):
        return Outcome(
            status=OutcomeStatus.SUCCESS,
            context_updates={"context.remote.answer": 42},
            notes=f"worker handled {node_id}",
        )

codex_backends.build_codergen_backend = lambda *args, **kwargs: Backend()

from attractor.handlers.execution_container import run_worker_node
raise SystemExit(run_worker_node())
"""
    return [sys.executable, "-c", script]


def _unused_tcp_port() -> int:
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as sock:
        sock.bind(("127.0.0.1", 0))
        return int(sock.getsockname()[1])


def _start_worker_server(app) -> tuple[uvicorn.Server, threading.Thread, str]:
    port = _unused_tcp_port()
    worker_server = uvicorn.Server(
        uvicorn.Config(app, host="127.0.0.1", port=port, log_level="warning", lifespan="off")
    )
    thread = threading.Thread(target=worker_server.run, daemon=True)
    thread.start()
    deadline = time.monotonic() + 5
    while not worker_server.started and thread.is_alive() and time.monotonic() < deadline:
        time.sleep(0.01)
    if not worker_server.started:
        worker_server.should_exit = True
        thread.join(timeout=2)
        raise AssertionError("worker test server did not start")
    return worker_server, thread, f"http://127.0.0.1:{port}"


def _stop_worker_server(worker_server: uvicorn.Server, thread: threading.Thread) -> None:
    worker_server.should_exit = True
    thread.join(timeout=5)


def _wait_for_question(client: TestClient, run_id: str) -> dict[str, Any]:
    deadline = time.monotonic() + 5
    while time.monotonic() < deadline:
        response = client.get(f"/pipelines/{run_id}/questions")
        assert response.status_code == 200
        questions = response.json()["questions"]
        if questions:
            return questions[0]
        time.sleep(0.01)
    raise AssertionError("timed out waiting for human gate question")


def _replace_run_id_placeholders(value: Any, run_id: str) -> Any:
    if value == "__RUN_ID__":
        return run_id
    if isinstance(value, dict):
        return {key: _replace_run_id_placeholders(item, run_id) for key, item in value.items()}
    if isinstance(value, list):
        return [_replace_run_id_placeholders(item, run_id) for item in value]
    return value


class _RemoteController:
    def __init__(
        self,
        *,
        outcome: dict[str, Any],
        outcomes_by_node: dict[str, dict[str, Any]] | None = None,
        node_failure: dict[str, Any] | None = None,
        run_failure: dict[str, Any] | None = None,
        failure_node: str | None = None,
        preparation_failure: dict[str, Any] | None = None,
        admission_status: str = "ready",
        event_context: dict[str, Any] | None = None,
        runtime_metadata: dict[str, Any] | None = None,
        close_stream_before_ready: bool = False,
        close_stream_after_node_started: bool = False,
    ) -> None:
        self.outcome = dict(outcome)
        self.outcomes_by_node = {node_id: dict(outcome) for node_id, outcome in dict(outcomes_by_node or {}).items()}
        self.node_failure = dict(node_failure) if node_failure is not None else None
        self.run_failure = dict(run_failure) if run_failure is not None else None
        self.failure_node = failure_node
        self.preparation_failure = dict(preparation_failure) if preparation_failure is not None else None
        self.admission_status = admission_status
        self.event_context = dict(event_context) if event_context is not None else {"context.remote.answer": 42}
        self.runtime_metadata = dict(runtime_metadata) if runtime_metadata is not None else {"runtime_id": "runtime-run"}
        self.close_stream_before_ready = close_stream_before_ready
        self.close_stream_after_node_started = close_stream_after_node_started
        self.node_requests: list[WorkerNodeRequest] = []
        self.request_events: list[tuple[str, dict[str, Any]]] = []
        self.callbacks: list[tuple[str, str, str, dict[str, Any]]] = []
        self.callback_errors: list[BaseException] = []
        self.cancel_requests: list[str] = []
        self.cleanup_requests: list[str] = []
        self.cleanup_error: BaseException | None = None
        self.complete_after_callback_types: set[str] = {"human_gate", "child_run", "child_status"}
        self._events: list[WorkerEvent | None] = []
        self._condition = threading.Condition()
        self._sequence = 0

    def factory(self, worker):
        return _FakeRemoteClient(self, worker)

    def next_event(self) -> WorkerEvent | None:
        with self._condition:
            while not self._events:
                self._condition.wait(timeout=0.25)
            return self._events.pop(0)

    def push(self, event: WorkerEvent | None) -> None:
        with self._condition:
            self._events.append(event)
            self._condition.notify_all()

    def event(self, run_id: str, event_type: str, payload: dict[str, Any], *, node_id: str | None = None) -> WorkerEvent:
        self._sequence += 1
        return WorkerEvent(
            run_id=run_id,
            sequence=self._sequence,
            event_type=event_type,
            timestamp=datetime.now(timezone.utc),
            worker_id="worker-a",
            execution_profile_id="remote-fast",
            payload=payload,
            node_id=node_id,
            node_attempt=1 if node_id else None,
        )


class _FakeRemoteClient:
    def __init__(self, controller: _RemoteController, worker) -> None:
        self.controller = controller
        self.worker = worker

    def __enter__(self):
        return self

    def __exit__(self, *_exc_info):
        return None

    def health(self):
        return WorkerHealthResponse(
            worker_id="worker-a",
            worker_version="1.2.3",
            protocol_version="v1",
            status="ok",
            capabilities={"shell": True},
        )

    def worker_info(self):
        return WorkerInfoResponse(
            worker_id="worker-a",
            worker_version="1.2.3",
            protocol_version="v1",
            status="ok",
            capabilities={"shell": True},
        )

    def admit_run(self, request):
        return WorkerRunAdmissionResponse(
            run_id=request.run_id,
            worker_id="worker-a",
            status=self.controller.admission_status,
            event_url=f"/v1/runs/{request.run_id}/events",
            last_sequence=0,
            accepted=True,
        )

    def stream_events(self, run_id: str, *, after: int | None = None, last_event_id: str | None = None):
        if self.controller.preparation_failure is not None:
            yield self.controller.event(
                run_id,
                "run_failed",
                dict(self.controller.preparation_failure),
            )
            return
        if self.controller.close_stream_before_ready:
            return
        if self.controller.admission_status == "ready":
            yield self.controller.event(run_id, "run_ready", {"status": "ready"})
        while True:
            event = self.controller.next_event()
            if event is None:
                return
            yield event

    def submit_node(self, run_id: str, request: WorkerNodeRequest) -> WorkerNodeAcceptedResponse:
        self.controller.node_requests.append(request)
        self.controller.push(
            self.controller.event(
                run_id,
                "node_started",
                {"status": "started", "node_execution_id": request.node_execution_id},
                node_id=request.node_id,
            )
        )
        if self.controller.close_stream_after_node_started:
            self.controller.push(None)
            return WorkerNodeAcceptedResponse(
                run_id=run_id,
                node_execution_id=request.node_execution_id or request.node_id,
                node_id=request.node_id,
                attempt=request.attempt,
            )
        if self.controller.request_events:
            for event_type, payload in self.controller.request_events:
                self.controller.push(
                    self.controller.event(
                        run_id,
                        event_type,
                        _replace_run_id_placeholders(payload, run_id),
                        node_id=request.node_id,
                    )
                )
            return WorkerNodeAcceptedResponse(
                run_id=run_id,
                node_execution_id=request.node_execution_id or request.node_id,
                node_id=request.node_id,
                attempt=request.attempt,
            )
        if self.controller.node_failure is not None and self._should_fail_node(request.node_id):
            self.controller.push(
                self.controller.event(
                    run_id,
                    "node_failed",
                    {"node_execution_id": request.node_execution_id, **self.controller.node_failure},
                    node_id=request.node_id,
                )
            )
        elif self.controller.run_failure is not None and self._should_fail_node(request.node_id):
            self.controller.push(
                self.controller.event(
                    run_id,
                    "run_failed",
                    dict(self.controller.run_failure),
                )
            )
        else:
            self.controller.push(
                self.controller.event(
                    run_id,
                        "node_result",
                        {
                            "node_execution_id": request.node_execution_id,
                            "outcome": self.controller.outcomes_by_node.get(request.node_id, self.controller.outcome),
                            "context": self.controller.event_context,
                            "runtime_metadata": self.controller.runtime_metadata,
                        },
                    node_id=request.node_id,
                )
            )
        if self._should_close_after_submit(request.node_id):
            self.controller.push(None)
        return WorkerNodeAcceptedResponse(
            run_id=run_id,
            node_execution_id=request.node_execution_id or request.node_id,
            node_id=request.node_id,
            attempt=request.attempt,
        )

    def _should_fail_node(self, node_id: str) -> bool:
        return self.controller.failure_node is None or self.controller.failure_node == node_id

    def _should_close_after_submit(self, node_id: str) -> bool:
        if self.controller.failure_node is not None:
            return self.controller.failure_node == node_id
        if self.controller.outcomes_by_node:
            return node_id in self.controller.outcomes_by_node
        return True

    def answer_human_gate(self, run_id: str, gate_id: str, request) -> WorkerCallbackResponse:
        self.controller.callbacks.append(("human_gate", run_id, gate_id, dict(request.payload)))
        self._raise_callback_error_if_configured()
        if "human_gate" in self.controller.complete_after_callback_types:
            self._complete_after_callback(run_id)
        return WorkerCallbackResponse(run_id=run_id, request_id=gate_id, status="accepted")

    def child_run_result(self, run_id: str, request_id: str, request) -> WorkerCallbackResponse:
        self.controller.callbacks.append(("child_run", run_id, request_id, dict(request.payload)))
        self._raise_callback_error_if_configured()
        if "child_run" in self.controller.complete_after_callback_types:
            self._complete_after_callback(run_id)
        return WorkerCallbackResponse(run_id=run_id, request_id=request_id, status="accepted")

    def child_status_result(self, run_id: str, request_id: str, request) -> WorkerCallbackResponse:
        self.controller.callbacks.append(("child_status", run_id, request_id, dict(request.payload)))
        self._raise_callback_error_if_configured()
        if "child_status" in self.controller.complete_after_callback_types:
            self._complete_after_callback(run_id)
        return WorkerCallbackResponse(run_id=run_id, request_id=request_id, status="accepted")

    def cancel_run(self, run_id: str) -> WorkerCancelResponse:
        self.controller.cancel_requests.append(run_id)
        self.controller.push(self.controller.event(run_id, "run_canceling", {"status": "canceling"}))
        self.controller.push(self.controller.event(run_id, "run_canceled", {"status": "canceled", "message": "aborted_by_user"}))
        self.controller.push(None)
        return WorkerCancelResponse(run_id=run_id, status="canceling")

    def cleanup_run(self, run_id: str) -> WorkerCleanupResponse:
        self.controller.cleanup_requests.append(run_id)
        if self.controller.cleanup_error is not None:
            raise self.controller.cleanup_error
        return WorkerCleanupResponse(run_id=run_id, status="closed", deleted=True)

    def _raise_callback_error_if_configured(self) -> None:
        if self.controller.callback_errors:
            error = self.controller.callback_errors.pop(0)
            if isinstance(error, BaseException):
                raise error

    def _complete_after_callback(self, run_id: str) -> None:
        if not self.controller.node_requests:
            return
        request = self.controller.node_requests[-1]
        self.controller.push(
            self.controller.event(
                run_id,
                "node_result",
                {
                    "node_execution_id": request.node_execution_id,
                    "outcome": self.controller.outcome,
                    "context": self.controller.event_context,
                    "runtime_metadata": self.controller.runtime_metadata,
                },
                node_id=request.node_id,
            )
        )
        self.controller.push(None)


@pytest.mark.parametrize(
    ("failure", "expected_error"),
    [
        ("worker_api_error", "worker-info check failed: worker metadata unavailable"),
        ("protocol_error", "did not advertise protocol metadata"),
    ],
)
def test_start_pipeline_rejects_remote_worker_info_failures_before_admission(
    attractor_api_client: TestClient,
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
    failure: str,
    expected_error: str,
) -> None:
    def unexpected_backend(*args, **kwargs):  # pragma: no cover - assertion path
        raise AssertionError("remote worker validation failure should stop before execution")

    server.configure_runtime_paths(runs_dir=tmp_path / "runs")
    config_dir = server.get_runtime_paths().config_dir
    config_dir.mkdir(parents=True, exist_ok=True)
    control_root = tmp_path / "control"
    (config_dir / "execution-profiles.toml").write_text(
        f"""
        [workers.worker-a]
        label = "Worker A"
        enabled = true
        base_url = "https://worker.example"
        auth_token_env = "SPARK_WORKER_TOKEN"

        [profiles.remote-fast]
        mode = "remote_worker"
        label = "Remote Fast"
        worker = "worker-a"
        image = "spark-worker:latest"
        control_project_root = "{control_root}"
        worker_project_root = "/srv/projects"
        worker_runtime_root = "/srv/runtime"
        capabilities = ["shell"]
        """,
        encoding="utf-8",
    )
    admission_requests = []

    class FakeRemoteClient:
        def __init__(self, worker):
            self.worker = worker

        def __enter__(self):
            return self

        def __exit__(self, *_exc_info):
            return None

        def health(self):
            return WorkerHealthResponse(
                worker_id="worker-a",
                worker_version="1.2.3",
                protocol_version="v1",
                status="ok",
                capabilities={"shell": True},
            )

        def worker_info(self):
            if failure == "worker_api_error":
                raise WorkerAPIError("worker_info_unavailable", "worker metadata unavailable", 503)
            raise ExecutionProtocolError("Remote worker 'worker-a' did not advertise protocol metadata.")

        def admit_run(self, request):
            admission_requests.append(request)
            raise AssertionError("remote worker admission should not be attempted")

    monkeypatch.setattr(server, "_build_codergen_backend", unexpected_backend)
    monkeypatch.setattr(execution_metadata, "RemoteWorkerClient", FakeRemoteClient)
    monkeypatch.setattr(server.asyncio, "create_task", _close_task_immediately)

    response = attractor_api_client.post(
        "/pipelines",
        json={
            "flow_content": "digraph G { start [shape=Mdiamond] done [shape=Msquare] start -> done }",
            "working_directory": str(control_root / "project"),
            "execution_profile_id": "remote-fast",
        },
    )

    assert response.status_code == 200
    payload = response.json()
    assert payload["status"] == "validation_error"
    assert expected_error in payload["error"]
    assert admission_requests == []


def test_start_pipeline_rejects_mismatched_remote_worker_identity_before_admission(
    attractor_api_client: TestClient,
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    def unexpected_backend(*args, **kwargs):  # pragma: no cover - assertion path
        raise AssertionError("remote worker validation failure should stop before execution")

    server.configure_runtime_paths(runs_dir=tmp_path / "runs")
    config_dir = server.get_runtime_paths().config_dir
    config_dir.mkdir(parents=True, exist_ok=True)
    control_root = tmp_path / "control"
    (config_dir / "execution-profiles.toml").write_text(
        f"""
        [workers.worker-a]
        label = "Worker A"
        enabled = true
        base_url = "https://worker.example"
        auth_token_env = "SPARK_WORKER_TOKEN"

        [profiles.remote-fast]
        mode = "remote_worker"
        label = "Remote Fast"
        worker = "worker-a"
        image = "spark-worker:latest"
        control_project_root = "{control_root}"
        worker_project_root = "/srv/projects"
        worker_runtime_root = "/srv/runtime"
        capabilities = ["shell"]
        """,
        encoding="utf-8",
    )
    admission_requests = []

    class FakeRemoteClient:
        def __init__(self, worker):
            self.worker = worker

        def __enter__(self):
            return self

        def __exit__(self, *_exc_info):
            return None

        def health(self):
            return WorkerHealthResponse(
                worker_id="worker-b",
                worker_version="1.2.3",
                protocol_version="v1",
                status="ok",
                capabilities={"shell": True},
            )

        def worker_info(self):
            return WorkerInfoResponse(
                worker_id="worker-b",
                worker_version="1.2.3",
                protocol_version="v1",
                status="ok",
                capabilities={"shell": True},
            )

        def admit_run(self, request):
            admission_requests.append(request)
            raise AssertionError("remote worker admission should not be attempted")

    monkeypatch.setattr(server, "_build_codergen_backend", unexpected_backend)
    monkeypatch.setattr(execution_metadata, "RemoteWorkerClient", FakeRemoteClient)
    monkeypatch.setattr(server.asyncio, "create_task", _close_task_immediately)

    run_id = "wrong-worker-identity"
    response = attractor_api_client.post(
        "/pipelines",
        json={
            "run_id": run_id,
            "flow_content": "digraph G { start [shape=Mdiamond] done [shape=Msquare] start -> done }",
            "working_directory": str(control_root / "project"),
            "execution_profile_id": "remote-fast",
        },
    )

    assert response.status_code == 200
    payload = response.json()
    assert payload["status"] == "validation_error"
    assert "mismatched configured worker identity" in payload["error"]
    assert admission_requests == []
    assert server._read_run_meta(server._run_meta_path(run_id)) is None


def test_start_pipeline_ignores_execution_container_image_as_selection_field(
    attractor_api_client: TestClient,
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    server.configure_runtime_paths(runs_dir=tmp_path / "runs")
    monkeypatch.setattr(server.asyncio, "create_task", _close_task_immediately)

    response = attractor_api_client.post(
        "/pipelines",
        json={
            "flow_content": "digraph G { start [shape=Mdiamond] done [shape=Msquare] start -> done }",
            "working_directory": str(tmp_path / "work"),
            "execution_container_image": "spark-exec:latest",
        },
    )

    assert response.status_code == 200
    payload = response.json()
    assert payload["status"] == "started"
    assert payload["execution_mode"] == "native"
    assert payload["execution_profile_id"] == "native"
    assert payload["execution_container_image"] is None


def test_start_pipeline_ignores_dot_authored_execution_profile_selection(
    attractor_api_client: TestClient,
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    server.configure_runtime_paths(runs_dir=tmp_path / "runs")
    config_dir = server.get_runtime_paths().config_dir
    config_dir.mkdir(parents=True, exist_ok=True)
    (config_dir / "execution-profiles.toml").write_text(
        """
        [profiles.local-dev]
        mode = "local_container"
        label = "Local Dev"
        image = "spark-exec:latest"
        """,
        encoding="utf-8",
    )
    monkeypatch.setattr(server.asyncio, "create_task", _close_task_immediately)

    response = attractor_api_client.post(
        "/pipelines",
        json={
            "flow_content": """
digraph G {
    graph [
        execution_profile_id="local-dev",
        execution_mode="local_container",
        execution_container_image="spark-exec:latest",
        worker="worker-a"
    ]
    start [shape=Mdiamond, execution_profile_id="local-dev"]
    done [shape=Msquare]
    start -> done
}
""",
            "working_directory": str(tmp_path / "work"),
        },
    )

    assert response.status_code == 200
    payload = response.json()
    assert payload["status"] == "started"
    assert payload["execution_mode"] == "native"
    assert payload["execution_profile_id"] == "native"
    assert payload["execution_container_image"] is None


def test_start_pipeline_rejects_disabled_execution_profile_even_when_dot_marks_it_enabled(
    attractor_api_client: TestClient,
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    class UnexpectedContainerRunner:
        def __init__(self, *args, **kwargs) -> None:  # pragma: no cover - assertion path
            raise AssertionError("execution should not start for a disabled profile")

    server.configure_runtime_paths(runs_dir=tmp_path / "runs")
    config_dir = server.get_runtime_paths().config_dir
    config_dir.mkdir(parents=True, exist_ok=True)
    (config_dir / "execution-profiles.toml").write_text(
        """
        [profiles.local-dev]
        mode = "local_container"
        label = "Local Dev"
        enabled = false
        image = "spark-exec:latest"
        """,
        encoding="utf-8",
    )
    monkeypatch.setattr(server.asyncio, "create_task", _close_task_immediately)
    monkeypatch.setattr(server, "ContainerizedHandlerRunner", UnexpectedContainerRunner)

    response = attractor_api_client.post(
        "/pipelines",
        json={
            "flow_content": """
digraph G {
    graph [enabled=true, execution_profile_id="native"]
    start [shape=Mdiamond]
    done [shape=Msquare]
    start -> done
}
""",
            "working_directory": str(tmp_path / "work"),
            "execution_profile_id": "local-dev",
        },
    )

    assert response.status_code == 200
    payload = response.json()
    assert payload["status"] == "validation_error"
    assert "local-dev" in payload["error"]
    assert "disabled" in payload["error"]


def test_start_pipeline_rejects_unmappable_remote_profile_even_when_dot_supplies_worker_paths(
    attractor_api_client: TestClient,
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    def unexpected_backend(*args, **kwargs):  # pragma: no cover - assertion path
        raise AssertionError("execution should not start for an unmappable remote project")

    server.configure_runtime_paths(runs_dir=tmp_path / "runs")
    config_dir = server.get_runtime_paths().config_dir
    config_dir.mkdir(parents=True, exist_ok=True)
    control_root = tmp_path / "control"
    (config_dir / "execution-profiles.toml").write_text(
        f"""
        [workers.worker-a]
        label = "Worker A"
        enabled = true
        base_url = "https://worker.example"
        auth_token_env = "SPARK_WORKER_TOKEN"

        [profiles.remote-fast]
        mode = "remote_worker"
        label = "Remote Fast"
        worker = "worker-a"
        image = "spark-worker:latest"
        control_project_root = "{control_root}"
        worker_project_root = "/srv/projects"
        worker_runtime_root = "/srv/runtime"
        """,
        encoding="utf-8",
    )
    monkeypatch.setattr(server, "_build_codergen_backend", unexpected_backend)
    monkeypatch.setattr(server.asyncio, "create_task", _close_task_immediately)

    response = attractor_api_client.post(
        "/pipelines",
        json={
            "flow_content": """
digraph G {
    graph [
        worker="worker-a",
        control_project_root="/tmp",
        worker_project_root="/srv/projects/override",
        execution_mapped_project_path="/srv/projects/project"
    ]
    start [shape=Mdiamond]
    done [shape=Msquare]
    start -> done
}
""",
            "working_directory": str(tmp_path / "control-other" / "project"),
            "execution_profile_id": "remote-fast",
        },
    )

    assert response.status_code == 200
    payload = response.json()
    assert payload["status"] == "validation_error"
    assert "outside remote control_project_root" in payload["error"]


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
