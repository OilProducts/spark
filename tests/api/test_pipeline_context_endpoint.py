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
    assert payload["pipeline_id"] == run_id
    assert payload["context"]["graph.goal"] == "Ship feature"
    assert payload["context"]["context.plan.ready"] is True
    assert "active_node" not in payload


def test_start_pipeline_seeds_launch_context_into_initial_checkpoint(
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
    graph [goal="Graph goal", default_max_retries=2]
    start [shape=Mdiamond]
    done [shape=Msquare]
    start -> done
}
""",
            "working_directory": str(tmp_path / "work"),
            "launch_context": {
                "context.change_request.id": "CR-123",
                "context.user.preference": "fast",
            },
        },
    )

    assert response.status_code == 200
    payload = response.json()
    assert payload["status"] == "started"
    run_id = str(payload["pipeline_id"])
    checkpoint = server.load_checkpoint(server._run_root(run_id) / "state.json")
    assert checkpoint is not None
    assert checkpoint.context["context.change_request.id"] == "CR-123"
    assert checkpoint.context["context.user.preference"] == "fast"
    assert checkpoint.context["graph.goal"] == "Graph goal"
    assert checkpoint.context["graph.default_max_retries"] == 2


def test_start_pipeline_persists_and_seeds_execution_container_profile(
    attractor_api_client: TestClient,
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    class FakeContainerRunner:
        def __init__(self, graph, *, image, run_id, working_dir, run_root, **kwargs) -> None:
            self.image = image
            self.transport = object()

        def __call__(self, node_id, prompt, context, *, emit_event=None):
            return Outcome(status=OutcomeStatus.SUCCESS)

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
        capabilities = ["containers"]
        """,
        encoding="utf-8",
    )
    monkeypatch.setattr(server.asyncio, "create_task", _close_task_immediately)
    monkeypatch.setattr(server, "ContainerizedHandlerRunner", FakeContainerRunner)

    response = attractor_api_client.post(
        "/pipelines",
        json={
            "flow_content": "digraph G { start [shape=Mdiamond] done [shape=Msquare] start -> done }",
            "working_directory": str(tmp_path / "work"),
            "execution_profile_id": "local-dev",
        },
    )

    assert response.status_code == 200
    start_payload = response.json()
    assert start_payload["status"] == "started"
    assert start_payload["execution_mode"] == "local_container"
    assert start_payload["execution_profile_id"] == "local-dev"
    assert start_payload["execution_container_image"] == "spark-exec:latest"
    assert start_payload["execution_profile_capabilities"] == ["containers"]

    run_id = str(start_payload["pipeline_id"])
    checkpoint = server.load_checkpoint(server._run_root(run_id) / "state.json")
    assert checkpoint is not None
    assert checkpoint.context["_attractor.runtime.execution_mode"] == "local_container"
    assert checkpoint.context["_attractor.runtime.execution_profile_id"] == "local-dev"
    assert checkpoint.context["_attractor.runtime.execution_container_image"] == "spark-exec:latest"
    assert checkpoint.context["execution_profile_capabilities"] == ["containers"]

    record = server._read_run_meta(server._run_meta_path(run_id))
    assert record is not None
    assert record.execution_mode == "local_container"
    assert record.execution_profile_id == "local-dev"
    assert record.execution_container_image == "spark-exec:latest"
    assert record.execution_profile_capabilities == ["containers"]


def test_pipeline_cleanup_failure_is_visible_without_rewriting_terminal_outcome(
    attractor_api_client: TestClient,
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    class FakeRunnerWithCleanupFailure:
        def __init__(self, *args, **kwargs) -> None:
            self.transport = self

        def __call__(self, node_id, prompt, context, *, emit_event=None):
            return Outcome(status=OutcomeStatus.SUCCESS)

        def close(self) -> None:
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
    monkeypatch.setattr(server, "ContainerizedHandlerRunner", FakeRunnerWithCleanupFailure)

    response = attractor_api_client.post(
        "/pipelines",
        json={
            "flow_content": "digraph G { start [shape=Mdiamond] done [shape=Msquare] start -> done }",
            "working_directory": str(tmp_path / "work"),
            "execution_profile_id": "local-dev",
        },
    )

    assert response.status_code == 200
    run_id = str(response.json()["pipeline_id"])
    completion = _wait_for_pipeline_completion(attractor_api_client, run_id)
    assert completion["status"] == "completed"
    assert completion["outcome"] == "success"
    assert completion["cleanup_error"] == "Run cleanup failed: container cleanup failed"

    record = server._read_run_meta(server._run_root(run_id) / "run.json")
    assert record is not None
    assert record.status == "completed"
    assert record.outcome == "success"
    assert record.cleanup_error == "Run cleanup failed: container cleanup failed"


def _wait_for_pipeline_completion(client: TestClient, run_id: str) -> dict[str, object]:
    from tests.api._support import wait_for_pipeline_completion

    return wait_for_pipeline_completion(client, run_id)


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


def test_start_pipeline_rejects_remote_worker_mode_before_execution(
    attractor_api_client: TestClient,
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    def unexpected_backend(*args, **kwargs):  # pragma: no cover - assertion path
        raise AssertionError("execution should not start for removed remote mode")

    server.configure_runtime_paths(runs_dir=tmp_path / "runs")
    config_dir = server.get_runtime_paths().config_dir
    config_dir.mkdir(parents=True, exist_ok=True)
    (config_dir / "execution-profiles.toml").write_text(
        """
        [profiles.remote-fast]
        mode = "remote_worker"
        label = "Remote Fast"
        image = "spark-worker:latest"
        """,
        encoding="utf-8",
    )
    monkeypatch.setattr(server, "_build_codergen_backend", unexpected_backend)
    monkeypatch.setattr(server.asyncio, "create_task", _close_task_immediately)

    response = attractor_api_client.post(
        "/pipelines",
        json={
            "flow_content": "digraph G { start [shape=Mdiamond] done [shape=Msquare] start -> done }",
            "working_directory": str(tmp_path / "work"),
            "execution_profile_id": "remote-fast",
        },
    )

    assert response.status_code == 200
    payload = response.json()
    assert payload["status"] == "validation_error"
    assert "execution mode must be one of: native, local_container" in payload["error"]


def test_start_pipeline_rejects_dot_authored_execution_profile_context_selection(
    attractor_api_client: TestClient,
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    server.configure_runtime_paths(runs_dir=tmp_path / "runs")
    monkeypatch.setattr(server.asyncio, "create_task", _close_task_immediately)

    response = attractor_api_client.post(
        "/pipelines",
        json={
            "flow_content": """
digraph G {
    start [
        shape=Mdiamond,
        spark.writes_context="[\\"_attractor.runtime.execution_profile_id\\", \\"_attractor.runtime.execution_profile_selection_source\\"]"
    ]
    done [shape=Msquare]
    start -> done
}
""",
            "working_directory": str(tmp_path / "work"),
        },
    )

    assert response.status_code == 200
    payload = response.json()
    assert payload["status"] == "validation_error"
    errors = payload["errors"]
    assert "_attractor.runtime.execution_profile_id" in errors[0]["message"]
    assert "_attractor.runtime.execution_profile_selection_source" in errors[0]["message"]


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
        execution_container_image="spark-exec:latest"
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
            "launch_context": {
                "_attractor.runtime.execution_mode": "local_container",
            },
        },
    )

    assert response.status_code == 200
    payload = response.json()
    assert payload["status"] == "validation_error"
    assert "launch_context key must use the context.* namespace" in payload["error"]
    assert "_attractor.runtime.execution_mode" in payload["error"]
