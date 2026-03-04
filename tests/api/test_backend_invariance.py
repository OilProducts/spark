from __future__ import annotations

from pathlib import Path
import time
from typing import List

import pytest
from fastapi.testclient import TestClient

import attractor.api.server as server
from attractor.engine import Context, load_checkpoint
from attractor.engine.outcome import Outcome, OutcomeStatus


FLOW = """
digraph G {
    start [shape=Mdiamond]
    done [shape=Msquare]
    start -> done
}
"""


def _close_task_immediately(coro):
    coro.close()

    class _DummyTask:
        pass

    return _DummyTask()


def _start_pipeline_via_http(api_client: TestClient, payload: dict) -> dict:
    response = api_client.post("/pipelines", json=payload)
    assert response.status_code == 200
    return response.json()


def _wait_for_pipeline_completion(api_client: TestClient, pipeline_id: str) -> dict:
    for _ in range(400):
        response = api_client.get(f"/pipelines/{pipeline_id}")
        assert response.status_code == 200
        payload = response.json()
        if payload["status"] != "running":
            return payload
        time.sleep(0.01)
    raise AssertionError("timed out waiting for pipeline completion")


def test_pipeline_start_request_accepts_dot_source_alias(
    api_client: TestClient,
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    monkeypatch.setattr(server, "RUNS_ROOT", tmp_path / "runs")
    monkeypatch.setattr(server.asyncio, "create_task", _close_task_immediately)

    payload = _start_pipeline_via_http(
        api_client,
        {
            "dot_source": FLOW,
            "working_directory": str(tmp_path / "work"),
            "backend": "codex",
        },
    )
    assert payload["status"] == "started"


@pytest.mark.parametrize("backend", ["codex", "codex-cli"])
def test_pipeline_definition_is_backend_invariant_for_backend_selection(
    api_client: TestClient,
    backend: str,
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    monkeypatch.setattr(server, "RUNS_ROOT", tmp_path / "runs")
    monkeypatch.setattr(server.asyncio, "create_task", _close_task_immediately)

    payload = _start_pipeline_via_http(
        api_client,
        {
            "flow_content": FLOW,
            "working_directory": str(tmp_path / "work"),
            "backend": backend,
        },
    )
    assert payload["status"] == "started"
    assert payload["working_directory"] == str((tmp_path / "work").resolve())


def test_pipeline_emits_lifecycle_phases_in_spec_order(
    api_client: TestClient,
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    monkeypatch.setattr(server, "RUNS_ROOT", tmp_path / "runs")

    payload = _start_pipeline_via_http(
        api_client,
        {
            "flow_content": FLOW,
            "working_directory": str(tmp_path / "work"),
            "backend": "codex",
        },
    )
    assert payload["status"] == "started"
    run_id = payload["run_id"]
    _wait_for_pipeline_completion(api_client, run_id)

    lifecycle_phases = [
        str(event.get("phase"))
        for event in server.EVENT_HUB.history(run_id)
        if event.get("type") == "lifecycle"
    ]

    assert lifecycle_phases == ["PARSE", "VALIDATE", "INITIALIZE", "EXECUTE", "FINALIZE"]


def test_pipeline_stream_includes_executor_typed_runtime_events(
    api_client: TestClient,
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    monkeypatch.setattr(server, "RUNS_ROOT", tmp_path / "runs")

    payload = _start_pipeline_via_http(
        api_client,
        {
            "flow_content": FLOW,
            "working_directory": str(tmp_path / "work"),
            "backend": "codex",
        },
    )
    assert payload["status"] == "started"
    run_id = payload["run_id"]
    _wait_for_pipeline_completion(api_client, run_id)

    event_types = [str(event.get("type")) for event in server.EVENT_HUB.history(run_id)]

    assert "PipelineStarted" in event_types
    assert "StageStarted" in event_types
    assert "StageCompleted" in event_types
    assert "CheckpointSaved" in event_types
    assert "PipelineCompleted" in event_types


def test_initialize_creates_run_dir_and_seed_checkpoint_with_transformed_graph(
    api_client: TestClient,
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    monkeypatch.setattr(server, "RUNS_ROOT", tmp_path / "runs")
    monkeypatch.setattr(server.asyncio, "create_task", _close_task_immediately)

    flow = """
    digraph G {
        graph [goal="Ship API", default_max_retry=7, default_fidelity="compact", model_stylesheet=".fast { llm_model: fast-model; }"]
        start [shape=Mdiamond]
        plan [shape=box, class="fast", prompt="Plan for $goal"]
        done [shape=Msquare]
        start -> plan -> done
    }
    """

    payload = _start_pipeline_via_http(
        api_client,
        {
            "flow_content": flow,
            "working_directory": str(tmp_path / "work"),
            "backend": "codex",
        },
    )
    assert payload["status"] == "started"
    run_id = payload["run_id"]
    run_root = tmp_path / "runs" / run_id
    assert run_root.exists()

    checkpoint = load_checkpoint(run_root / "state.json")
    assert checkpoint is not None
    assert checkpoint.current_node == "start"
    assert checkpoint.completed_nodes == []
    assert checkpoint.context["graph.goal"] == "Ship API"
    assert checkpoint.context["graph.default_max_retry"] == 7
    assert checkpoint.context["graph.default_fidelity"] == "compact"

    history = server.EVENT_HUB.history(run_id)
    lifecycle_phases = [
        str(event.get("phase"))
        for event in history
        if event.get("type") == "lifecycle"
    ]
    assert lifecycle_phases == ["PARSE", "VALIDATE", "INITIALIZE"]

    graph_event = next(event for event in history if event.get("type") == "graph")
    nodes_by_id = {str(node["id"]): node for node in graph_event["nodes"]}
    assert nodes_by_id["plan"]["prompt"] == "Plan for Ship API"
    assert nodes_by_id["plan"]["llm_model"] == "fast-model"


@pytest.mark.parametrize(
    ("backend_name", "expected_type"),
    [
        ("codex", server.LocalCodexAppServerBackend),
        ("codex_app_server", server.LocalCodexAppServerBackend),
        ("codex-cli", server.LocalCodexCliBackend),
    ],
)
def test_backend_factory_builds_multiple_implementations(
    backend_name: str, expected_type: type[object], tmp_path: Path
) -> None:
    events: List[dict] = []

    backend = server._build_codergen_backend(
        backend_name,
        str(tmp_path),
        events.append,
        model=None,
    )

    assert isinstance(backend, expected_type)


def test_local_codex_cli_backend_missing_binary_returns_fail_outcome_and_emits_log(
    monkeypatch: pytest.MonkeyPatch, tmp_path: Path
) -> None:
    events: List[dict] = []
    backend = server.LocalCodexCliBackend(str(tmp_path), events.append, model=None)

    def _raise_missing(*args, **kwargs):
        raise FileNotFoundError("codex")

    monkeypatch.setattr(server.subprocess, "run", _raise_missing)

    result = backend.run("plan", "hello", Context())

    assert isinstance(result, Outcome)
    assert result.status == OutcomeStatus.FAIL
    assert result.failure_reason == "codex executable not found on PATH"
    assert events[-1] == {"type": "log", "msg": "[plan] codex executable not found on PATH"}


def test_local_codex_app_server_backend_missing_binary_returns_fail_outcome_and_emits_log(
    monkeypatch: pytest.MonkeyPatch, tmp_path: Path
) -> None:
    events: List[dict] = []
    backend = server.LocalCodexAppServerBackend(str(tmp_path), events.append, model=None)

    def _raise_missing(*args, **kwargs):
        raise FileNotFoundError("codex")

    monkeypatch.setattr(server.subprocess, "Popen", _raise_missing)

    result = backend.run("plan", "hello", Context())

    assert isinstance(result, Outcome)
    assert result.status == OutcomeStatus.FAIL
    assert result.failure_reason == "codex app-server not found on PATH"
    assert events[-1] == {"type": "log", "msg": "[plan] codex app-server not found on PATH"}


def test_local_codex_app_server_backend_reuses_session_for_same_thread_key(tmp_path: Path) -> None:
    backend = server.LocalCodexAppServerBackend(str(tmp_path), lambda event: None, model=None)
    created: list[str] = []

    def _start_thread() -> str:
        thread_id = f"thread-{len(created) + 1}"
        created.append(thread_id)
        return thread_id

    first = backend._resolve_session_thread_id("loop-a", _start_thread)
    second = backend._resolve_session_thread_id("loop-a", _start_thread)

    assert first == "thread-1"
    assert second == "thread-1"
    assert created == ["thread-1"]


def test_local_codex_app_server_backend_isolates_sessions_for_different_thread_keys(
    tmp_path: Path,
) -> None:
    backend = server.LocalCodexAppServerBackend(str(tmp_path), lambda event: None, model=None)
    created: list[str] = []

    def _start_thread() -> str:
        thread_id = f"thread-{len(created) + 1}"
        created.append(thread_id)
        return thread_id

    first = backend._resolve_session_thread_id("loop-a", _start_thread)
    second = backend._resolve_session_thread_id("loop-b", _start_thread)

    assert first == "thread-1"
    assert second == "thread-2"
    assert created == ["thread-1", "thread-2"]


def test_local_codex_app_server_backend_does_not_cache_empty_thread_key(tmp_path: Path) -> None:
    backend = server.LocalCodexAppServerBackend(str(tmp_path), lambda event: None, model=None)
    created: list[str] = []

    def _start_thread() -> str:
        thread_id = f"thread-{len(created) + 1}"
        created.append(thread_id)
        return thread_id

    first = backend._resolve_session_thread_id("", _start_thread)
    second = backend._resolve_session_thread_id("", _start_thread)

    assert first == "thread-1"
    assert second == "thread-2"
    assert created == ["thread-1", "thread-2"]
