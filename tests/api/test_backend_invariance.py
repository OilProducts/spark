from __future__ import annotations

import asyncio
from pathlib import Path
from typing import List

import pytest

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


@pytest.mark.parametrize("backend", ["codex", "codex-cli"])
def test_pipeline_definition_is_backend_invariant_for_backend_selection(
    backend: str, monkeypatch: pytest.MonkeyPatch, tmp_path: Path
) -> None:
    monkeypatch.setattr(server, "RUNS_ROOT", tmp_path / "runs")
    monkeypatch.setattr(server.asyncio, "create_task", _close_task_immediately)

    payload = asyncio.run(
        server._start_pipeline(
            server.PipelineStartRequest(
                flow_content=FLOW,
                working_directory=str(tmp_path / "work"),
                backend=backend,
            )
        )
    )
    assert payload["status"] == "started"
    assert payload["working_directory"] == str((tmp_path / "work").resolve())

    pipeline_id = payload["pipeline_id"]
    server._pop_active_run(pipeline_id)


def test_pipeline_emits_lifecycle_phases_in_spec_order(
    monkeypatch: pytest.MonkeyPatch, tmp_path: Path
) -> None:
    monkeypatch.setattr(server, "RUNS_ROOT", tmp_path / "runs")

    async def _start_and_wait() -> str:
        payload = await server._start_pipeline(
            server.PipelineStartRequest(
                flow_content=FLOW,
                working_directory=str(tmp_path / "work"),
                backend="codex",
            )
        )
        assert payload["status"] == "started"
        run_id = payload["run_id"]

        for _ in range(400):
            record = server._read_run_meta(server._run_meta_path(run_id))
            if record and record.status != "running":
                break
            await asyncio.sleep(0.01)
        else:
            raise AssertionError("timed out waiting for pipeline completion")

        return run_id

    run_id = asyncio.run(_start_and_wait())
    lifecycle_phases = [
        str(event.get("phase"))
        for event in server.EVENT_HUB.history(run_id)
        if event.get("type") == "lifecycle"
    ]

    assert lifecycle_phases == ["PARSE", "VALIDATE", "INITIALIZE", "EXECUTE", "FINALIZE"]


def test_initialize_creates_run_dir_and_seed_checkpoint_with_transformed_graph(
    monkeypatch: pytest.MonkeyPatch, tmp_path: Path
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

    payload = asyncio.run(
        server._start_pipeline(
            server.PipelineStartRequest(
                flow_content=flow,
                working_directory=str(tmp_path / "work"),
                backend="codex",
            )
        )
    )
    assert payload["status"] == "started"
    run_id = payload["run_id"]
    run_root = server._run_root(run_id)
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

    server._pop_active_run(run_id)


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
