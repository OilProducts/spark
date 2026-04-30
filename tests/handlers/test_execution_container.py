from __future__ import annotations

import io
import json
from types import SimpleNamespace
from pathlib import Path
from typing import Any

from attractor.dsl import parse_dot
from attractor.engine.executor import PipelineExecutor
from attractor.engine.context import Context
from attractor.engine.outcome import Outcome
from attractor.engine.outcome import OutcomeStatus
from attractor.handlers.base import ChildRunRequest, ChildRunResult
from attractor.handlers.execution_container import (
    ContainerExecutionError,
    ContainerizedHandlerRunner,
    DockerContainerTransport,
    WorkerCallbacks,
    WorkerProtocolInterviewer,
    _container_env,
    graph_to_payload,
    resolve_execution_profile,
    run_worker_node,
    worker_child_run_launcher,
    worker_child_status_resolver,
)
from attractor.interviewer import Question, QuestionType


class FakeTransport:
    def __init__(self) -> None:
        self.requests: list[dict[str, Any]] = []
        self.closed = False

    def run_node(self, request: dict[str, Any], callbacks: WorkerCallbacks) -> dict[str, Any]:
        self.requests.append(request)
        if callbacks.emit_event is not None:
            callbacks.emit_event("WorkerProgress", {"node_id": request["node_id"]})
        return {
            "type": "result",
            "context": {
                **dict(request["context"]),
                "context.container_ran": request["node_id"],
            },
            "outcome": {
                "status": "success",
                "preferred_label": "",
                "suggested_next_ids": [],
                "context_updates": {},
                "notes": "ran in fake container",
                "failure_reason": "",
            },
        }

    def close(self) -> None:
        self.closed = True

    def cancel(self) -> None:
        self.closed = True


def test_resolve_execution_profile_prefers_run_override_then_project_default() -> None:
    assert resolve_execution_profile(requested_image="run:latest", project_default_image="project:latest").image == "run:latest"
    assert resolve_execution_profile(requested_image="", project_default_image="project:latest").image == "project:latest"
    assert resolve_execution_profile(requested_image=None, project_default_image=None).mode == "native"


def test_containerized_handler_runner_delegates_node_and_merges_worker_context(tmp_path: Path) -> None:
    graph = parse_dot(
        """
        digraph G {
          start [shape=Mdiamond];
          work [shape=tool, tool.command="true"];
          done [shape=Msquare];
          start -> work -> done;
        }
        """
    )
    transport = FakeTransport()
    runner = ContainerizedHandlerRunner(
        graph,
        image="spark-exec:test",
        run_id="run-container-test",
        working_dir=tmp_path,
        run_root=tmp_path / "runs" / "run-container-test",
        transport=transport,
    )
    runner.set_logs_root(tmp_path / "runs" / "run-container-test" / "logs")
    context = Context(values={"context.input": "value"})
    events: list[tuple[str, dict[str, object]]] = []

    outcome = runner("work", "do work", context, emit_event=lambda event_type, **payload: events.append((event_type, payload)))

    assert outcome is not None
    assert outcome.status == OutcomeStatus.SUCCESS
    assert context.get("context.container_ran") == "work"
    assert transport.requests[0]["node_id"] == "work"
    assert transport.requests[0]["context"]["context.input"] == "value"
    assert events == [("WorkerProgress", {"node_id": "work"})]
    runner.close()
    assert transport.closed is False


