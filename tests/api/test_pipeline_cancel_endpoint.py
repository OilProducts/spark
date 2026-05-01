from __future__ import annotations

from datetime import datetime, timezone
from pathlib import Path
import threading
import time
from typing import Any

import pytest
from fastapi.testclient import TestClient

import attractor.api.server as server
import attractor.execution.metadata as execution_metadata
from attractor.engine.outcome import Outcome
from attractor.engine.outcome import OutcomeStatus
from attractor.execution import (
    WorkerCancelResponse,
    WorkerCleanupResponse,
    WorkerEvent,
    WorkerHealthResponse,
    WorkerInfoResponse,
    WorkerNodeAcceptedResponse,
    WorkerNodeRequest,
    WorkerRunAdmissionResponse,
)
from tests.api._support import (
    close_task_immediately as _close_task_immediately,
    start_pipeline as _start_pipeline,
    wait_for_pipeline_completion as _wait_for_pipeline_completion,
    wait_for_pipeline_terminal_status as _wait_for_pipeline_terminal_status,
)


def test_cancel_pipeline_returns_404_for_unknown_pipeline(
    attractor_api_client: TestClient,
    monkeypatch: pytest.MonkeyPatch, tmp_path: Path
) -> None:
    server.configure_runtime_paths(runs_dir=tmp_path / "runs")

    response = attractor_api_client.post("/pipelines/missing-run/cancel")
    assert response.status_code == 404
    assert response.json()["detail"] == "Unknown pipeline"


def test_cancel_pipeline_requests_cancel_for_active_run(
    attractor_api_client: TestClient,
    monkeypatch: pytest.MonkeyPatch, tmp_path: Path
) -> None:
    server.configure_runtime_paths(runs_dir=tmp_path / "runs")
    monkeypatch.setattr(server.asyncio, "create_task", _close_task_immediately)
    start_payload = _start_pipeline(attractor_api_client, tmp_path / "work")
    run_id = str(start_payload["pipeline_id"])
    transport = _CancelableTransport()
    server._register_shared_container_transport(run_id, transport)

    response = attractor_api_client.post(f"/pipelines/{run_id}/cancel")
    assert response.status_code == 200
    payload = response.json()

    status_response = attractor_api_client.get(f"/pipelines/{run_id}")
    assert status_response.status_code == 200
    status_payload = status_response.json()

    runs_response = attractor_api_client.get("/runs")
    assert runs_response.status_code == 200
    run_rows = runs_response.json()["runs"]
    row = next((entry for entry in run_rows if entry["run_id"] == run_id), None)
    assert row is not None
    assert row["status"] == "cancel_requested"

    assert payload == {"status": "cancel_requested", "pipeline_id": run_id}
    assert transport.cancel_calls == 1
    assert status_payload["status"] == "cancel_requested"
    assert status_payload["last_error"] == "cancel_requested_by_user"
    assert server.RUNTIME.status == "cancel_requested"
    assert server.RUNTIME.last_error == "cancel_requested_by_user"

    history = server.EVENT_HUB.history(run_id)
    assert any(
        event.get("type") == "runtime"
        and event.get("status") == "cancel_requested"
        and event.get("outcome") is None
        and event.get("outcome_reason_code") is None
        and event.get("outcome_reason_message") is None
        and event.get("run_id") == run_id
        and isinstance(event.get("sequence"), int)
        and isinstance(event.get("emitted_at"), str)
        for event in history
    )
    assert any(
        event.get("type") == "log"
        and event.get("msg") == "[System] Cancel requested. Stopping after current node."
        and event.get("run_id") == run_id
        and isinstance(event.get("sequence"), int)
        and isinstance(event.get("emitted_at"), str)
        for event in history
    )


