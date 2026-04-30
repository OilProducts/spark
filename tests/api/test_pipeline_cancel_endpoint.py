from __future__ import annotations

from pathlib import Path
import threading
import time

import pytest
from fastapi.testclient import TestClient

import attractor.api.server as server
from attractor.engine.outcome import Outcome
from attractor.engine.outcome import OutcomeStatus
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
        execution_mode="container",
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


def test_cancel_active_container_run_records_canceled_after_transport_interrupt(
    attractor_api_client: TestClient,
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    server.configure_runtime_paths(runs_dir=tmp_path / "runs")
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
            "execution_container_image": "spark-exec:test",
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