def test_docker_transport_creates_run_container_with_labels_env_mounts_and_cleanup(
    tmp_path: Path, monkeypatch
) -> None:
    commands: list[list[str]] = []
    stdin_writes: list[str] = []
    monkeypatch.setenv("SPARK_PROJECTS_HOST_DIR", str(tmp_path / "host-projects"))
    monkeypatch.setenv("SPARK_DOCKER_HOME", str(tmp_path / "host-spark"))
    monkeypatch.setattr("attractor.handlers.execution_container.shutil.which", lambda _: "/usr/bin/docker")

    def fake_run(args, **kwargs):
        commands.append(list(args))
        if args[:2] == ["docker", "run"]:
            return SimpleNamespace(returncode=0, stdout="container-123\n", stderr="")
        return SimpleNamespace(returncode=0, stdout="", stderr="")

    class FakePopen:
        def __init__(self, args, **kwargs):
            commands.append(list(args))
            self.stdin = _RecordingStdin(stdin_writes)
            self.stdout = iter(
                [
                    json.dumps({"type": "event", "event_type": "WorkerProgress", "payload": {"node_id": "work"}})
                    + "\n",
                    json.dumps(
                        {
                            "type": "result",
                            "context": {"context.ran": True},
                            "outcome": {"status": "success", "suggested_next_ids": [], "context_updates": {}},
                        }
                    )
                    + "\n",
                ]
            )
            self.stderr = io.StringIO("")

        def wait(self, timeout=None):
            return 0

    monkeypatch.setattr("attractor.handlers.execution_container.subprocess.run", fake_run)
    monkeypatch.setattr("attractor.handlers.execution_container.subprocess.Popen", FakePopen)
    transport = DockerContainerTransport(
        image="spark-exec:test",
        run_id="run-123",
        project_path=Path("/projects/acme"),
        run_root=Path("/spark/runs/project/run-123"),
        spark_runtime_root=Path("/spark/runtime/codex"),
        labels={"spark.project": "acme", "spark.execution_mode": "container"},
        env={"OPENAI_API_KEY": "secret", "UNRELATED": "kept-if-explicit"},
    )

    events: list[tuple[str, dict[str, Any]]] = []
    result = transport.run_node(
        {"node_id": "work"},
        WorkerCallbacks(emit_event=lambda event_type, payload: events.append((event_type, payload))),
    )
    transport.close()

    run_command = commands[0]
    assert run_command[:5] == ["docker", "run", "-d", "--name", transport.container_name]
    assert "--label" in run_command
    assert "spark.run_id=run-123" in run_command
    assert "spark.project=acme" in run_command
    assert "spark.execution_mode=container" in run_command
    assert "-v" in run_command
    assert f"{tmp_path / 'host-projects' / 'acme'}:/projects/acme:rw" in run_command
    assert f"{tmp_path / 'host-spark' / 'runs/project'}:/spark/runs/project:rw" in run_command
    assert f"{tmp_path / 'host-spark' / 'runs/project/run-123'}:/spark/runs/project/run-123:rw" in run_command
    assert f"{tmp_path / 'host-spark' / 'runtime/codex'}:/spark/runtime/codex:rw" in run_command
    assert "-e" in run_command
    assert "OPENAI_API_KEY=secret" in run_command
    assert "UNRELATED=kept-if-explicit" in run_command
    assert commands[1] == ["docker", "exec", "-i", "container-123", "spark-server", "worker", "run-node"]
    assert commands[-1] == ["docker", "rm", "-f", "container-123"]
    assert json.loads(stdin_writes[0])["node_id"] == "work"
    assert events == [("WorkerProgress", {"node_id": "work"})]
    assert result["context"] == {"context.ran": True}


def test_containerized_handler_runner_default_transport_labels_run_and_project_metadata(
    tmp_path: Path, monkeypatch
) -> None:
    commands: list[list[str]] = []
    project_dir = tmp_path / "project"
    run_root = tmp_path / "runs" / "run-production-labels"
    project_dir.mkdir()
    monkeypatch.setattr("attractor.handlers.execution_container.shutil.which", lambda _: "/usr/bin/docker")

    def fake_run(args, **kwargs):
        commands.append(list(args))
        if args[:2] == ["docker", "run"]:
            return SimpleNamespace(returncode=0, stdout="container-production\n", stderr="")
        return SimpleNamespace(returncode=0, stdout="", stderr="")

    class FakePopen:
        def __init__(self, args, **kwargs):
            commands.append(list(args))
            self.stdin = _RecordingStdin([])
            self.stdout = iter(
                [
                    json.dumps(
                        {
                            "type": "result",
                            "context": {},
                            "outcome": {"status": "success", "suggested_next_ids": [], "context_updates": {}},
                        }
                    )
                    + "\n"
                ]
            )
            self.stderr = io.StringIO("")

        def wait(self, timeout=None):
            return 0

    monkeypatch.setattr("attractor.handlers.execution_container.subprocess.run", fake_run)
    monkeypatch.setattr("attractor.handlers.execution_container.subprocess.Popen", FakePopen)
    graph = parse_dot("digraph G { start [shape=Mdiamond]; done [shape=Msquare]; start -> done; }")
    runner = ContainerizedHandlerRunner(
        graph,
        image="spark-exec:test",
        run_id="run-production-labels",
        working_dir=project_dir,
        run_root=run_root,
    )

    runner("start", "", Context())
    runner.close()

    run_command = commands[0]
    assert "spark.run_id=run-production-labels" in run_command
    assert "spark.execution_mode=container" in run_command
    assert f"spark.project_path={project_dir.resolve()}" in run_command