def test_cancel_pipeline_dispatches_active_remote_runner_cancel(
    attractor_api_client: TestClient,
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    server.configure_runtime_paths(runs_dir=tmp_path / "runs")
    monkeypatch.setattr(server.asyncio, "create_task", _close_task_immediately)
    start_payload = _start_pipeline(attractor_api_client, tmp_path / "work")
    run_id = str(start_payload["pipeline_id"])
    runner = _CancelableRunner()
    with server.ACTIVE_RUNS_LOCK:
        assert run_id in server.ACTIVE_RUNS
        server.ACTIVE_RUNS[run_id].runner = runner

    response = attractor_api_client.post(f"/pipelines/{run_id}/cancel")

    assert response.status_code == 200
    assert response.json() == {"status": "cancel_requested", "pipeline_id": run_id}
    assert runner.cancel_calls == 1


def test_cancel_remote_worker_pipeline_records_worker_cancel_terminal_and_stops_node_submission(
    attractor_api_client: TestClient,
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    server.configure_runtime_paths(runs_dir=tmp_path / "runs")
    _write_remote_profile_config(tmp_path)
    controller = _RemoteCancelController(cancel_terminal="run_canceled")
    monkeypatch.setattr(execution_metadata, "RemoteWorkerClient", controller.factory)

    response = attractor_api_client.post(
        "/pipelines",
        json={
            "flow_content": "digraph G { start [shape=box] second [shape=box] done [shape=Msquare] start -> second -> done }",
            "working_directory": str(tmp_path / "control" / "project"),
            "execution_profile_id": "remote-fast",
        },
    )
    assert response.status_code == 200, response.text
    run_id = str(response.json()["run_id"])
    controller.wait_for_node_requests(1)

    cancel_response = attractor_api_client.post(f"/pipelines/{run_id}/cancel")
    assert cancel_response.status_code == 200
    assert cancel_response.json() == {"status": "cancel_requested", "pipeline_id": run_id}

    final_payload = _wait_for_pipeline_completion(attractor_api_client, run_id)

    assert final_payload["status"] == "canceled"
    assert controller.cancel_requests == [run_id]
    assert [request.node_id for request in controller.node_requests] == ["start"]
    persisted_events = server._read_persisted_run_events(run_id)
    remote_events = [event for event in persisted_events if event["type"] == "remote_worker_event"]
    assert [event["event"] for event in remote_events] == [
        "run_ready",
        "node_started",
        "run_canceling",
        "run_canceled",
    ]
    history_events = [event for event in server.EVENT_HUB.history(run_id) if event.get("type") == "remote_worker_event"]
    assert [event["event"] for event in history_events] == [
        "run_ready",
        "node_started",
        "run_canceling",
        "run_canceled",
    ]


def test_cancel_remote_worker_pipeline_preserves_worker_failure_terminal_precedence(
    attractor_api_client: TestClient,
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    server.configure_runtime_paths(runs_dir=tmp_path / "runs")
    _write_remote_profile_config(tmp_path)
    controller = _RemoteCancelController(cancel_terminal="node_failed")
    monkeypatch.setattr(execution_metadata, "RemoteWorkerClient", controller.factory)

    response = attractor_api_client.post(
        "/pipelines",
        json={
            "flow_content": "digraph G { start [shape=box] second [shape=box] done [shape=Msquare] start -> second -> done }",
            "working_directory": str(tmp_path / "control" / "project"),
            "execution_profile_id": "remote-fast",
        },
    )
    assert response.status_code == 200, response.text
    run_id = str(response.json()["run_id"])
    controller.wait_for_node_requests(1)

    cancel_response = attractor_api_client.post(f"/pipelines/{run_id}/cancel")
    assert cancel_response.status_code == 200
    final_payload = _wait_for_pipeline_completion(attractor_api_client, run_id)

    assert final_payload["status"] == "failed"
    assert "worker process crashed during cancellation" in str(final_payload["last_error"])
    assert controller.cancel_requests == [run_id]
    assert [request.node_id for request in controller.node_requests] == ["start"]
    persisted_events = server._read_persisted_run_events(run_id)
    remote_events = [event for event in persisted_events if event["type"] == "remote_worker_event"]
    assert [event["event"] for event in remote_events] == ["run_ready", "node_started", "run_canceling", "node_failed"]


def test_cancel_pipeline_cancels_child_run_shared_root_container(
    attractor_api_client: TestClient,
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    server.configure_runtime_paths(runs_dir=tmp_path / "runs")
    monkeypatch.setattr(server.asyncio, "create_task", _close_task_immediately)
    start_payload = _start_pipeline(attractor_api_client, tmp_path / "work")
    root_run_id = str(start_payload["pipeline_id"])
    child_run_id = "child-cancel-container"
    server._record_run_start(
        child_run_id,
        "child.dot",
        str(tmp_path / "work"),
        "gpt-test",
        parent_run_id=root_run_id,
        parent_node_id="manager",
        root_run_id=root_run_id,
        execution_mode="local_container",
        execution_container_image="spark-exec:test",
    )
    with server.ACTIVE_RUNS_LOCK:
        server.ACTIVE_RUNS[child_run_id] = server.ActiveRun(
            run_id=child_run_id,
            flow_name="child.dot",
            working_directory=str(tmp_path / "work"),
            model="gpt-test",
            status="running",
            control=server.ExecutionControl(),
        )
    transport = _CancelableTransport()
    server._register_shared_container_transport(root_run_id, transport)

    response = attractor_api_client.post(f"/pipelines/{child_run_id}/cancel")

    assert response.status_code == 200
    assert response.json() == {"status": "cancel_requested", "pipeline_id": child_run_id}
    assert transport.cancel_calls == 1


def test_cancel_pipeline_ignores_non_running_known_pipeline(
    attractor_api_client: TestClient,
    monkeypatch: pytest.MonkeyPatch, tmp_path: Path
) -> None:
    server.configure_runtime_paths(runs_dir=tmp_path / "runs")
    start_payload = _start_pipeline(attractor_api_client, tmp_path / "work")
    run_id = str(start_payload["pipeline_id"])
    final_status = _wait_for_pipeline_terminal_status(attractor_api_client, run_id)
    assert final_status == "completed"

    response = attractor_api_client.post(f"/pipelines/{run_id}/cancel")
    assert response.status_code == 200
    payload = response.json()

    assert payload == {"status": "ignored", "pipeline_id": run_id}


class _CancelableTransport:
    def __init__(self) -> None:
        self.cancel_calls = 0

    def cancel(self) -> None:
        self.cancel_calls += 1


class _CancelableRunner:
    def __init__(self) -> None:
        self.cancel_calls = 0

    def cancel(self) -> None:
        self.cancel_calls += 1


def _write_remote_profile_config(tmp_path: Path) -> None:
    config_dir = server.get_runtime_paths().config_dir
    config_dir.mkdir(parents=True, exist_ok=True)
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
        control_project_root = "{tmp_path / "control"}"
        worker_project_root = "{tmp_path / "worker-project"}"
        worker_runtime_root = "{tmp_path / "worker-runtime"}"
        capabilities = ["shell"]
        """,
        encoding="utf-8",
    )


class _RemoteCancelController:
    def __init__(self, *, cancel_terminal: str) -> None:
        self.cancel_terminal = cancel_terminal
        self.node_requests: list[WorkerNodeRequest] = []
        self.cancel_requests: list[str] = []
        self.cleanup_requests: list[str] = []
        self._events: list[WorkerEvent | None] = []
        self._sequence = 0
        self._condition = threading.Condition()

    def factory(self, _worker):
        return _RemoteCancelClient(self)

    def event(
        self,
        run_id: str,
        event_type: str,
        payload: dict[str, Any],
        *,
        node_id: str | None = None,
    ) -> WorkerEvent:
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

    def push(self, event: WorkerEvent | None) -> None:
        with self._condition:
            self._events.append(event)
            self._condition.notify_all()

    def next_event(self) -> WorkerEvent | None:
        with self._condition:
            while not self._events:
                self._condition.wait(timeout=0.25)
            return self._events.pop(0)

    def append_node_request(self, request: WorkerNodeRequest) -> None:
        with self._condition:
            self.node_requests.append(request)
            self._condition.notify_all()

    def wait_for_node_requests(self, count: int) -> None:
        deadline = time.monotonic() + 5
        with self._condition:
            while len(self.node_requests) < count and time.monotonic() < deadline:
                self._condition.wait(timeout=0.05)
            if len(self.node_requests) < count:
                raise AssertionError(f"timed out waiting for {count} remote node request(s)")


class _RemoteCancelClient:
    def __init__(self, controller: _RemoteCancelController) -> None:
        self.controller = controller

    def __enter__(self):
        return self

    def __exit__(self, *_exc_info):
        return None

    def health(self) -> WorkerHealthResponse:
        return WorkerHealthResponse(
            worker_id="worker-a",
            worker_version="1.2.3",
            protocol_version="v1",
            status="ok",
            capabilities={"shell": True},
        )

    def worker_info(self) -> WorkerInfoResponse:
        return WorkerInfoResponse(
            worker_id="worker-a",
            worker_version="1.2.3",
            protocol_version="v1",
            status="ok",
            capabilities={"shell": True},
        )

    def admit_run(self, request) -> WorkerRunAdmissionResponse:
        return WorkerRunAdmissionResponse(
            run_id=request.run_id,
            worker_id="worker-a",
            status="ready",
            event_url=f"/v1/runs/{request.run_id}/events",
            last_sequence=0,
            accepted=True,
        )

    def stream_events(self, run_id: str, *, after: int | None = None, last_event_id: str | None = None):
        yield self.controller.event(run_id, "run_ready", {"status": "ready"})
        while True:
            event = self.controller.next_event()
            if event is None:
                return
            yield event

    def submit_node(self, run_id: str, request: WorkerNodeRequest) -> WorkerNodeAcceptedResponse:
        self.controller.append_node_request(request)
        self.controller.push(
            self.controller.event(
                run_id,
                "node_started",
                {"status": "started", "node_execution_id": request.node_execution_id},
                node_id=request.node_id,
            )
        )
        return WorkerNodeAcceptedResponse(
            run_id=run_id,
            node_execution_id=request.node_execution_id or request.node_id,
            node_id=request.node_id,
            attempt=request.attempt,
        )

    def cancel_run(self, run_id: str) -> WorkerCancelResponse:
        self.controller.cancel_requests.append(run_id)
        self.controller.push(self.controller.event(run_id, "run_canceling", {"status": "canceling"}))
        if self.controller.cancel_terminal == "run_canceled":
            self.controller.push(
                self.controller.event(run_id, "run_canceled", {"status": "canceled", "message": "aborted_by_user"})
            )
        elif self.controller.cancel_terminal == "node_failed":
            request = self.controller.node_requests[-1]
            self.controller.push(
                self.controller.event(
                    run_id,
                    "node_failed",
                    {
                        "node_execution_id": request.node_execution_id,
                        "code": "process_crashed",
                        "message": "worker process crashed during cancellation",
                    },
                    node_id=request.node_id,
                )
            )
        self.controller.push(None)
        return WorkerCancelResponse(run_id=run_id, status="canceling")

    def cleanup_run(self, run_id: str) -> WorkerCleanupResponse:
        self.controller.cleanup_requests.append(run_id)
        return WorkerCleanupResponse(run_id=run_id, status="closed", deleted=True)


def test_cancel_active_container_run_records_canceled_after_transport_interrupt(
    attractor_api_client: TestClient,
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    server.configure_runtime_paths(runs_dir=tmp_path / "runs")
    config_dir = server.get_runtime_paths().config_dir
    config_dir.mkdir(parents=True, exist_ok=True)
    (config_dir / "execution-profiles.toml").write_text(
        """
        [profiles.local-test]
        mode = "local_container"
        label = "Local Test"
        image = "spark-exec:test"
        """,
        encoding="utf-8",
    )
    runners: list[_BlockingContainerRunner] = []

    def fake_container_runner(*args, **kwargs):
        runner = _BlockingContainerRunner(*args, **kwargs)
        runners.append(runner)
        return runner

    monkeypatch.setattr(server, "ContainerizedHandlerRunner", fake_container_runner)

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
            "execution_profile_id": "local-test",
        },
    )
    assert response.status_code == 200
    run_id = str(response.json()["pipeline_id"])

    assert runners
    transport = runners[0].transport
    assert transport.started.wait(timeout=2)

    cancel_response = attractor_api_client.post(f"/pipelines/{run_id}/cancel")
    assert cancel_response.status_code == 200
    assert cancel_response.json() == {"status": "cancel_requested", "pipeline_id": run_id}

    final_payload = _wait_for_pipeline_completion(attractor_api_client, run_id, attempts=400)
    runs = attractor_api_client.get("/runs").json()["runs"]
    row = next(run for run in runs if run["run_id"] == run_id)

    assert final_payload["status"] == "canceled"
    assert row["status"] == "canceled"
    assert transport.cancel_calls == 1
    assert transport.close_calls >= 1


class _BlockingContainerRunner:
    def __init__(self, *args, transport=None, **kwargs) -> None:
        self.transport = transport or _BlockingContainerTransport()
        self.logs_root = None

    def set_logs_root(self, logs_root) -> None:
        self.logs_root = logs_root

    def set_control(self, control) -> None:
        self.control = control

    def close(self) -> None:
        self.transport.close()

    def __call__(self, node_id: str, prompt: str, context, *, emit_event=None):
        self.transport.run_node({"node_id": node_id}, None)
        return Outcome(status=OutcomeStatus.SUCCESS)


class _BlockingContainerTransport:
    def __init__(self) -> None:
        self.started = threading.Event()
        self.canceled = threading.Event()
        self.cancel_calls = 0
        self.close_calls = 0

    def run_node(self, request, callbacks) -> dict:
        self.started.set()
        assert self.canceled.wait(timeout=2)
        raise RuntimeError("container exec terminated")

    def cancel(self) -> None:
        self.cancel_calls += 1
        self.canceled.set()

    def close(self) -> None:
        self.close_calls += 1


def test_cancel_pipeline_stops_nested_manager_loop_child_execution(
    attractor_api_client: TestClient,
    tmp_path: Path,
) -> None:
    server.configure_runtime_paths(runs_dir=tmp_path / "runs")

    child_dot_path = tmp_path / "child.dot"
    child_dot_path.write_text(
        """
        digraph Child {
            start [shape=Mdiamond]
            first [shape=parallelogram, tool.command="sleep 0.5"]
            second [shape=parallelogram, tool.command="sleep 0.5"]
            done [shape=Msquare]

            start -> first -> second -> done
        }
        """,
        encoding="utf-8",
    )

    start_payload = _start_pipeline(
        attractor_api_client,
        tmp_path / "work",
        flow_content=f"""
        digraph Parent {{
            graph [stack.child_dotfile="{child_dot_path}"]
            start [shape=Mdiamond]
            manager [shape=house, manager.poll_interval=0ms, manager.max_cycles=1, manager.actions=""]
            done [shape=Msquare]

            start -> manager -> done
        }}
        """,
    )
    run_id = str(start_payload["pipeline_id"])

    child_run_id = ""
    for _ in range(200):
        events = server._read_persisted_run_events(run_id)
        child_started = next(
            (event for event in events if event.get("type") == "ChildRunStarted"),
            None,
        )
        if child_started is not None:
            child_run_id = str(child_started.get("child_run_id") or "")
            break
        time.sleep(0.01)
    else:
        raise AssertionError("timed out waiting for child run start before cancel")

    for _ in range(200):
        child_events = server._read_persisted_run_events(child_run_id)
        if any(
            event.get("type") == "StageStarted"
            and event.get("source_scope") == "root"
            and event.get("node_id") == "first"
            for event in child_events
        ):
            break
        time.sleep(0.01)
    else:
        raise AssertionError("timed out waiting for child stage start before cancel")

    cancel_response = attractor_api_client.post(f"/pipelines/{run_id}/cancel")
    assert cancel_response.status_code == 200
    assert cancel_response.json() == {"status": "cancel_requested", "pipeline_id": run_id}

    final_payload = _wait_for_pipeline_completion(attractor_api_client, run_id, attempts=800)

    assert final_payload["status"] == "canceled"

    child_events = server._read_persisted_run_events(child_run_id)
    child_started_nodes = [
        str(event.get("node_id"))
        for event in child_events
        if event.get("type") == "StageStarted" and event.get("source_scope") == "root"
    ]
    assert "first" in child_started_nodes
    assert "second" not in child_started_nodes
    events = server._read_persisted_run_events(run_id)
    assert any(
        event.get("type") == "runtime" and event.get("status") == "canceled"
        for event in events
    )


def test_cancel_pipeline_stops_first_class_child_run_by_child_run_id(
    attractor_api_client: TestClient,
    tmp_path: Path,
) -> None:
    server.configure_runtime_paths(runs_dir=tmp_path / "runs")

    child_dot_path = tmp_path / "child.dot"
    child_dot_path.write_text(
        """
        digraph Child {
            start [shape=Mdiamond]
            first [shape=parallelogram, tool.command="sleep 0.5"]
            second [shape=parallelogram, tool.command="sleep 0.5"]
            done [shape=Msquare]

            start -> first -> second -> done
        }
        """,
        encoding="utf-8",
    )

    start_payload = _start_pipeline(
        attractor_api_client,
        tmp_path / "work",
        flow_content=f"""
        digraph Parent {{
            graph [stack.child_dotfile="{child_dot_path}"]
            start [shape=Mdiamond]
            manager [shape=house, manager.poll_interval=0ms, manager.max_cycles=1, manager.actions=""]
            done [shape=Msquare]

            start -> manager
            manager -> done [condition="outcome=success"]
        }}
        """,
    )
    parent_run_id = str(start_payload["pipeline_id"])

    child_run_id = ""
    for _ in range(200):
        events = server._read_persisted_run_events(parent_run_id)
        child_started = next(
            (event for event in events if event.get("type") == "ChildRunStarted"),
            None,
        )
        if child_started is not None:
            child_run_id = str(child_started.get("child_run_id") or "")
            break
        time.sleep(0.01)
    else:
        raise AssertionError("timed out waiting for child run start before cancel")

    for _ in range(200):
        child_events = server._read_persisted_run_events(child_run_id)
        if any(
            event.get("type") == "StageStarted"
            and event.get("source_scope") == "root"
            and event.get("node_id") == "first"
            for event in child_events
        ):
            break
        time.sleep(0.01)
    else:
        raise AssertionError("timed out waiting for child stage start before cancel")

    cancel_response = attractor_api_client.post(f"/pipelines/{child_run_id}/cancel")
    assert cancel_response.status_code == 200
    assert cancel_response.json() == {"status": "cancel_requested", "pipeline_id": child_run_id}

    child_final_payload = _wait_for_pipeline_completion(attractor_api_client, child_run_id, attempts=800)
    parent_final_payload = _wait_for_pipeline_completion(attractor_api_client, parent_run_id, attempts=800)

    assert child_final_payload["status"] == "canceled"
    assert "second" not in child_final_payload["completed_nodes"]
    assert parent_final_payload["status"] == "failed"
    assert parent_final_payload["last_error"] == "aborted_by_user"

    child_events = server._read_persisted_run_events(child_run_id)
    child_started_nodes = [
        str(event.get("node_id"))
        for event in child_events
        if event.get("type") == "StageStarted" and event.get("source_scope") == "root"
    ]
    assert "first" in child_started_nodes
    assert "second" not in child_started_nodes

    parent_context = attractor_api_client.get(f"/pipelines/{parent_run_id}/context").json()["context"]
    assert parent_context["context.stack.child.run_id"] == child_run_id
    assert parent_context["context.stack.child.status"] == "canceled"

    parent_events = server._read_persisted_run_events(parent_run_id)
    assert any(
        event.get("type") == "ChildRunCompleted"
        and event.get("child_run_id") == child_run_id
        and event.get("status") == "canceled"
        for event in parent_events
    )

    runs = attractor_api_client.get("/runs").json()["runs"]
    child_row = next(run for run in runs if run["run_id"] == child_run_id)
    parent_row = next(run for run in runs if run["run_id"] == parent_run_id)
    assert child_row["status"] == "canceled"
    assert parent_row["status"] == "failed"
