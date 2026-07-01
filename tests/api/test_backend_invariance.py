from __future__ import annotations

from contextlib import nullcontext
import inspect
import json
import itertools
from pathlib import Path
import threading
from spark_common.turn_stream import TurnStreamEvent, TurnStreamSource
from types import SimpleNamespace
from typing import List

import pytest
from fastapi.testclient import TestClient

import attractor.api.codex_backends as codex_backends_module
import attractor.api.server as server
from attractor.engine import Context, load_checkpoint
from attractor.engine.context_contracts import ContextWriteContract
from attractor.engine.outcome import FailureKind, Outcome, OutcomeStatus
from attractor.handlers.base import ChildInterventionRequest, ChildInterventionResult
from spark_common.project_identity import build_project_id
from spark_common.runtime_path import resolve_runtime_workspace_path
from tests.api._support import (
    SIMPLE_FLOW as FLOW,
    close_task_immediately as _close_task_immediately,
    wait_for_pipeline_completion as _wait_for_pipeline_completion,
)


def _start_pipeline_via_http(attractor_api_client: TestClient, payload: dict) -> dict:
    response = attractor_api_client.post("/pipelines", json=payload)
    assert response.status_code == 200
    return response.json()


class _BackendRunAdapter:
    def __init_subclass__(cls) -> None:
        super().__init_subclass__()
        if "run" not in cls.__dict__:
            return
        original_run = cls.run

        def run(self, node_id, prompt, context, *, emit_event=None, **kwargs):  # type: ignore[no-untyped-def]
            del emit_event
            parameters = inspect.signature(original_run).parameters
            if not any(parameter.kind == inspect.Parameter.VAR_KEYWORD for parameter in parameters.values()):
                kwargs = {key: value for key, value in kwargs.items() if key in parameters}
            return original_run(self, node_id, prompt, context, **kwargs)

        cls.run = run  # type: ignore[method-assign]