def test_docker_transport_fails_without_native_fallback_when_docker_missing(monkeypatch, tmp_path: Path) -> None:
    monkeypatch.setattr("attractor.handlers.execution_container.shutil.which", lambda _: None)

    try:
        DockerContainerTransport(
            image="spark-exec:test",
            run_id="run-missing-docker",
            project_path=tmp_path,
            run_root=tmp_path / "runs" / "run-missing-docker",
        )
    except ContainerExecutionError as exc:
        assert "docker CLI was not found" in str(exc)
    else:
        raise AssertionError("container mode silently fell back to native execution")


def test_container_env_uses_provider_allowlist_and_codex_runtime(monkeypatch, tmp_path: Path) -> None:
    monkeypatch.setenv("OPENAI_API_KEY", "openai-secret")
    monkeypatch.setenv("ANTHROPIC_API_KEY", "anthropic-secret")
    monkeypatch.setenv("UNRELATED_SECRET", "nope")
    monkeypatch.setenv("ATTRACTOR_CODEX_RUNTIME_ROOT", str(tmp_path / "codex-runtime"))

    env = _container_env()

    assert env["OPENAI_API_KEY"] == "openai-secret"
    assert env["ANTHROPIC_API_KEY"] == "anthropic-secret"
    assert "UNRELATED_SECRET" not in env
    assert env["ATTRACTOR_CODEX_RUNTIME_ROOT"] == str(tmp_path / "codex-runtime")


def test_docker_transport_cancel_terminates_active_exec_and_removes_container(monkeypatch, tmp_path: Path) -> None:
    commands: list[list[str]] = []

    def fake_run(args, **kwargs):
        commands.append(list(args))
        return SimpleNamespace(returncode=0, stdout="", stderr="")

    monkeypatch.setattr("attractor.handlers.execution_container.subprocess.run", fake_run)
    transport = DockerContainerTransport.__new__(DockerContainerTransport)
    transport.docker = "docker"
    transport.container_id = "container-123"
    transport._proc_lock = __import__("threading").Lock()
    transport._active_proc = _CancelableProc()

    transport.cancel()

    assert transport._active_proc.terminated is True
    assert commands == [["docker", "rm", "-f", "container-123"]]
    assert transport.container_id is None


def test_worker_protocol_serializes_human_gate_request_and_reply(monkeypatch, capsys) -> None:
    monkeypatch.setattr(
        "builtins.input",
        lambda: json.dumps({"type": "human_gate_answer", "answer": {"value": "YES", "selected_values": ["YES"]}}),
    )

    answer = WorkerProtocolInterviewer().ask(
        Question(text="Continue?", type=QuestionType.FREEFORM, stage="gate", metadata={"node_id": "gate"})
    )

    emitted = json.loads(capsys.readouterr().out)
    assert emitted["type"] == "human_gate_request"
    assert emitted["question"]["text"] == "Continue?"
    assert emitted["question"]["stage"] == "gate"
    assert answer.value == "YES"
    assert answer.selected_values == ["YES"]


