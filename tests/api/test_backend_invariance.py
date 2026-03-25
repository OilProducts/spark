from __future__ import annotations

import itertools
from pathlib import Path
from typing import List

import pytest
from fastapi.testclient import TestClient

import attractor.api.codex_backends as codex_backends_module
import attractor.api.server as server
from attractor.engine import Context, load_checkpoint
from attractor.engine.outcome import Outcome, OutcomeStatus
from spark_common.runtime import (
    build_codex_runtime_environment,
    build_project_id,
    resolve_runtime_workspace_path,
)
from tests.api._support import (
    SIMPLE_FLOW as FLOW,
    close_task_immediately as _close_task_immediately,
    wait_for_pipeline_completion as _wait_for_pipeline_completion,
)


def _start_pipeline_via_http(attractor_api_client: TestClient, payload: dict) -> dict:
    response = attractor_api_client.post("/pipelines", json=payload)
    assert response.status_code == 200
    return response.json()


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
    assert checkpoint.context["graph.default_max_retry"] == 7
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
    with pytest.raises(ValueError, match="Unsupported backend. Supported backends: codex-app-server."):
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
    assert events[-1] == {"type": "log", "msg": "[plan] codex app-server not found on PATH"}


def test_resolve_runtime_workspace_path_maps_host_repo_root_override_to_runtime_repo_root(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    runtime_repo_root = Path(server.__file__).resolve().parents[3]
    monkeypatch.setenv("ATTRACTOR_HOST_REPO_ROOT", "/Users/chris/projects/spark")
    monkeypatch.setenv("ATTRACTOR_RUNTIME_REPO_ROOT", str(runtime_repo_root))
    translated = resolve_runtime_workspace_path("/home/chris/tinker/spark/frontend")

    assert translated == str((runtime_repo_root / "frontend").resolve(strict=False))


def test_build_codex_runtime_environment_isolates_home_and_seeds_runtime_state(
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    runtime_root = tmp_path / "codex-runtime"
    seed_dir = tmp_path / "codex-seed"
    seed_dir.mkdir()
    (seed_dir / "auth.json").write_text('{"token":"seed"}', encoding="utf-8")
    (seed_dir / "config.toml").write_text("model = 'gpt-test'\n", encoding="utf-8")
    monkeypatch.setenv("ATTRACTOR_CODEX_RUNTIME_ROOT", str(runtime_root))
    monkeypatch.setenv("ATTRACTOR_CODEX_SEED_DIR", str(seed_dir))
    monkeypatch.delenv("CODEX_HOME", raising=False)
    monkeypatch.delenv("XDG_CONFIG_HOME", raising=False)
    monkeypatch.delenv("XDG_DATA_HOME", raising=False)

    env = build_codex_runtime_environment()

    assert env["HOME"] == str(runtime_root)
    assert env["CODEX_HOME"] == str(runtime_root / ".codex")
    assert env["XDG_CONFIG_HOME"] == str(runtime_root / ".config")
    assert env["XDG_DATA_HOME"] == str(runtime_root / ".local/share")
    assert (runtime_root / ".codex" / "auth.json").read_text(encoding="utf-8") == '{"token":"seed"}'
    assert (runtime_root / ".codex" / "config.toml").read_text(encoding="utf-8") == "model = 'gpt-test'\n"


def test_build_codex_runtime_environment_falls_back_to_host_codex_home_when_seed_dir_missing(
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    runtime_root = tmp_path / "codex-runtime"
    host_home = tmp_path / "host-home"
    host_codex_home = host_home / ".codex"
    host_codex_home.mkdir(parents=True)
    (host_codex_home / "auth.json").write_text('{"token":"host-seed"}', encoding="utf-8")
    (host_codex_home / "config.toml").write_text("model = 'host-model'\n", encoding="utf-8")

    monkeypatch.setenv("HOME", str(host_home))
    monkeypatch.setenv("ATTRACTOR_CODEX_RUNTIME_ROOT", str(runtime_root))
    monkeypatch.setenv("ATTRACTOR_CODEX_SEED_DIR", str(tmp_path / "missing-seed"))
    monkeypatch.delenv("CODEX_HOME", raising=False)
    monkeypatch.delenv("XDG_CONFIG_HOME", raising=False)
    monkeypatch.delenv("XDG_DATA_HOME", raising=False)

    env = build_codex_runtime_environment()

    assert env["CODEX_HOME"] == str(runtime_root / ".codex")
    assert (runtime_root / ".codex" / "auth.json").read_text(encoding="utf-8") == '{"token":"host-seed"}'
    assert (runtime_root / ".codex" / "config.toml").read_text(encoding="utf-8") == "model = 'host-model'\n"


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
    assert str(missing_dir.resolve(strict=False)) in str(result.failure_reason)
    assert "working directory is unavailable in the runtime" in events[-1]["msg"]


def test_codex_app_server_backend_reuses_session_for_same_thread_key(tmp_path: Path) -> None:
    backend = server.CodexAppServerBackend(str(tmp_path), lambda event: None, model=None)
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


def test_codex_app_server_backend_isolates_sessions_for_different_thread_keys(
    tmp_path: Path,
) -> None:
    backend = server.CodexAppServerBackend(str(tmp_path), lambda event: None, model=None)
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


def test_codex_app_server_backend_does_not_cache_empty_thread_key(tmp_path: Path) -> None:
    backend = server.CodexAppServerBackend(str(tmp_path), lambda event: None, model=None)
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

    result = backend.run("review", "hello", Context())

    assert isinstance(result, Outcome)
    assert result.status == OutcomeStatus.FAIL
    assert result.notes == "needs fixes"
    assert result.failure_reason == "review requested changes"
    assert result.context_updates == {"context.review.summary": "missing validation"}


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
    monkeypatch.setattr(codex_backends_module.codex_app_server, "APP_SERVER_TURN_IDLE_TIMEOUT_SECONDS", 1.0)
    monkeypatch.setattr(codex_backends_module.time, "monotonic", lambda: next(monotonic_values))

    result = backend.run("plan", "hello", Context())

    assert isinstance(result, Outcome)
    assert result.status == OutcomeStatus.FAIL
    assert result.failure_reason == "codex app-server turn timed out waiting for activity"