def test_pipeline_start_request_requires_flow_content_or_flow_name(
    attractor_api_client: TestClient,
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    monkeypatch.setattr(server.asyncio, "create_task", _close_task_immediately)

    payload = _start_pipeline_via_http(
        attractor_api_client,
        {
            "dot_source": FLOW,
            "working_directory": str(tmp_path / "work"),
            "backend": "codex-app-server",
        },
    )
    assert payload == {
        "status": "validation_error",
        "error": "Either flow_content or flow_name is required.",
    }


@pytest.mark.parametrize("backend", ["codex-app-server"])
def test_pipeline_definition_is_backend_invariant_for_backend_selection(
    attractor_api_client: TestClient,
    backend: str,
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    server.configure_runtime_paths(runs_dir=tmp_path / "runs")
    monkeypatch.setattr(server.asyncio, "create_task", _close_task_immediately)

    payload = _start_pipeline_via_http(
        attractor_api_client,
        {
            "flow_content": FLOW,
            "working_directory": str(tmp_path / "work"),
            "backend": backend,
        },
    )
    assert payload["status"] == "started"
    assert payload["working_directory"] == str((tmp_path / "work").resolve())


def test_pipeline_start_uses_flow_ui_default_model_when_request_model_missing(
    attractor_api_client: TestClient,
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    server.configure_runtime_paths(runs_dir=tmp_path / "runs")
    monkeypatch.setattr(server.asyncio, "create_task", _close_task_immediately)

    selected_models: list[str | None] = []

    class _Backend(_BackendRunAdapter):
        def run(  # type: ignore[no-untyped-def]
            self,
            node_id,
            prompt,
            context,
            *,
            response_contract="",
            contract_repair_attempts=0,
            timeout=None,
            model=None,
            write_contract=None,
        ):
            del node_id, prompt, context, response_contract, contract_repair_attempts, timeout, model, write_contract
            return ""

    def fake_build_backend(backend_name, working_dir, emit, *, model, on_usage_update=None):  # type: ignore[no-untyped-def]
        del backend_name, working_dir, emit, on_usage_update
        selected_models.append(model)
        return _Backend()

    monkeypatch.setattr(server, "_build_codergen_backend", fake_build_backend)

    payload = _start_pipeline_via_http(
        attractor_api_client,
        {
            "flow_content": """
            digraph G {
                graph [ui_default_llm_model="gpt-flow-default"]
                start [shape=Mdiamond]
                done [shape=Msquare]
                start -> done
            }
            """,
            "working_directory": str(tmp_path / "work"),
            "backend": "codex-app-server",
        },
    )

    assert payload["status"] == "started"
    assert payload["model"] == "gpt-flow-default"
    assert selected_models == ["gpt-flow-default"]
    record = server._read_run_meta(server._run_meta_path(payload["run_id"]))
    assert record is not None
    assert record.model == "gpt-flow-default"


def test_pipeline_start_explicit_model_overrides_flow_ui_default_model(
    attractor_api_client: TestClient,
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    server.configure_runtime_paths(runs_dir=tmp_path / "runs")
    monkeypatch.setattr(server.asyncio, "create_task", _close_task_immediately)

    selected_models: list[str | None] = []

    class _Backend(_BackendRunAdapter):
        def run(  # type: ignore[no-untyped-def]
            self,
            node_id,
            prompt,
            context,
            *,
            response_contract="",
            contract_repair_attempts=0,
            timeout=None,
            model=None,
            write_contract=None,
        ):
            del node_id, prompt, context, response_contract, contract_repair_attempts, timeout, model, write_contract
            return ""

    def fake_build_backend(backend_name, working_dir, emit, *, model, on_usage_update=None):  # type: ignore[no-untyped-def]
        del backend_name, working_dir, emit, on_usage_update
        selected_models.append(model)
        return _Backend()

    monkeypatch.setattr(server, "_build_codergen_backend", fake_build_backend)

    payload = _start_pipeline_via_http(
        attractor_api_client,
        {
            "flow_content": """
            digraph G {
                graph [ui_default_llm_model="gpt-flow-default"]
                start [shape=Mdiamond]
                done [shape=Msquare]
                start -> done
            }
            """,
            "working_directory": str(tmp_path / "work"),
            "backend": "codex-app-server",
            "model": "gpt-explicit",
        },
    )

    assert payload["status"] == "started"
    assert payload["model"] == "gpt-explicit"
    assert selected_models == ["gpt-explicit"]
    record = server._read_run_meta(server._run_meta_path(payload["run_id"]))
    assert record is not None
    assert record.model == "gpt-explicit"


@pytest.mark.parametrize(
    ("flow_content", "expected_model"),
    [
        (
            """
            digraph G {
                start [shape=Mdiamond]
                task [shape=box, llm_model="gpt-node-override"]
                done [shape=Msquare]
                start -> task
                task -> done
            }
            """,
            "gpt-node-override",
        ),
        (
            """
            digraph G {
                graph [model_stylesheet="box { llm_model: gpt-style-override; }"]
                start [shape=Mdiamond]
                task [shape=box]
                done [shape=Msquare]
                start -> task
                task -> done
            }
            """,
            "gpt-style-override",
        ),
        (
            """
            digraph G {
                start [shape=Mdiamond]
                task [shape=box]
                done [shape=Msquare]
                start -> task
                task -> done
            }
            """,
            "gpt-launch-default",
        ),
    ],
)
def test_pipeline_execution_resolves_effective_model_per_node(
    attractor_api_client: TestClient,
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
    flow_content: str,
    expected_model: str,
) -> None:
    server.configure_runtime_paths(runs_dir=tmp_path / "runs")

    build_models: list[str | None] = []
    run_models: list[str | None] = []

    class _Backend(_BackendRunAdapter):
        def run(  # type: ignore[no-untyped-def]
            self,
            node_id,
            prompt,
            context,
            *,
            response_contract="",
            contract_repair_attempts=0,
            timeout=None,
            model=None,
            write_contract=None,
        ):
            del node_id, prompt, context, response_contract, contract_repair_attempts, timeout, write_contract
            run_models.append(model)
            return ""

    def fake_build_backend(backend_name, working_dir, emit, *, model, on_usage_update=None):  # type: ignore[no-untyped-def]
        del backend_name, working_dir, emit, on_usage_update
        build_models.append(model)
        return _Backend()

    monkeypatch.setattr(server, "_build_codergen_backend", fake_build_backend)

    payload = _start_pipeline_via_http(
        attractor_api_client,
        {
            "flow_content": flow_content,
            "working_directory": str(tmp_path / "work"),
            "backend": "codex-app-server",
            "model": "gpt-launch-default",
        },
    )

    assert payload["status"] == "started"
    _wait_for_pipeline_completion(attractor_api_client, payload["run_id"])

    assert build_models == ["gpt-launch-default"]
    assert run_models == [expected_model]


def test_pipeline_emits_lifecycle_phases_in_spec_order(
    attractor_api_client: TestClient,
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    server.configure_runtime_paths(runs_dir=tmp_path / "runs")

    payload = _start_pipeline_via_http(
        attractor_api_client,
        {
            "flow_content": FLOW,
            "working_directory": str(tmp_path / "work"),
            "backend": "codex-app-server",
        },
    )
    assert payload["status"] == "started"
    run_id = payload["run_id"]
    _wait_for_pipeline_completion(attractor_api_client, run_id)

    lifecycle_phases = [
        str(event.get("phase"))
        for event in server.EVENT_HUB.history(run_id)
        if event.get("type") == "lifecycle"
    ]

    assert lifecycle_phases == ["PARSE", "TRANSFORM", "VALIDATE", "INITIALIZE", "EXECUTE", "FINALIZE"]


def test_pipeline_stream_includes_executor_typed_runtime_events(
    attractor_api_client: TestClient,
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    server.configure_runtime_paths(runs_dir=tmp_path / "runs")

    payload = _start_pipeline_via_http(
        attractor_api_client,
        {
            "flow_content": FLOW,
            "working_directory": str(tmp_path / "work"),
            "backend": "codex-app-server",
        },
    )
    assert payload["status"] == "started"
    run_id = payload["run_id"]
    _wait_for_pipeline_completion(attractor_api_client, run_id)

    event_types = [str(event.get("type")) for event in server.EVENT_HUB.history(run_id)]

    assert "PipelineStarted" in event_types
    assert "StageStarted" in event_types
    assert "StageCompleted" in event_types
    assert "CheckpointSaved" in event_types
    assert "PipelineCompleted" in event_types


@pytest.mark.anyio
async def test_pipeline_failure_preserves_last_error_and_emits_descriptive_terminal_summary(
    attractor_api_client: TestClient,
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    server.configure_runtime_paths(runs_dir=tmp_path / "runs")

    scheduled: list[object] = []

    class _Executor:
        def __init__(self, *args, **kwargs) -> None:  # type: ignore[no-untyped-def]
            del args, kwargs

        def run(self, context, resume=True):  # type: ignore[no-untyped-def]
            del context, resume
            return SimpleNamespace(
                status="failed",
                current_node="fail_work",
                completed_nodes=["start"],
                context={},
                node_outcomes={},
                route_trace=["start", "fail_work"],
                failure_reason="boom",
                outcome=None,
                outcome_reason_code=None,
                outcome_reason_message=None,
            )

    def _capture_task(coro):  # type: ignore[no-untyped-def]
        scheduled.append(coro)
        return SimpleNamespace()

    monkeypatch.setattr(server, "PipelineExecutor", _Executor)
    monkeypatch.setattr(server.asyncio, "create_task", _capture_task)

    run_id = "run-failure-summary"
    payload = await server._start_pipeline(
        server.PipelineStartRequest(
            flow_content="""
            digraph G {
                start [shape=Mdiamond]
                done [shape=Msquare]
                start -> done
            }
            """,
            working_directory=str(tmp_path / "work"),
            backend="codex-app-server",
        ),
        run_id=run_id,
    )
    assert payload["status"] == "started"
    assert len(scheduled) == 1

    await scheduled.pop()

    status_payload = await server.get_pipeline(run_id)
    assert status_payload["status"] == "failed"
    assert status_payload["last_error"] == "boom"

    history = server.EVENT_HUB.history(run_id)
    assert any(
        event.get("type") == "runtime"
        and event.get("status") == "failed"
        and event.get("outcome") is None
        and event.get("outcome_reason_code") is None
        and event.get("outcome_reason_message") is None
        and event.get("last_error") == "boom"
        and event.get("run_id") == run_id
        and isinstance(event.get("sequence"), int)
        and isinstance(event.get("emitted_at"), str)
        for event in history
    )
    assert any(
        event.get("type") == "log"
        and event.get("msg") == "Pipeline failed: boom"
        and event.get("run_id") == run_id
        and isinstance(event.get("sequence"), int)
        and isinstance(event.get("emitted_at"), str)
        for event in history
    )


def test_initialize_creates_run_dir_and_seed_checkpoint_with_transformed_graph(
    attractor_api_client: TestClient,
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    monkeypatch.setattr(server.asyncio, "create_task", _close_task_immediately)

    flow = """
    digraph G {
        graph [goal="Ship API", default_max_retries=7, default_fidelity="compact", model_stylesheet=".fast { llm_model: fast-model; }"]
        start [shape=Mdiamond]
        plan [shape=box, class="fast", prompt="Plan for $goal"]
        done [shape=Msquare]
        start -> plan -> done
    }
    """

    payload = _start_pipeline_via_http(
        attractor_api_client,
        {
            "flow_content": flow,
            "working_directory": str(tmp_path / "work"),
            "backend": "codex-app-server",
        },
    )
    assert payload["status"] == "started"
    run_id = payload["run_id"]
    run_root = server._run_root(run_id)
    assert run_root.exists()
    assert run_root == tmp_path / ".spark" / "attractor" / "runs" / build_project_id(str((tmp_path / "work").resolve())) / run_id

    checkpoint = load_checkpoint(run_root / "state.json")
    assert checkpoint is not None
    assert checkpoint.current_node == "start"
    assert checkpoint.completed_nodes == []
    assert checkpoint.context["graph.goal"] == "Ship API"
    assert checkpoint.context["graph.default_max_retries"] == 7
    assert checkpoint.context["graph.default_fidelity"] == "compact"

    history = server.EVENT_HUB.history(run_id)
    lifecycle_phases = [
        str(event.get("phase"))
        for event in history
        if event.get("type") == "lifecycle"
    ]
    assert lifecycle_phases == ["PARSE", "TRANSFORM", "VALIDATE", "INITIALIZE"]

    graph_event = next(event for event in history if event.get("type") == "graph")
    nodes_by_id = {str(node["id"]): node for node in graph_event["nodes"]}
    assert nodes_by_id["plan"]["prompt"] == "Plan for Ship API"
    assert nodes_by_id["plan"]["llm_model"] == "fast-model"


@pytest.mark.parametrize(
    ("backend_name", "expected_type"),
    [
        ("codex-app-server", server.CodexAppServerBackend),
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


@pytest.mark.parametrize("backend_name", ["codex", "codex_app_server", "codex-cli"])
def test_backend_factory_rejects_non_canonical_backend_names(backend_name: str, tmp_path: Path) -> None:
    with pytest.raises(
        ValueError,
        match="Unsupported backend. Supported backends: provider-router, codex-app-server.",
    ):
        server._build_codergen_backend(
            backend_name,
            str(tmp_path),
            lambda event: None,
            model=None,
        )


def test_codex_app_server_backend_missing_binary_returns_fail_outcome_and_emits_log(
    monkeypatch: pytest.MonkeyPatch, tmp_path: Path
) -> None:
    events: List[dict] = []
    backend = server.CodexAppServerBackend(str(tmp_path), events.append, model=None)

    def _raise_missing(*args, **kwargs):
        raise FileNotFoundError("codex")

    monkeypatch.setattr(codex_backends_module.subprocess, "Popen", _raise_missing)

    result = backend.run("plan", "hello", Context())

    assert isinstance(result, Outcome)
    assert result.status == OutcomeStatus.FAIL
    assert result.failure_reason == "codex app-server not found on PATH"
    assert result.failure_kind == FailureKind.RUNTIME
    assert result.retryable is False
    assert events[-1] == {"type": "log", "msg": "[plan] codex app-server not found on PATH"}


def test_resolve_runtime_workspace_path_maps_host_repo_root_override_to_runtime_repo_root(
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    runtime_repo_root = Path(server.__file__).resolve().parents[3]
    host_repo_root = tmp_path / "host-repo-root"
    monkeypatch.setenv("ATTRACTOR_HOST_REPO_ROOT", str(host_repo_root))
    monkeypatch.setenv("ATTRACTOR_RUNTIME_REPO_ROOT", str(runtime_repo_root))
    translated = resolve_runtime_workspace_path(str(host_repo_root / "frontend"))

    assert translated == str((runtime_repo_root / "frontend").resolve(strict=False))


def test_codex_app_server_backend_missing_runtime_working_directory_returns_specific_failure(
    monkeypatch: pytest.MonkeyPatch, tmp_path: Path
) -> None:
    missing_dir = tmp_path / "missing-workdir"
    events: List[dict] = []
    backend = server.CodexAppServerBackend(str(missing_dir), events.append, model=None)

    def _raise_missing(*args, **kwargs):
        raise FileNotFoundError(str(missing_dir))

    monkeypatch.setattr(codex_backends_module.subprocess, "Popen", _raise_missing)

    result = backend.run("plan", "hello", Context())

    assert isinstance(result, Outcome)
    assert result.status == OutcomeStatus.FAIL
    assert "working directory is unavailable in the runtime" in str(result.failure_reason)
    assert result.failure_kind == FailureKind.RUNTIME
    assert result.retryable is False
    assert str(missing_dir.resolve(strict=False)) in str(result.failure_reason)
    assert "working directory is unavailable in the runtime" in events[-1]["msg"]


def test_codex_app_server_backend_reuses_session_for_same_thread_key(tmp_path: Path) -> None:
    backend = server.CodexAppServerBackend(str(tmp_path), lambda event: None, model=None)
    created: list[str] = []

    def _start_thread() -> str:
        thread_id = f"thread-{len(created) + 1}"
        created.append(thread_id)
        return thread_id

    first = backend._resolve_session_thread_id("loop-a", "gpt-node", _start_thread)
    second = backend._resolve_session_thread_id("loop-a", "gpt-node", _start_thread)

    assert first == "thread-1"
    assert second == "thread-1"
    assert created == ["thread-1"]


def test_codex_app_server_backend_isolates_sessions_for_different_thread_keys(
    tmp_path: Path,
) -> None:
    backend = server.CodexAppServerBackend(str(tmp_path), lambda event: None, model=None)
    created: list[str] = []

    def _start_thread() -> str:
        thread_id = f"thread-{len(created) + 1}"
        created.append(thread_id)
        return thread_id

    first = backend._resolve_session_thread_id("loop-a", "gpt-node", _start_thread)
    second = backend._resolve_session_thread_id("loop-b", "gpt-node", _start_thread)

    assert first == "thread-1"
    assert second == "thread-2"
    assert created == ["thread-1", "thread-2"]


def test_codex_app_server_backend_does_not_cache_empty_thread_key(tmp_path: Path) -> None:
    backend = server.CodexAppServerBackend(str(tmp_path), lambda event: None, model=None)
    created: list[str] = []

    def _start_thread() -> str:
        thread_id = f"thread-{len(created) + 1}"
        created.append(thread_id)
        return thread_id

    first = backend._resolve_session_thread_id("", "gpt-node", _start_thread)
    second = backend._resolve_session_thread_id("", "gpt-node", _start_thread)

    assert first == "thread-1"
    assert second == "thread-2"
    assert created == ["thread-1", "thread-2"]


def test_codex_app_server_backend_isolates_sessions_for_different_models_on_same_thread_key(
    tmp_path: Path,
) -> None:
    backend = server.CodexAppServerBackend(str(tmp_path), lambda event: None, model=None)
    created: list[str] = []

    def _start_thread() -> str:
        thread_id = f"thread-{len(created) + 1}"
        created.append(thread_id)
        return thread_id

    first = backend._resolve_session_thread_id("loop-a", "gpt-fast", _start_thread)
    second = backend._resolve_session_thread_id("loop-a", "gpt-deep", _start_thread)

    assert first == "thread-1"
    assert second == "thread-2"
    assert created == ["thread-1", "thread-2"]


def test_codex_app_server_backend_drains_notifications_queued_during_turn_start_response(
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    events: List[dict] = []
    backend = server.CodexAppServerBackend(str(tmp_path), events.append, model=None)

    class FakeStdout:
        def __init__(self, lines: list[str]) -> None:
            self._lines = list(lines)

        def readline(self) -> str:
            if not self._lines:
                return ""
            return f"{self._lines.pop(0)}\n"

    class FakeStdin:
        def write(self, text: str) -> None:
            return None

        def flush(self) -> None:
            return None

    class FakeProcess:
        def __init__(self, lines: list[str]) -> None:
            self.stdout = FakeStdout(lines)
            self.stdin = FakeStdin()

        def poll(self) -> None:
            return None

        def terminate(self) -> None:
            return None

        def wait(self, timeout: float | None = None) -> None:
            return None

        def kill(self) -> None:
            return None

    lines = [
        '{"jsonrpc":"2.0","id":1,"result":{"capabilities":{"experimentalApi":true}}}',
        '{"jsonrpc":"2.0","id":2,"result":{"thread":{"id":"thread-123"}}}',
        '{"jsonrpc":"2.0","method":"item/agentMessage/delta","params":{"delta":"Ack"}}',
        '{"jsonrpc":"2.0","method":"item/completed","params":{"item":{"type":"AgentMessage","id":"msg-1","content":[{"type":"Text","text":"Ack"}],"phase":"final_answer"}}}',
        '{"jsonrpc":"2.0","id":3,"result":{"turn":{"id":"turn-123","status":"inProgress","items":[]}}}',
        '{"jsonrpc":"2.0","method":"turn/completed","params":{"turn":{"id":"turn-123","status":"completed"}}}',
    ]

    monkeypatch.setattr(codex_backends_module.subprocess, "Popen", lambda *args, **kwargs: FakeProcess(lines))

    result = backend.run("plan", "hello", Context())

    assert result == "Ack"
    assert {"type": "log", "msg": "[plan] Ack"} in events


def test_codex_app_server_backend_logs_token_usage(
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    events: List[dict] = []
    backend = server.CodexAppServerBackend(str(tmp_path), events.append, model=None)

    class FakeResult:
        assistant_message = "Ack"
        command_text = ""
        token_total = 321

    class FakeClient:
        def __init__(self, *args, **kwargs) -> None:
            return None

        def ensure_process(self, **kwargs) -> None:
            return None

        def start_thread(self, **kwargs) -> str:
            return "thread-123"

        def run_turn(self, **kwargs) -> FakeResult:
            return FakeResult()

        def close(self) -> None:
            return None

    monkeypatch.setattr(codex_backends_module, "CodexAppServerClient", FakeClient)

    result = backend.run("plan", "hello", Context())

    assert result == "Ack"
    assert {"type": "log", "msg": "[plan] Ack"} in events
    assert {"type": "log", "msg": "[plan] tokens used: 321"} in events


def test_codex_app_server_backend_accumulates_live_usage_by_model(
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    usage_snapshots: list[dict[str, object]] = []
    backend = server.CodexAppServerBackend(
        str(tmp_path),
        lambda event: None,
        model=None,
        on_usage_update=lambda snapshot: usage_snapshots.append(snapshot.to_dict()),
    )

    class FakeResult:
        def __init__(self, token_total: int) -> None:
            self.assistant_message = "Ack"
            self.command_text = ""
            self.token_total = token_total
            self.token_usage_payload = None

    class FakeClient:
        def __init__(self, *args, **kwargs) -> None:
            return None

        def ensure_process(self, **kwargs) -> None:
            return None

        def start_thread(self, **kwargs) -> str:
            return "thread-123"

        def run_turn(self, **kwargs) -> FakeResult:
            on_event = kwargs.get("on_event")
            model = kwargs.get("model")
            if model == "gpt-5.4":
                on_event(
                    codex_backends_module.TurnStreamEvent(
                        kind="token_usage_updated",
                        token_usage={
                            "last": {
                                "inputTokens": 10,
                                "cachedInputTokens": 2,
                                "outputTokens": 5,
                                "reasoningOutputTokens": 1,
                                "totalTokens": 15,
                            },
                            "total": {
                                "inputTokens": 10,
                                "cachedInputTokens": 2,
                                "outputTokens": 5,
                                "reasoningOutputTokens": 1,
                                "totalTokens": 15,
                            },
                        },
                    )
                )
                on_event(
                    codex_backends_module.TurnStreamEvent(
                        kind="token_usage_updated",
                        token_usage={
                            "total": {
                                "inputTokens": 15,
                                "cachedInputTokens": 3,
                                "outputTokens": 9,
                                "reasoningOutputTokens": 4,
                                "totalTokens": 24,
                            },
                        },
                    )
                )
                return FakeResult(token_total=24)
            on_event(
                codex_backends_module.TurnStreamEvent(
                    kind="token_usage_updated",
                    token_usage={
                        "last": {
                            "inputTokens": 8,
                            "cachedInputTokens": 0,
                            "outputTokens": 4,
                            "reasoningOutputTokens": 2,
                            "totalTokens": 12,
                        },
                        "total": {
                            "inputTokens": 8,
                            "cachedInputTokens": 0,
                            "outputTokens": 4,
                            "reasoningOutputTokens": 2,
                            "totalTokens": 12,
                        },
                    },
                )
            )
            return FakeResult(token_total=12)

        def close(self) -> None:
            return None

    monkeypatch.setattr(codex_backends_module, "CodexAppServerClient", FakeClient)

    assert backend.run("plan", "hello", Context(), model="gpt-5.4") == "Ack"
    assert backend.run("review", "hello", Context(), model="gpt-5.3-codex-spark") == "Ack"

    assert usage_snapshots[-1] == {
        "input_tokens": 23,
        "cached_input_tokens": 3,
        "output_tokens": 13,
        "total_tokens": 36,
        "by_model": {
            "gpt-5.3-codex-spark": {
                "input_tokens": 8,
                "cached_input_tokens": 0,
                "output_tokens": 4,
                "total_tokens": 12,
            },
            "gpt-5.4": {
                "input_tokens": 15,
                "cached_input_tokens": 3,
                "output_tokens": 9,
                "total_tokens": 24,
            },
        },
    }


def test_codex_app_server_backend_emits_llm_content_progress(
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    progress_events: list[dict[str, object]] = []
    backend = server.CodexAppServerBackend(str(tmp_path), lambda event: None, model=None)

    class FakeResult:
        assistant_message = "Final"
        command_text = ""
        token_total = None
        token_usage_payload = None

    class FakeClient:
        def __init__(self, *args, **kwargs) -> None:
            return None

        def ensure_process(self, **kwargs) -> None:
            return None

        def start_thread(self, **kwargs) -> str:
            return "thread-123"

        def run_turn(self, **kwargs) -> FakeResult:
            on_event = kwargs["on_event"]
            on_event(
                TurnStreamEvent(
                    kind="content_delta",
                    channel="assistant",
                    content_delta="Hello ",
                    phase="thinking",
                    source=TurnStreamSource(
                        backend="codex_app_server",
                        app_turn_id="turn-1",
                        item_id="msg-1",
                    ),
                )
            )
            on_event(
                TurnStreamEvent(
                    kind="content_completed",
                    channel="assistant",
                    content_delta="Hello world",
                    source=TurnStreamSource(
                        backend="codex_app_server",
                        app_turn_id="turn-1",
                        item_id="msg-1",
                    ),
                )
            )
            return FakeResult()

        def close(self) -> None:
            return None

    monkeypatch.setattr(codex_backends_module, "CodexAppServerClient", FakeClient)

    result = backend.run(
        "plan",
        "hello",
        Context(),
        emit_event=lambda event_type, **payload: progress_events.append({
            "type": event_type,
            **payload,
        }),
    )

    assert result == "Final"
    assert progress_events == [
        {
            "type": "LLMContent",
            "node_id": "plan",
            "channel": "assistant",
            "content_delta": "Hello ",
            "status": "streaming",
            "phase": "thinking",
            "source": {
                "backend": "codex_app_server",
                "app_turn_id": "turn-1",
                "item_id": "msg-1",
            },
        },
        {
            "type": "LLMContent",
            "node_id": "plan",
            "channel": "assistant",
            "content_delta": "Hello world",
            "status": "complete",
            "phase": None,
            "source": {
                "backend": "codex_app_server",
                "app_turn_id": "turn-1",
                "item_id": "msg-1",
            },
        },
    ]


def test_codex_app_server_backend_parses_structured_outcome_agent_text(
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    events: List[dict] = []
    backend = server.CodexAppServerBackend(str(tmp_path), events.append, model=None)

    class FakeStdout:
        def __init__(self, lines: list[str]) -> None:
            self._lines = list(lines)

        def readline(self) -> str:
            if not self._lines:
                return ""
            return f"{self._lines.pop(0)}\n"

    class FakeStdin:
        def write(self, text: str) -> None:
            return None

        def flush(self) -> None:
            return None

    class FakeProcess:
        def __init__(self, lines: list[str]) -> None:
            self.stdout = FakeStdout(lines)
            self.stdin = FakeStdin()

        def poll(self) -> None:
            return None

        def terminate(self) -> None:
            return None

        def wait(self, timeout: float | None = None) -> None:
            return None

        def kill(self) -> None:
            return None

    lines = [
        '{"jsonrpc":"2.0","id":1,"result":{"capabilities":{"experimentalApi":true}}}',
        '{"jsonrpc":"2.0","id":2,"result":{"thread":{"id":"thread-123"}}}',
        '{"jsonrpc":"2.0","id":3,"result":{"turn":{"id":"turn-123","status":"inProgress","items":[]}}}',
        '{"jsonrpc":"2.0","method":"item/agentMessage/delta","params":{"delta":"{\\"outcome\\":\\"fail\\",\\"notes\\":\\"needs fixes\\",\\"failure_reason\\":\\"review requested changes\\",\\"context_updates\\":{\\"context.review.summary\\":\\"missing validation\\"}}"}}',
        '{"jsonrpc":"2.0","method":"item/completed","params":{"item":{"type":"AgentMessage","id":"msg-1","content":[{"type":"Text","text":"{\\"outcome\\":\\"fail\\",\\"notes\\":\\"needs fixes\\",\\"failure_reason\\":\\"review requested changes\\",\\"context_updates\\":{\\"context.review.summary\\":\\"missing validation\\"}}"}],"phase":"final_answer"}}}',
        '{"jsonrpc":"2.0","method":"turn/completed","params":{"turn":{"id":"turn-123","status":"completed"}}}',
    ]

    monkeypatch.setattr(codex_backends_module.subprocess, "Popen", lambda *args, **kwargs: FakeProcess(lines))

    result = backend.run(
        "review",
        "hello",
        Context(),
        response_contract="status_envelope",
        contract_repair_attempts=1,
    )

    assert isinstance(result, Outcome)
    assert result.status == OutcomeStatus.FAIL
    assert result.notes == "needs fixes"
    assert result.failure_reason == "review requested changes"
    assert result.failure_kind == FailureKind.BUSINESS
    assert result.context_updates == {"context.review.summary": "missing validation"}


def test_codex_app_server_backend_treats_any_response_contract_fail_as_business_result(
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    events: List[dict] = []
    backend = server.CodexAppServerBackend(str(tmp_path), events.append, model=None)

    class FakeResult:
        def __init__(self, assistant_message: str) -> None:
            self.assistant_message = assistant_message
            self.command_text = ""
            self.token_total = None

    class FakeClient:
        def __init__(self, *args, **kwargs) -> None:
            return None

        def ensure_process(self, **kwargs) -> None:
            return None

        def start_thread(self, **kwargs) -> str:
            return "thread-123"

        def run_turn(self, **kwargs) -> FakeResult:
            del kwargs
            return FakeResult('{"outcome":"fail","notes":"needs fixes","failure_reason":"review requested changes"}')

        def close(self) -> None:
            return None

    monkeypatch.setattr(codex_backends_module, "CodexAppServerClient", FakeClient)

    result = backend.run(
        "review",
        "hello",
        Context(),
        response_contract="custom_contract",
        contract_repair_attempts=1,
    )

    assert isinstance(result, Outcome)
    assert result.status == OutcomeStatus.FAIL
    assert result.failure_kind == FailureKind.BUSINESS


def test_codex_app_server_backend_repairs_malformed_contract_output_on_same_thread(
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    events: List[dict] = []
    backend = server.CodexAppServerBackend(str(tmp_path), events.append, model=None)
    prompts: list[dict[str, object]] = []

    class FakeResult:
        def __init__(self, assistant_message: str, token_total: int | None = None) -> None:
            self.assistant_message = assistant_message
            self.command_text = ""
            self.token_total = token_total

    class FakeClient:
        def __init__(self, *args, **kwargs) -> None:
            return None

        def ensure_process(self, **kwargs) -> None:
            return None

        def start_thread(self, **kwargs) -> str:
            return "thread-123"

        def run_turn(self, **kwargs) -> FakeResult:
            prompts.append(kwargs)
            if len(prompts) == 1:
                return FakeResult('{"outcome":"success","notes":["bad"]}')
            return FakeResult('{"outcome":"success","notes":"corrected"}')

        def close(self) -> None:
            return None

    monkeypatch.setattr(codex_backends_module, "CodexAppServerClient", FakeClient)

    result = backend.run(
        "audit_milestone",
        "hello",
        Context(),
        response_contract="status_envelope",
        contract_repair_attempts=1,
    )

    assert isinstance(result, Outcome)
    assert result.status == OutcomeStatus.SUCCESS
    assert result.notes == "corrected"
    assert len(prompts) == 2
    assert prompts[0]["thread_id"] == "thread-123"
    assert prompts[1]["thread_id"] == "thread-123"
    assert prompts[0]["prompt"] == "hello"
    assert prompts[1]["prompt"] != prompts[0]["prompt"]


def test_codex_app_server_backend_repairs_malformed_output_for_any_response_contract(
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    events: List[dict] = []
    backend = server.CodexAppServerBackend(str(tmp_path), events.append, model=None)
    prompts: list[dict[str, object]] = []

    class FakeResult:
        def __init__(self, assistant_message: str) -> None:
            self.assistant_message = assistant_message
            self.command_text = ""
            self.token_total = None

    class FakeClient:
        def __init__(self, *args, **kwargs) -> None:
            return None

        def ensure_process(self, **kwargs) -> None:
            return None

        def start_thread(self, **kwargs) -> str:
            return "thread-123"

        def run_turn(self, **kwargs) -> FakeResult:
            prompts.append(kwargs)
            if len(prompts) == 1:
                return FakeResult('{"outcome":"success","notes":["bad"]}')
            return FakeResult('{"outcome":"success","notes":"corrected"}')

        def close(self) -> None:
            return None

    monkeypatch.setattr(codex_backends_module, "CodexAppServerClient", FakeClient)

    result = backend.run(
        "audit_milestone",
        "hello",
        Context(),
        response_contract="custom_contract",
        contract_repair_attempts=1,
    )

    assert isinstance(result, Outcome)
    assert result.status == OutcomeStatus.SUCCESS
    assert result.notes == "corrected"
    assert len(prompts) == 2
    assert prompts[0]["prompt"] == "hello"
    assert prompts[1]["prompt"] != prompts[0]["prompt"]


def test_codex_app_server_backend_repairs_undeclared_context_updates_on_same_thread(
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    events: List[dict] = []
    backend = server.CodexAppServerBackend(str(tmp_path), events.append, model=None)
    prompts: list[dict[str, object]] = []

    class FakeResult:
        def __init__(self, assistant_message: str) -> None:
            self.assistant_message = assistant_message
            self.command_text = ""
            self.token_total = None

    class FakeClient:
        def __init__(self, *args, **kwargs) -> None:
            return None

        def ensure_process(self, **kwargs) -> None:
            return None

        def start_thread(self, **kwargs) -> str:
            return "thread-123"

        def run_turn(self, **kwargs) -> FakeResult:
            prompts.append(kwargs)
            if len(prompts) == 1:
                return FakeResult(
                    '{"outcome":"success","context_updates":{"context.review.summary":"ready","context.review.extra":"nope"}}'
                )
            return FakeResult('{"outcome":"success","context_updates":{"context.review.summary":"ready"}}')

        def close(self) -> None:
            return None

    monkeypatch.setattr(codex_backends_module, "CodexAppServerClient", FakeClient)

    result = backend.run(
        "review",
        "hello",
        Context(),
        response_contract="status_envelope",
        contract_repair_attempts=1,
        write_contract=ContextWriteContract(allowed_keys=("context.review.summary",)),
    )

    assert isinstance(result, Outcome)
    assert result.status == OutcomeStatus.SUCCESS
    assert result.context_updates == {"context.review.summary": "ready"}
    assert len(prompts) == 2
    assert prompts[0]["thread_id"] == "thread-123"
    assert prompts[1]["thread_id"] == "thread-123"
    assert prompts[0]["prompt"] == "hello"
    assert prompts[1]["prompt"] != prompts[0]["prompt"]


def test_codex_app_server_backend_repairs_context_updates_when_node_declares_no_writes(
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    events: List[dict] = []
    backend = server.CodexAppServerBackend(str(tmp_path), events.append, model=None)
    prompts: list[dict[str, object]] = []

    class FakeResult:
        def __init__(self, assistant_message: str) -> None:
            self.assistant_message = assistant_message
            self.command_text = ""
            self.token_total = None

    class FakeClient:
        def __init__(self, *args, **kwargs) -> None:
            return None

        def ensure_process(self, **kwargs) -> None:
            return None

        def start_thread(self, **kwargs) -> str:
            return "thread-123"

        def run_turn(self, **kwargs) -> FakeResult:
            prompts.append(kwargs)
            if len(prompts) == 1:
                return FakeResult('{"outcome":"success","context_updates":{"context.review.summary":"ready"}}')
            return FakeResult('{"outcome":"success"}')

        def close(self) -> None:
            return None

    monkeypatch.setattr(codex_backends_module, "CodexAppServerClient", FakeClient)

    result = backend.run(
        "review",
        "hello",
        Context(),
        response_contract="status_envelope",
        contract_repair_attempts=1,
        write_contract=ContextWriteContract(),
    )

    assert isinstance(result, Outcome)
    assert result.status == OutcomeStatus.SUCCESS
    assert result.context_updates == {}
    assert len(prompts) == 2
    assert prompts[0]["thread_id"] == "thread-123"
    assert prompts[1]["thread_id"] == "thread-123"
    assert prompts[0]["prompt"] == "hello"
    assert prompts[1]["prompt"] != prompts[0]["prompt"]


def test_codex_app_server_backend_repairs_invalid_context_update_keys_on_same_thread(
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    events: List[dict] = []
    backend = server.CodexAppServerBackend(str(tmp_path), events.append, model=None)
    prompts: list[dict[str, object]] = []

    class FakeResult:
        def __init__(self, assistant_message: str) -> None:
            self.assistant_message = assistant_message
            self.command_text = ""
            self.token_total = None

    class FakeClient:
        def __init__(self, *args, **kwargs) -> None:
            return None

        def ensure_process(self, **kwargs) -> None:
            return None

        def start_thread(self, **kwargs) -> str:
            return "thread-123"

        def run_turn(self, **kwargs) -> FakeResult:
            prompts.append(kwargs)
            if len(prompts) == 1:
                return FakeResult(
                    '{"outcome":"success","context_updates":{"runtime/state.json":"nope"}}'
                )
            return FakeResult('{"outcome":"success","context_updates":{"context.review.summary":"ready"}}')

        def close(self) -> None:
            return None

    monkeypatch.setattr(codex_backends_module, "CodexAppServerClient", FakeClient)

    result = backend.run(
        "review",
        "hello",
        Context(),
        response_contract="status_envelope",
        contract_repair_attempts=1,
        write_contract=ContextWriteContract(allowed_keys=("context.review.summary",)),
    )

    assert isinstance(result, Outcome)
    assert result.status == OutcomeStatus.SUCCESS
    assert result.context_updates == {"context.review.summary": "ready"}
    assert len(prompts) == 2
    assert prompts[0]["thread_id"] == "thread-123"
    assert prompts[1]["thread_id"] == "thread-123"
    assert prompts[0]["prompt"] == "hello"
    assert prompts[1]["prompt"] != prompts[0]["prompt"]


def test_codex_app_server_backend_returns_contract_failure_when_repair_exhausted(
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    events: List[dict] = []
    backend = server.CodexAppServerBackend(str(tmp_path), events.append, model=None)
    prompts: list[dict[str, object]] = []
    invalid_payload = '{"outcome":"success","notes":["bad"]}'

    class FakeResult:
        def __init__(self, assistant_message: str) -> None:
            self.assistant_message = assistant_message
            self.command_text = ""
            self.token_total = None

    class FakeClient:
        def __init__(self, *args, **kwargs) -> None:
            return None

        def ensure_process(self, **kwargs) -> None:
            return None

        def start_thread(self, **kwargs) -> str:
            return "thread-123"

        def run_turn(self, **kwargs) -> FakeResult:
            prompts.append(kwargs)
            return FakeResult(invalid_payload)

        def close(self) -> None:
            return None

    monkeypatch.setattr(codex_backends_module, "CodexAppServerClient", FakeClient)

    result = backend.run(
        "audit_milestone",
        "hello",
        Context(),
        response_contract="status_envelope",
        contract_repair_attempts=1,
    )

    assert isinstance(result, Outcome)
    assert result.status == OutcomeStatus.FAIL
    assert result.failure_kind == FailureKind.CONTRACT
    assert result.failure_reason == "invalid structured status envelope: notes must be a string"
    assert result.notes == invalid_payload
    assert len(prompts) == 2


def test_codex_app_server_backend_returns_contract_failure_when_write_contract_repair_exhausted(
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    events: List[dict] = []
    backend = server.CodexAppServerBackend(str(tmp_path), events.append, model=None)
    prompts: list[dict[str, object]] = []
    invalid_payload = (
        '{"outcome":"success","context_updates":{"context.review.summary":"ready","context.review.extra":"nope"}}'
    )

    class FakeResult:
        def __init__(self, assistant_message: str) -> None:
            self.assistant_message = assistant_message
            self.command_text = ""
            self.token_total = None

    class FakeClient:
        def __init__(self, *args, **kwargs) -> None:
            return None

        def ensure_process(self, **kwargs) -> None:
            return None

        def start_thread(self, **kwargs) -> str:
            return "thread-123"

        def run_turn(self, **kwargs) -> FakeResult:
            prompts.append(kwargs)
            return FakeResult(invalid_payload)

        def close(self) -> None:
            return None

    monkeypatch.setattr(codex_backends_module, "CodexAppServerClient", FakeClient)

    result = backend.run(
        "review",
        "hello",
        Context(),
        response_contract="status_envelope",
        contract_repair_attempts=1,
        write_contract=ContextWriteContract(allowed_keys=("context.review.summary",)),
    )

    assert isinstance(result, Outcome)
    assert result.status == OutcomeStatus.FAIL
    assert result.failure_kind == FailureKind.CONTRACT
    assert "undeclared context_updates keys" in result.failure_reason
    assert "context.review.extra" in result.failure_reason
    assert "context.review.summary" in result.failure_reason
    assert result.notes == invalid_payload
    assert len(prompts) == 2


def test_codex_app_server_backend_preserves_exact_json_validation_reason(
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    events: List[dict] = []
    backend = server.CodexAppServerBackend(str(tmp_path), events.append, model=None)
    invalid_payload = '{"outcome":"success",}'

    class FakeResult:
        def __init__(self, assistant_message: str) -> None:
            self.assistant_message = assistant_message
            self.command_text = ""
            self.token_total = None

    class FakeClient:
        def __init__(self, *args, **kwargs) -> None:
            return None

        def ensure_process(self, **kwargs) -> None:
            return None

        def start_thread(self, **kwargs) -> str:
            return "thread-123"

        def run_turn(self, **kwargs) -> FakeResult:
            del kwargs
            return FakeResult(invalid_payload)

        def close(self) -> None:
            return None

    monkeypatch.setattr(codex_backends_module, "CodexAppServerClient", FakeClient)

    result = backend.run(
        "audit_milestone",
        "hello",
        Context(),
        response_contract="status_envelope",
        contract_repair_attempts=0,
    )

    assert isinstance(result, Outcome)
    assert result.status == OutcomeStatus.FAIL
    assert result.failure_kind == FailureKind.CONTRACT
    assert result.failure_reason.startswith("invalid structured status envelope: invalid JSON:")
    assert "line 1 column" in result.failure_reason
    assert "expected a JSON object with top-level key outcome" not in result.failure_reason


def test_codex_app_server_backend_fails_closed_on_malformed_structured_outcome_agent_text(
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    events: List[dict] = []
    backend = server.CodexAppServerBackend(str(tmp_path), events.append, model=None)

    class FakeStdout:
        def __init__(self, lines: list[str]) -> None:
            self._lines = list(lines)

        def readline(self) -> str:
            if not self._lines:
                return ""
            return f"{self._lines.pop(0)}\n"

    class FakeStdin:
        def write(self, text: str) -> None:
            return None

        def flush(self) -> None:
            return None

    class FakeProcess:
        def __init__(self, lines: list[str]) -> None:
            self.stdout = FakeStdout(lines)
            self.stdin = FakeStdin()

        def poll(self) -> None:
            return None

        def terminate(self) -> None:
            return None

        def wait(self, timeout: float | None = None) -> None:
            return None

        def kill(self) -> None:
            return None

    malformed_payload = (
        '{"outcome":"success","context":{"workflow_outcome":"failure"},'
        '"notes":"attempted blocked exit"}'
    )
    lines = [
        '{"jsonrpc":"2.0","id":1,"result":{"capabilities":{"experimentalApi":true}}}',
        '{"jsonrpc":"2.0","id":2,"result":{"thread":{"id":"thread-123"}}}',
        '{"jsonrpc":"2.0","id":3,"result":{"turn":{"id":"turn-123","status":"inProgress","items":[]}}}',
        f'{{"jsonrpc":"2.0","method":"item/agentMessage/delta","params":{{"delta":{json.dumps(malformed_payload)}}}}}',
        (
            '{"jsonrpc":"2.0","method":"item/completed","params":{"item":{"type":"AgentMessage","id":"msg-1",'
            f'"content":[{{"type":"Text","text":{json.dumps(malformed_payload)}}}],"phase":"final_answer"}}}}'
        ),
        '{"jsonrpc":"2.0","method":"turn/completed","params":{"turn":{"id":"turn-123","status":"completed"}}}',
    ]

    monkeypatch.setattr(codex_backends_module.subprocess, "Popen", lambda *args, **kwargs: FakeProcess(lines))

    result = backend.run("blocked_exit", "hello", Context())

    assert isinstance(result, Outcome)
    assert result.status == OutcomeStatus.FAIL
    assert result.notes == malformed_payload
    assert result.failure_reason == "invalid structured status envelope: unexpected top-level keys context"
    assert result.context_updates == {}


def test_codex_app_server_backend_requires_turn_completed_after_final_answer(
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    events: List[dict] = []
    backend = server.CodexAppServerBackend(str(tmp_path), events.append, model=None)

    class FakeStdout:
        def __init__(self, lines: list[str]) -> None:
            self._lines = list(lines)

        def readline(self) -> str:
            if not self._lines:
                return ""
            return f"{self._lines.pop(0)}\n"

    class FakeStdin:
        def write(self, text: str) -> None:
            return None

        def flush(self) -> None:
            return None

    class FakeProcess:
        def __init__(self, lines: list[str]) -> None:
            self.stdout = FakeStdout(lines)
            self.stdin = FakeStdin()

        def poll(self) -> None:
            return None

        def terminate(self) -> None:
            return None

        def wait(self, timeout: float | None = None) -> None:
            return None

        def kill(self) -> None:
            return None

    lines = [
        '{"jsonrpc":"2.0","id":1,"result":{"capabilities":{"experimentalApi":true}}}',
        '{"jsonrpc":"2.0","id":2,"result":{"thread":{"id":"thread-123"}}}',
        '{"jsonrpc":"2.0","id":3,"result":{"turn":{"id":"turn-123","status":"inProgress","items":[]}}}',
        '{"jsonrpc":"2.0","method":"item/agentMessage/delta","params":{"delta":"Ack"}}',
        '{"jsonrpc":"2.0","method":"item/completed","params":{"item":{"type":"AgentMessage","id":"msg-1","content":[{"type":"Text","text":"Ack"}],"phase":"final_answer"}}}',
    ]
    monotonic_values = itertools.count(0.0, 0.1)

    monkeypatch.setattr(codex_backends_module.subprocess, "Popen", lambda *args, **kwargs: FakeProcess(lines))
    monkeypatch.setattr(codex_backends_module.codex_app_protocol, "APP_SERVER_TURN_IDLE_TIMEOUT_SECONDS", 1.0)
    monkeypatch.setattr(codex_backends_module.time, "monotonic", lambda: next(monotonic_values))

    result = backend.run("plan", "hello", Context())

    assert isinstance(result, Outcome)
    assert result.status == OutcomeStatus.FAIL
    assert result.failure_reason == "codex app-server turn timed out waiting for activity"


def test_codex_app_server_backend_writes_stage_raw_rpc_log(
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    events: List[dict] = []
    backend = server.CodexAppServerBackend(str(tmp_path), events.append, model=None)

    class FakeStdout:
        def __init__(self, lines: list[str]) -> None:
            self._lines = list(lines)

        def readline(self) -> str:
            if not self._lines:
                return ""
            return f"{self._lines.pop(0)}\n"

    class FakeStdin:
        def write(self, text: str) -> None:
            return None

        def flush(self) -> None:
            return None

    class FakeProcess:
        def __init__(self, lines: list[str]) -> None:
            self.stdout = FakeStdout(lines)
            self.stdin = FakeStdin()

        def poll(self) -> None:
            return None

        def terminate(self) -> None:
            return None

        def wait(self, timeout: float | None = None) -> None:
            return None

        def kill(self) -> None:
            return None

    lines = [
        '{"jsonrpc":"2.0","id":1,"result":{"capabilities":{"experimentalApi":true}}}',
        '{"jsonrpc":"2.0","id":2,"result":{"thread":{"id":"thread-123"}}}',
        '{"jsonrpc":"2.0","id":3,"result":{"turn":{"id":"turn-123","status":"inProgress","items":[]}}}',
        '{"jsonrpc":"2.0","method":"item/agentMessage/delta","params":{"delta":"Ack"}}',
        '{"jsonrpc":"2.0","method":"item/completed","params":{"item":{"type":"AgentMessage","id":"msg-1","content":[{"type":"Text","text":"Ack"}],"phase":"final_answer"}}}',
        '{"jsonrpc":"2.0","method":"turn/completed","params":{"turn":{"id":"turn-123","status":"completed"}}}',
    ]

    monkeypatch.setattr(codex_backends_module.subprocess, "Popen", lambda *args, **kwargs: FakeProcess(lines))

    logs_root = tmp_path / "logs"
    with backend.bind_stage_raw_rpc_log("plan", logs_root):
        result = backend.run("plan", "hello", Context())

    assert result == "Ack"
    raw_log_path = logs_root / "plan" / "raw-rpc.jsonl"
    assert raw_log_path.exists()
    entries = [json.loads(line) for line in raw_log_path.read_text(encoding="utf-8").splitlines()]
    assert any(
        entry["direction"] == "outgoing" and json.loads(entry["line"]).get("method") == "turn/start"
        for entry in entries
    )
    assert any(
        entry["direction"] == "incoming" and json.loads(entry["line"]).get("method") == "turn/completed"
        for entry in entries
    )


def test_codex_app_server_backend_forwards_reasoning_effort_to_turn(
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    backend = server.CodexAppServerBackend(str(tmp_path), lambda event: None, model=None)
    run_turn_calls: list[dict[str, object]] = []

    class FakeResult:
        assistant_message = "Ack"
        command_text = ""
        token_total = None
        token_usage_payload = None

    class FakeClient:
        def __init__(self, *args, **kwargs) -> None:
            return None

        def ensure_process(self, **kwargs) -> None:
            return None

        def start_thread(self, **kwargs) -> str:
            return "thread-123"

        def run_turn(self, **kwargs) -> FakeResult:
            run_turn_calls.append(kwargs)
            return FakeResult()

        def close(self) -> None:
            return None

    monkeypatch.setattr(codex_backends_module, "CodexAppServerClient", FakeClient)

    result = backend.run("plan", "hello", Context(), reasoning_effort="high")

    assert result == "Ack"
    assert run_turn_calls[0]["reasoning_effort"] == "high"


def test_codex_app_server_backend_delivers_intervention_to_active_turn(
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    backend = server.CodexAppServerBackend(str(tmp_path), lambda event: None, model=None)
    intervention_result: ChildInterventionResult | None = None
    steer_calls: list[tuple[str, str, str]] = []

    class FakeResult:
        assistant_message = "Ack"
        command_text = ""
        token_total = None
        token_usage_payload = None

    class FakeClient:
        def __init__(self, *args, **kwargs) -> None:
            return None

        def ensure_process(self, **kwargs) -> None:
            return None

        def start_thread(self, **kwargs) -> str:
            return "thread-123"

        def run_turn(self, **kwargs) -> FakeResult:
            nonlocal intervention_result
            kwargs["on_turn_started"]("turn-456")
            intervention_result = backend.request_child_intervention(
                ChildInterventionRequest(
                    child_run_id="child-1",
                    message="Please fix the failure.",
                    parent_run_id="parent-1",
                    parent_node_id="manager",
                    root_run_id="parent-1",
                    reason="tests failed",
                    target_node_id="task",
                )
            )
            return FakeResult()

        def steer_turn(self, thread_id: str, turn_id: str, message: str) -> None:
            steer_calls.append((thread_id, turn_id, message))

        def close(self) -> None:
            return None

    monkeypatch.setattr(codex_backends_module, "CodexAppServerClient", FakeClient)

    assert backend.run("plan", "hello", Context()) == "Ack"
    assert steer_calls == [("thread-123", "turn-456", "Please fix the failure.")]
    assert intervention_result is not None
    assert intervention_result.status == "delivered"
    assert intervention_result.delivery_mode == "codex_app_server_turn"
    assert intervention_result.reason == "tests failed"
    inactive = backend.request_child_intervention(
        ChildInterventionRequest(
            child_run_id="child-1",
            message="late",
            parent_run_id="parent-1",
            parent_node_id="manager",
            root_run_id="parent-1",
        )
    )
    assert inactive.status == "rejected"
    assert inactive.reason == "no_active_turn"


def test_codex_app_server_backend_reports_steer_errors(
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    backend = server.CodexAppServerBackend(str(tmp_path), lambda event: None, model=None)
    intervention_result: ChildInterventionResult | None = None

    class FakeResult:
        assistant_message = "Ack"
        command_text = ""
        token_total = None
        token_usage_payload = None

    class FakeClient:
        def __init__(self, *args, **kwargs) -> None:
            return None

        def ensure_process(self, **kwargs) -> None:
            return None

        def start_thread(self, **kwargs) -> str:
            return "thread-123"

        def run_turn(self, **kwargs) -> FakeResult:
            nonlocal intervention_result
            kwargs["on_turn_started"]("turn-456")
            intervention_result = backend.request_child_intervention(
                ChildInterventionRequest(
                    child_run_id="child-1",
                    message="Please fix the failure.",
                    parent_run_id="parent-1",
                    parent_node_id="manager",
                    root_run_id="parent-1",
                )
            )
            return FakeResult()

        def steer_turn(self, thread_id: str, turn_id: str, message: str) -> None:
            del thread_id, turn_id, message
            raise RuntimeError("codex app-server turn/steer failed: rejected")

        def close(self) -> None:
            return None

    monkeypatch.setattr(codex_backends_module, "CodexAppServerClient", FakeClient)

    assert backend.run("plan", "hello", Context()) == "Ack"
    assert intervention_result is not None
    assert intervention_result.status == "rejected"
    assert intervention_result.reason == "app_server_steer_failed"
    assert "rejected" in intervention_result.message


@pytest.mark.parametrize(
    ("provider", "expected_backend"),
    [
        ("", "unified"),
        ("codex", "unified"),
        ("openai", "unified"),
        ("anthropic", "unified"),
        ("gemini", "unified"),
        ("openrouter", "unified"),
        ("litellm", "unified"),
    ],
)
def test_provider_router_dispatches_supported_providers(
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
    provider: str,
    expected_backend: str,
) -> None:
    calls: list[dict[str, object]] = []

    class FakeCodexBackend(_BackendRunAdapter):
        def __init__(self, *args, **kwargs) -> None:
            calls.append({"backend": "codex_init", "kwargs": kwargs})

        def bind_stage_raw_rpc_log(self, node_id, logs_root):
            raise AssertionError("not used")

        def run(self, *args, **kwargs) -> str:
            calls.append({"backend": "codex", "kwargs": kwargs})
            return "codex-result"

    class FakeUnifiedBackend(_BackendRunAdapter):
        def __init__(self, *args, **kwargs) -> None:
            calls.append({"backend": "unified_init", "kwargs": kwargs})

        def run(self, *args, **kwargs) -> str:
            calls.append({"backend": "unified", "kwargs": kwargs})
            return "unified-result"

    monkeypatch.setattr(codex_backends_module, "CodexAppServerBackend", FakeCodexBackend)
    monkeypatch.setattr(codex_backends_module, "UnifiedAgentBackend", FakeUnifiedBackend)
    backend = codex_backends_module.ProviderRouterBackend(str(tmp_path), lambda event: None, model="fallback-model")

    result = backend.run(
        "plan",
        "hello",
        Context(),
        provider=provider,
        model="node-model",
        reasoning_effort="medium",
    )

    assert result == f"{expected_backend}-result"
    run_call = calls[-1]
    assert run_call["backend"] == expected_backend
    assert run_call["kwargs"]["model"] == "node-model"
    assert run_call["kwargs"]["reasoning_effort"] == "medium"


def test_provider_router_forwards_intervention_to_active_backend(
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    router_box: dict[str, codex_backends_module.ProviderRouterBackend] = {}
    forwarded_results: list[ChildInterventionResult] = []

    class FakeUnifiedBackend(_BackendRunAdapter):
        def __init__(self, *args, **kwargs) -> None:
            del args, kwargs

        def request_child_intervention(self, request: ChildInterventionRequest) -> ChildInterventionResult:
            return ChildInterventionResult(
                run_id=request.child_run_id,
                status="delivered",
                delivery_mode="fake_unified",
                reason=request.reason,
            )

        def run(self, *args, **kwargs) -> str:
            del args, kwargs
            forwarded_results.append(
                router_box["router"].request_child_intervention(
                    ChildInterventionRequest(
                        child_run_id="child-1",
                        message="steer",
                        parent_run_id="parent-1",
                        parent_node_id="manager",
                        root_run_id="parent-1",
                        reason="tests failed",
                    )
                )
            )
            return "unified-result"

    monkeypatch.setattr(codex_backends_module, "UnifiedAgentBackend", FakeUnifiedBackend)
    router = codex_backends_module.ProviderRouterBackend(str(tmp_path), lambda event: None)
    router_box["router"] = router

    assert router.run("plan", "hello", Context(), provider="codex") == "unified-result"
    assert forwarded_results[0].status == "delivered"
    assert forwarded_results[0].delivery_mode == "fake_unified"
    inactive = router.request_child_intervention(
        ChildInterventionRequest(
            child_run_id="child-1",
            message="late",
            parent_run_id="parent-1",
            parent_node_id="manager",
            root_run_id="parent-1",
        )
    )
    assert inactive.status == "rejected"
    assert inactive.reason == "no_active_turn"


def test_provider_router_rejects_intervention_when_active_backend_is_unsupported(
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    router_box: dict[str, codex_backends_module.ProviderRouterBackend] = {}
    intervention_results: list[ChildInterventionResult] = []

    class FakeUnifiedBackend(_BackendRunAdapter):
        def __init__(self, *args, **kwargs) -> None:
            del args, kwargs

        def run(self, *args, **kwargs) -> str:
            del args, kwargs
            intervention_results.append(
                router_box["router"].request_child_intervention(
                    ChildInterventionRequest(
                        child_run_id="child-1",
                        message="steer",
                        parent_run_id="parent-1",
                        parent_node_id="manager",
                        root_run_id="parent-1",
                    )
                )
            )
            return "unified-result"

    monkeypatch.setattr(codex_backends_module, "UnifiedAgentBackend", FakeUnifiedBackend)
    router = codex_backends_module.ProviderRouterBackend(str(tmp_path), lambda event: None)
    router_box["router"] = router

    assert router.run("plan", "hello", Context(), provider="codex") == "unified-result"
    assert intervention_results[0].status == "rejected"
    assert intervention_results[0].reason == "backend_steering_unsupported"


def test_provider_router_fails_unknown_provider(tmp_path: Path) -> None:
    backend = codex_backends_module.ProviderRouterBackend(str(tmp_path), lambda event: None)

    result = backend.run("plan", "hello", Context(), provider="unknown")

    assert isinstance(result, Outcome)
    assert result.status == OutcomeStatus.FAIL
    assert result.failure_kind == FailureKind.RUNTIME
    assert "Unsupported llm_provider" in result.failure_reason


def test_pipeline_unified_provider_runtime_failure_writes_codergen_artifacts(
    attractor_api_client: TestClient,
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    server.configure_runtime_paths(runs_dir=tmp_path / "runs")
    provider_calls: list[str | None] = []

    class FakeCodexBackend(_BackendRunAdapter):
        def __init__(self, *args, **kwargs) -> None:
            del args, kwargs

        def bind_stage_raw_rpc_log(self, node_id, logs_root):
            del node_id, logs_root
            return nullcontext()

        def run(self, *args, **kwargs):
            raise AssertionError("codex backend should not run for unified providers")

    class FakeUnifiedBackend(_BackendRunAdapter):
        def __init__(self, *args, **kwargs) -> None:
            del args, kwargs

        def run(self, *args, **kwargs) -> Outcome:
            del args
            provider_calls.append(kwargs.get("provider"))
            return Outcome(
                status=OutcomeStatus.FAIL,
                failure_kind=FailureKind.RUNTIME,
                failure_reason="provider exploded",
            )

    monkeypatch.setattr(codex_backends_module, "CodexAppServerBackend", FakeCodexBackend)
    monkeypatch.setattr(codex_backends_module, "UnifiedAgentBackend", FakeUnifiedBackend)

    payload = _start_pipeline_via_http(
        attractor_api_client,
        {
            "flow_content": """
            digraph G {
                start [shape=Mdiamond]
                task [shape=box, prompt="Call provider"]
                done [shape=Msquare]
                start -> task
                task -> done
            }
            """,
            "working_directory": str(tmp_path / "work"),
            "llm_provider": "openai",
            "model": "gpt-test",
        },
    )

    result = _wait_for_pipeline_completion(attractor_api_client, payload["run_id"])

    assert result["status"] == "failed"
    assert provider_calls == ["openai"]
    stage_dir = server._run_root(payload["run_id"]) / "logs" / "task"
    assert (stage_dir / "response.md").read_text(encoding="utf-8").strip() == "provider exploded"
    status = json.loads((stage_dir / "status.json").read_text(encoding="utf-8"))
    assert status["outcome"] == "fail"
    assert status["failure_kind"] == "runtime"
    assert status["context_updates"]["last_response"] == "provider exploded"


def test_provider_router_reports_cumulative_unified_usage_across_nodes(
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    usage_updates: list[codex_backends_module.TokenUsageBreakdown] = []

    class FakeCodexBackend(_BackendRunAdapter):
        def __init__(self, *args, **kwargs) -> None:
            del args, kwargs

        def bind_stage_raw_rpc_log(self, node_id, logs_root):
            raise AssertionError("not used")

    class FakeUnifiedBackend(_BackendRunAdapter):
        def __init__(self, *args, **kwargs) -> None:
            del args
            self._on_usage_update = kwargs["on_usage_update"]

        def run(self, *args, **kwargs) -> str:
            del args
            model = str(kwargs["model"])
            usage_by_model = {
                "model-a": codex_backends_module.TokenUsageBucket(
                    input_tokens=3,
                    output_tokens=4,
                    total_tokens=7,
                ),
                "model-b": codex_backends_module.TokenUsageBucket(
                    input_tokens=5,
                    cached_input_tokens=2,
                    output_tokens=6,
                    total_tokens=11,
                ),
            }
            snapshot = codex_backends_module.TokenUsageBreakdown()
            snapshot.add_for_model(model, usage_by_model[model])
            self._on_usage_update(snapshot)
            return f"ok-{model}"

    monkeypatch.setattr(codex_backends_module, "CodexAppServerBackend", FakeCodexBackend)
    monkeypatch.setattr(codex_backends_module, "UnifiedAgentBackend", FakeUnifiedBackend)

    backend = codex_backends_module.ProviderRouterBackend(
        str(tmp_path),
        lambda event: None,
        on_usage_update=usage_updates.append,
    )

    assert backend.run("first", "hello", Context(), provider="openai", model="model-a") == "ok-model-a"
    assert backend.run("second", "hello", Context(), provider="anthropic", model="model-b") == "ok-model-b"

    assert len(usage_updates) == 2
    assert usage_updates[0].to_dict() == {
        "input_tokens": 3,
        "cached_input_tokens": 0,
        "output_tokens": 4,
        "total_tokens": 7,
        "by_model": {
            "model-a": {
                "input_tokens": 3,
                "cached_input_tokens": 0,
                "output_tokens": 4,
                "total_tokens": 7,
            },
        },
    }
    assert usage_updates[-1].to_dict() == {
        "input_tokens": 8,
        "cached_input_tokens": 2,
        "output_tokens": 10,
        "total_tokens": 18,
        "by_model": {
            "model-a": {
                "input_tokens": 3,
                "cached_input_tokens": 0,
                "output_tokens": 4,
                "total_tokens": 7,
            },
            "model-b": {
                "input_tokens": 5,
                "cached_input_tokens": 2,
                "output_tokens": 6,
                "total_tokens": 11,
            },
        },
    }


def test_launch_provider_ignores_ui_default_provider_until_materialized() -> None:
    graph = server.parse_dot(
        """
        digraph G {
            graph [ui_default_llm_provider="openai"]
            task [shape=box]
        }
        """
    )

    assert server._resolve_launch_provider(graph, None) == "codex"


def test_launch_reasoning_ignores_ui_default_reasoning_until_materialized() -> None:
    graph = server.parse_dot(
        """
        digraph G {
            graph [ui_default_reasoning_effort="high"]
            task [shape=box]
        }
        """
    )

    assert server._resolve_launch_reasoning_effort(graph, None) is None


def test_pipeline_launch_rejects_missing_api_provider_key_before_scheduling(
    attractor_api_client: TestClient,
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    monkeypatch.delenv("OPENAI_API_KEY", raising=False)
    run_id = "missing-openai-key"

    payload = _start_pipeline_via_http(
        attractor_api_client,
        {
            "run_id": run_id,
            "flow_content": """
            digraph G {
                start [shape=Mdiamond]
                task [shape=box, prompt="Use OpenAI"]
                done [shape=Msquare]
                start -> task -> done
            }
            """,
            "working_directory": str(tmp_path / "work"),
            "llm_provider": "openai",
        },
    )

    assert payload["status"] == "validation_error"
    assert payload["error"] == "Provider openai is not configured: missing OPENAI_API_KEY."
    assert server._get_active_run(run_id) is None
    assert server._read_run_meta(server._run_meta_path(run_id)) is None


def test_pipeline_launch_rejects_missing_openrouter_key_before_scheduling(
    attractor_api_client: TestClient,
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    monkeypatch.delenv("OPENROUTER_API_KEY", raising=False)
    payload = _start_pipeline_via_http(
        attractor_api_client,
        {
            "run_id": "missing-openrouter-key",
            "flow_content": """
            digraph G {
                start [shape=Mdiamond]
                task [shape=box, prompt="Use OpenRouter"]
                done [shape=Msquare]
                start -> task -> done
            }
            """,
            "working_directory": str(tmp_path / "work"),
            "llm_provider": "openrouter",
            "model": "openai/gpt-test",
        },
    )

    assert payload["status"] == "validation_error"
    assert payload["error"] == "Provider openrouter is not configured: missing OPENROUTER_API_KEY."


def test_pipeline_launch_rejects_missing_litellm_base_url_before_scheduling(
    attractor_api_client: TestClient,
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    monkeypatch.delenv("LITELLM_BASE_URL", raising=False)
    payload = _start_pipeline_via_http(
        attractor_api_client,
        {
            "run_id": "missing-litellm-base-url",
            "flow_content": """
            digraph G {
                start [shape=Mdiamond]
                task [shape=box, prompt="Use LiteLLM"]
                done [shape=Msquare]
                start -> task -> done
            }
            """,
            "working_directory": str(tmp_path / "work"),
            "llm_provider": "litellm",
            "model": "team-model",
        },
    )

    assert payload["status"] == "validation_error"
    assert payload["error"] == "Provider litellm is not configured: missing LITELLM_BASE_URL."


def test_pipeline_launch_rejects_missing_codex_auth_before_scheduling(
    attractor_api_client: TestClient,
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    runtime_root = tmp_path / "empty-codex-runtime"
    monkeypatch.setenv("HOME", str(tmp_path / "empty-home"))
    monkeypatch.setenv("ATTRACTOR_CODEX_RUNTIME_ROOT", str(runtime_root))
    monkeypatch.setenv("ATTRACTOR_CODEX_SEED_DIR", str(tmp_path / "empty-seed"))
    run_id = "missing-codex-auth"

    payload = _start_pipeline_via_http(
        attractor_api_client,
        {
            "run_id": run_id,
            "flow_content": """
            digraph G {
                start [shape=Mdiamond]
                task [shape=box, prompt="Use Codex"]
                done [shape=Msquare]
                start -> task -> done
            }
            """,
            "working_directory": str(tmp_path / "work"),
            "llm_provider": "codex",
        },
    )

    assert payload["status"] == "validation_error"
    assert payload["error"] == "Provider codex is not configured: missing CODEX_HOME/auth.json."
    assert server._get_active_run(run_id) is None
    assert server._read_run_meta(server._run_meta_path(run_id)) is None


def test_pipeline_launch_skips_provider_preflight_for_non_llm_flow(
    attractor_api_client: TestClient,
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    monkeypatch.delenv("OPENAI_API_KEY", raising=False)

    payload = _start_pipeline_via_http(
        attractor_api_client,
        {
            "flow_content": """
            digraph G {
                start [shape=Mdiamond]
                done [shape=Msquare]
                start -> done
            }
            """,
            "working_directory": str(tmp_path / "work"),
            "llm_provider": "openai",
        },
    )

    assert payload["status"] == "started"
    final_payload = _wait_for_pipeline_completion(attractor_api_client, str(payload["run_id"]))
    assert final_payload["status"] == "completed"


class _FakeRustBoundary:
    def __init__(
        self,
        outputs: list[dict[str, object] | BaseException],
        *,
        before_output=None,
        steer_outputs: list[dict[str, object] | BaseException] | None = None,
    ) -> None:
        self.requests: list[dict[str, object]] = []
        self.steer_requests: list[dict[str, object]] = []
        self.outputs = list(outputs)
        self.steer_outputs = list(steer_outputs or [{"status": "delivered", "delivery_mode": "rust_boundary_codergen_turn"}])
        self.before_output = before_output

    def run_codergen(self, payload: dict[str, object]) -> dict[str, object]:
        self.requests.append(payload)
        if self.before_output is not None:
            self.before_output()
        output = self.outputs.pop(0)
        if isinstance(output, BaseException):
            raise output
        return output

    def steer_codergen_turn(self, payload: dict[str, object]) -> dict[str, object]:
        self.steer_requests.append(payload)
        output = self.steer_outputs.pop(0)
        if isinstance(output, BaseException):
            raise output
        return output


def test_unified_agent_backend_returns_plain_text_and_records_tool_events(
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    del monkeypatch
    emitted: list[dict[str, str]] = []
    usage_snapshots = []
    boundary = _FakeRustBoundary(
        [
            {
                "events": [
                    {"kind": "content_delta", "channel": "assistant", "content_delta": "Hello"},
                    {
                        "kind": "tool_call_updated",
                        "tool_call": {
                            "id": "tool-1",
                            "kind": "command_execution",
                            "status": "running",
                            "title": "shell",
                        },
                    },
                    {
                        "kind": "tool_call_updated",
                        "tool_call": {
                            "id": "tool-1",
                            "kind": "command_execution",
                            "status": "completed",
                            "title": "shell",
                        },
                    },
                ],
                "response": {"kind": "text", "value": "Unified reply"},
                "usage": {"input_tokens": 3, "output_tokens": 4, "total_tokens": 7},
            }
        ]
    )
    backend = codex_backends_module.UnifiedAgentBackend(
        str(tmp_path),
        emitted.append,
        provider="openai",
        on_usage_update=usage_snapshots.append,
        boundary=boundary,
    )

    result = backend.run("plan", "hello", Context(), model="gpt-test", reasoning_effort="high")

    assert result == "Unified reply"
    assert boundary.requests[0]["reasoning_effort"] == "high"
    assert [event["msg"] for event in emitted] == [
        "[plan] Hello",
        "[plan] tool running: shell",
        "[plan] tool completed: shell",
        "[plan] Unified reply",
    ]
    assert usage_snapshots[-1].by_model["gpt-test"].total_tokens == 7


def test_unified_agent_backend_delivers_intervention_to_active_session(
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    del monkeypatch
    backend_box: dict[str, codex_backends_module.UnifiedAgentBackend] = {}
    intervention_results: list[ChildInterventionResult] = []

    def request_during_turn() -> None:
        intervention_results.append(
            backend_box["backend"].request_child_intervention(
                ChildInterventionRequest(
                    child_run_id="child-1",
                    message="Please fix the failure.",
                    parent_run_id="parent-1",
                    parent_node_id="manager",
                    root_run_id="parent-1",
                    reason="tests failed",
                )
            )
        )

    boundary = _FakeRustBoundary(
        [{"response": {"kind": "text", "value": "Unified reply"}}],
        before_output=request_during_turn,
    )
    backend = codex_backends_module.UnifiedAgentBackend(
        str(tmp_path),
        lambda event: None,
        provider="openai",
        boundary=boundary,
    )
    backend_box["backend"] = backend

    assert backend.run("plan", "hello", Context(), model="gpt-test") == "Unified reply"
    assert intervention_results[0].status == "delivered"
    assert intervention_results[0].delivery_mode == "rust_boundary_codergen_turn"
    assert intervention_results[0].reason == "tests failed"
    assert boundary.steer_requests[0]["turn_id"] == boundary.requests[0]["turn_id"]
    assert boundary.steer_requests[0]["message"] == "Please fix the failure."
    assert boundary.steer_requests[0]["child_run_id"] == "child-1"
    assert boundary.steer_requests[0]["parent_node_id"] == "manager"
    inactive = backend.request_child_intervention(
        ChildInterventionRequest(
            child_run_id="child-1",
            message="late",
            parent_run_id="parent-1",
            parent_node_id="manager",
            root_run_id="parent-1",
        )
    )
    assert inactive.status == "rejected"
    assert inactive.reason == "no_active_turn"


def test_unified_agent_backend_emits_llm_content_progress(
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    del monkeypatch
    progress_events: list[dict[str, object]] = []
    boundary = _FakeRustBoundary(
        [
            {
                "events": [
                    {
                        "kind": "content_delta",
                        "channel": "assistant",
                        "content_delta": "Hello ",
                        "source": {
                            "backend": "rust_agent",
                            "raw_kind": "assistant_text_delta",
                            "response_id": "r-1",
                        },
                    },
                    {
                        "kind": "content_completed",
                        "channel": "assistant",
                        "content_delta": "Hello world",
                        "source": {
                            "backend": "rust_agent",
                            "raw_kind": "assistant_text_end",
                            "response_id": "r-1",
                        },
                    },
                    {
                        "kind": "content_delta",
                        "channel": "reasoning",
                        "content_delta": "Thinking",
                        "source": {
                            "backend": "rust_agent",
                            "raw_kind": "assistant_reasoning_delta",
                            "response_id": "r-1",
                        },
                    },
                ],
                "response": {"kind": "text", "value": "Hello world"},
            }
        ]
    )
    backend = codex_backends_module.UnifiedAgentBackend(
        str(tmp_path),
        lambda event: None,
        provider="openai",
        boundary=boundary,
    )

    result = backend.run(
        "plan",
        "hello",
        Context(),
        emit_event=lambda event_type, **payload: progress_events.append({
            "type": event_type,
            **payload,
        }),
    )

    assert result == "Hello world"
    assert progress_events == [
        {
            "type": "LLMContent",
            "node_id": "plan",
            "channel": "assistant",
            "content_delta": "Hello ",
            "status": "streaming",
            "phase": None,
            "source": {
                "backend": "rust_agent",
                "raw_kind": "assistant_text_delta",
                "response_id": "r-1",
            },
        },
        {
            "type": "LLMContent",
            "node_id": "plan",
            "channel": "assistant",
            "content_delta": "Hello world",
            "status": "complete",
            "phase": None,
            "source": {
                "backend": "rust_agent",
                "raw_kind": "assistant_text_end",
                "response_id": "r-1",
            },
        },
        {
            "type": "LLMContent",
            "node_id": "plan",
            "channel": "reasoning",
            "content_delta": "Thinking",
            "status": "streaming",
            "phase": None,
            "source": {
                "backend": "rust_agent",
                "raw_kind": "assistant_reasoning_delta",
                "response_id": "r-1",
            },
        },
    ]


def test_unified_agent_backend_coerces_status_envelope(
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    del monkeypatch
    boundary = _FakeRustBoundary(
        [
            {
                "response": {
                    "kind": "text",
                    "value": '{"outcome":"success","context_updates":{"result":"ok"},"notes":"done"}',
                }
            }
        ]
    )
    backend = codex_backends_module.UnifiedAgentBackend(
        str(tmp_path),
        lambda event: None,
        provider="anthropic",
        boundary=boundary,
    )

    result = backend.run("plan", "hello", Context(), response_contract="status_envelope")

    assert isinstance(result, Outcome)
    assert result.status == OutcomeStatus.SUCCESS
    assert result.context_updates == {"result": "ok"}


def test_unified_agent_backend_repairs_contract_in_same_session(
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    del monkeypatch
    boundary = _FakeRustBoundary(
        [
            {"response": {"kind": "text", "value": '{"outcome":"success","notes":["bad"]}'}},
            {"response": {"kind": "text", "value": '{"outcome":"success","notes":"fixed"}'}},
        ]
    )
    backend = codex_backends_module.UnifiedAgentBackend(
        str(tmp_path),
        lambda event: None,
        provider="gemini",
        boundary=boundary,
    )

    result = backend.run(
        "plan",
        "hello",
        Context(),
        response_contract="status_envelope",
        contract_repair_attempts=1,
    )

    assert isinstance(result, Outcome)
    assert result.status == OutcomeStatus.SUCCESS
    assert len(boundary.requests) == 2
    assert boundary.requests[0]["prompt"] == "hello"
    assert boundary.requests[1]["repair_attempt"] == 1
    assert boundary.requests[1]["prompt"] != boundary.requests[0]["prompt"]


def test_unified_agent_backend_fails_interactive_input_request(
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    del monkeypatch
    boundary = _FakeRustBoundary(
        [
            {
                "error": {
                    "message": "unified-agent codergen requested interactive input; this is not supported for v1",
                    "retryable": False,
                }
            }
        ]
    )
    backend = codex_backends_module.UnifiedAgentBackend(
        str(tmp_path),
        lambda event: None,
        provider="openai",
        boundary=boundary,
    )

    result = backend.run("plan", "hello", Context())

    assert isinstance(result, Outcome)
    assert result.status == OutcomeStatus.FAIL
    assert result.failure_kind == FailureKind.RUNTIME
    assert "interactive input" in result.failure_reason


def test_unified_agent_backend_normalizes_provider_exception_and_closes_session(
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    del monkeypatch
    boundary = _FakeRustBoundary([ValueError("provider exploded")])

    backend = codex_backends_module.UnifiedAgentBackend(
        str(tmp_path),
        lambda event: None,
        provider="openai",
        boundary=boundary,
    )

    result = backend.run("plan", "hello", Context(), model="gpt-test")

    assert isinstance(result, Outcome)
    assert result.status == OutcomeStatus.FAIL
    assert result.failure_kind == FailureKind.RUNTIME
    assert result.failure_reason == "provider exploded"


def test_unified_agent_backend_marks_auth_setup_failures_non_retryable(
    tmp_path: Path,
) -> None:
    boundary = _FakeRustBoundary([ValueError("missing OPENAI_API_KEY")])
    backend = codex_backends_module.UnifiedAgentBackend(
        str(tmp_path),
        lambda event: None,
        provider="openai",
        boundary=boundary,
    )

    result = backend.run("plan", "hello", Context(), model="gpt-test")

    assert isinstance(result, Outcome)
    assert result.status == OutcomeStatus.FAIL
    assert result.failure_kind == FailureKind.RUNTIME
    assert result.failure_reason == "missing OPENAI_API_KEY"
    assert result.retryable is False


def test_unified_agent_backend_preserves_timeout_in_boundary_request(
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    del monkeypatch
    boundary = _FakeRustBoundary([{"response": {"kind": "text", "value": "on time"}}])

    backend = codex_backends_module.UnifiedAgentBackend(
        str(tmp_path),
        lambda event: None,
        provider="openai",
        boundary=boundary,
    )

    result = backend.run("plan", "hello", Context(), model="gpt-test", timeout=0.5)

    assert result == "on time"
    assert boundary.requests[0]["timeout_seconds"] == 0.5


def test_unified_agent_backend_times_out_blocked_boundary_call(tmp_path: Path) -> None:
    release = threading.Event()

    class BlockingBoundary(_FakeRustBoundary):
        def run_codergen(self, payload: dict[str, object]) -> dict[str, object]:
            self.requests.append(payload)
            release.wait(2)
            return {"response": {"kind": "text", "value": "late"}}

    boundary = BlockingBoundary([])
    backend = codex_backends_module.UnifiedAgentBackend(
        str(tmp_path),
        lambda event: None,
        provider="openai",
        boundary=boundary,
    )

    try:
        result = backend.run("plan", "hello", Context(), model="gpt-test", timeout=0.01)
    finally:
        release.set()

    assert isinstance(result, Outcome)
    assert result.status == OutcomeStatus.FAIL
    assert result.failure_kind == FailureKind.RUNTIME
    assert "timed out after 0.01s" in result.failure_reason
    assert boundary.requests[0]["timeout_seconds"] == 0.01