def test_worker_protocol_delegates_child_run_and_status_lookup(monkeypatch, capsys) -> None:
    graph = parse_dot("digraph G { start [shape=Mdiamond]; done [shape=Msquare]; start -> done; }")
    responses = iter(
        [
            json.dumps(
                {
                    "type": "child_run_result",
                    "result": {
                        "run_id": "child-1",
                        "status": "completed",
                        "outcome": "success",
                        "current_node": "done",
                        "completed_nodes": ["start", "done"],
                        "route_trace": ["start", "done"],
                    },
                }
            ),
            json.dumps(
                {
                    "type": "child_status_result",
                    "result": {
                        "run_id": "child-1",
                        "status": "running",
                        "current_node": "work",
                        "completed_nodes": ["start"],
                    },
                }
            ),
        ]
    )
    monkeypatch.setattr("builtins.input", lambda: next(responses))

    child_result = worker_child_run_launcher(
        ChildRunRequest(
            child_run_id="child-1",
            child_graph=graph,
            child_flow_name="child.dot",
            child_flow_path=Path("/projects/acme/child.dot"),
            child_workdir=Path("/projects/acme"),
            parent_context=Context(values={"context.goal": "ship"}),
            parent_run_id="root-1",
            parent_node_id="manager",
            root_run_id="root-1",
        )
    )
    status_result = worker_child_status_resolver("child-1")
    emitted = [json.loads(line) for line in capsys.readouterr().out.strip().splitlines()]

    assert emitted[0]["type"] == "child_run_request"
    assert emitted[0]["child_run_id"] == "child-1"
    assert emitted[0]["parent_context"] == {"context.goal": "ship"}
    assert emitted[1] == {"type": "child_status_request", "run_id": "child-1"}
    assert child_result.status == "completed"
    assert child_result.completed_nodes == ["start", "done"]
    assert status_result is not None
    assert status_result.current_node == "work"


def test_worker_run_node_streams_events_and_serializes_outcome(monkeypatch, capsys) -> None:
    graph = parse_dot(
        """
        digraph G {
          work [shape=box, label="Do work"];
        }
        """
    )

    class FakeBackend:
        def run(self, *args, emit_event=None, **kwargs):
            if emit_event is not None:
                emit_event("LLMContent", node_id="work", text="chunk")
            return Outcome(status=OutcomeStatus.SUCCESS, notes="done", context_updates={"context.answer": "done"})

    monkeypatch.setattr("attractor.api.codex_backends.build_codergen_backend", lambda *args, **kwargs: FakeBackend())
    monkeypatch.setattr(
        "builtins.input",
        lambda: json.dumps(
            {
                "graph": graph_to_payload(graph),
                "node_id": "work",
                "prompt": "Do work",
                "context": {"context.input": "value"},
                "context_logs": [],
                "logs_root": None,
                "working_dir": ".",
                "backend_name": "provider-router",
                "model": "gpt-test",
            }
        ),
    )

    assert run_worker_node() == 0
    emitted = [json.loads(line) for line in capsys.readouterr().out.strip().splitlines()]

    assert emitted[0]["type"] == "event"
    assert emitted[0]["event_type"] == "LLMContent"
    assert emitted[-1]["type"] == "result"
    assert emitted[-1]["outcome"]["status"] == "success"
    assert emitted[-1]["outcome"]["notes"] == "done"
    assert emitted[-1]["outcome"]["context_updates"]["context.answer"] == "done"


def test_behavioral_flow_executes_tool_and_llm_nodes_through_same_container_runner(tmp_path: Path) -> None:
    graph = parse_dot(
        """
        digraph G {
          start [shape=Mdiamond];
          tool_node [shape=tool, tool.command="true"];
          llm_node [shape=box, label="LLM work"];
          done [shape=Msquare];
          start -> tool_node -> llm_node -> done;
        }
        """
    )
    transport = FakeTransport()
    runner = ContainerizedHandlerRunner(
        graph,
        image="spark-exec:test",
        run_id="run-behavioral",
        working_dir=tmp_path,
        run_root=tmp_path / "runs" / "run-behavioral",
        transport=transport,
    )
    executor = PipelineExecutor(graph, runner, logs_root=str(tmp_path / "runs" / "run-behavioral" / "logs"))

    result = executor.run(Context())

    assert result.status == "completed"
    assert [request["node_id"] for request in transport.requests] == ["start", "tool_node", "llm_node"]
    assert {request["working_dir"] for request in transport.requests} == {str(tmp_path.resolve())}
    assert {request["logs_root"] for request in transport.requests} == {
        str(tmp_path / "runs" / "run-behavioral" / "logs")
    }


class _RecordingStdin:
    def __init__(self, writes: list[str]) -> None:
        self.writes = writes

    def write(self, value: str) -> None:
        self.writes.append(value)

    def flush(self) -> None:
        return None


class _CancelableProc:
    def __init__(self) -> None:
        self.terminated = False
        self.killed = False

    def poll(self) -> None:
        return None

    def terminate(self) -> None:
        self.terminated = True

    def wait(self, timeout=None) -> int:
        if timeout is not None:
            return 0
        return 0

    def kill(self) -> None:
        self.killed = True
