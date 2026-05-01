from __future__ import annotations

from collections.abc import Iterator
from datetime import datetime, timezone
import threading
import time
from typing import Any

import pytest

from attractor.engine import Context, Outcome, OutcomeStatus
from attractor.engine.outcome import FailureKind
from attractor.handlers.base import ChildRunResult
from attractor.interviewer.models import Answer, Question
from attractor.execution import (
    ExecutionLaunchError,
    ExecutionProfile,
    RemoteActiveRunFailed,
    RemoteHandlerRunner,
    RemoteLaunchAdmission,
    RemotePreparationFailed,
    WorkerHealthResponse,
    WorkerInfoResponse,
    WorkerNodeAcceptedResponse,
    WorkerNodeRequest,
    WorkerCancelResponse,
    WorkerCleanupResponse,
    WorkerProfile,
    WorkerRunAdmissionResponse,
    WorkerRunSnapshot,
)
from attractor.execution.errors import WorkerAPIError
from attractor.execution.worker_models import WorkerEvent


def _profile() -> ExecutionProfile:
    return ExecutionProfile(
        id="remote-fast",
        mode="remote_worker",
        worker_id="worker-a",
        image="spark-worker:latest",
        capabilities={"shell": True},
    )


def _worker() -> WorkerProfile:
    return WorkerProfile(
        id="worker-a",
        label="Worker A",
        base_url="https://worker.example",
        auth_token_env="SPARK_WORKER_TOKEN",
    )


def _admission(*, status: str = "preparing") -> RemoteLaunchAdmission:
    return RemoteLaunchAdmission(
        metadata=_launch_metadata(),
        health=WorkerHealthResponse(
            worker_id="worker-a",
            worker_version="1.2.3",
            protocol_version="v1",
            status="ok",
            capabilities={"shell": True},
        ),
        worker_info=WorkerInfoResponse(
            worker_id="worker-a",
            worker_version="1.2.3",
            protocol_version="v1",
            status="ok",
            capabilities={"shell": True},
        ),
        admission=WorkerRunAdmissionResponse(
            run_id="run-1",
            worker_id="worker-a",
            status=status,
            event_url="/v1/runs/run-1/events",
            last_sequence=2,
        ),
    )


def _launch_metadata():
    from attractor.execution.models import build_launch_metadata

    return build_launch_metadata(_profile(), mapped_worker_project_path="/srv/project", worker_runtime_root="/srv/runtime")


class ControlledRemoteClient:
    def __init__(self, controller: RemoteController) -> None:
        self.controller = controller

    def __enter__(self) -> ControlledRemoteClient:
        return self

    def __exit__(self, *_exc_info: object) -> None:
        return None

    def stream_events(self, run_id: str, *, after: int | None = None, last_event_id: str | None = None) -> Iterator[WorkerEvent]:
        self.controller.calls.append(("GET", f"/v1/runs/{run_id}/events", {"after": after, "last_event_id": last_event_id}))
        while True:
            event = self.controller.next_event()
            if event is None:
                return
            yield event

    def submit_node(self, run_id: str, request: WorkerNodeRequest) -> WorkerNodeAcceptedResponse:
        self.controller.calls.append(("POST", f"/v1/runs/{run_id}/nodes", request))
        return WorkerNodeAcceptedResponse(
            run_id=run_id,
            node_execution_id=request.node_execution_id or request.node_id,
            node_id=request.node_id,
            attempt=request.attempt,
        )

    def cancel_run(self, run_id: str) -> WorkerCancelResponse:
        self.controller.calls.append(("POST", f"/v1/runs/{run_id}/cancel", None))
        if self.controller.cancel_error is not None:
            raise self.controller.cancel_error
        return WorkerCancelResponse(run_id=run_id, status="canceling")

    def cleanup_run(self, run_id: str) -> WorkerCleanupResponse:
        self.controller.calls.append(("DELETE", f"/v1/runs/{run_id}", None))
        if self.controller.cleanup_error is not None:
            raise self.controller.cleanup_error
        return WorkerCleanupResponse(run_id=run_id, status="closed", deleted=True)

    def get_run(self, run_id: str) -> WorkerRunSnapshot:
        self.controller.calls.append(("GET", f"/v1/runs/{run_id}", None))
        if self.controller.snapshot is None:
            raise AssertionError("snapshot was not configured")
        return self.controller.snapshot

    def answer_human_gate(self, run_id: str, gate_id: str, request) -> Any:
        self.controller.calls.append(("POST", f"/v1/runs/{run_id}/human-gates/{gate_id}/answer", request))
        if self.controller.callback_errors:
            raise self.controller.callback_errors.pop(0)
        if self.controller.callback_error is not None:
            raise self.controller.callback_error
        return _callback_response(run_id, gate_id)

    def child_run_result(self, run_id: str, request_id: str, request) -> Any:
        self.controller.calls.append(("POST", f"/v1/runs/{run_id}/child-runs/{request_id}/result", request))
        if self.controller.callback_errors:
            raise self.controller.callback_errors.pop(0)
        if self.controller.callback_error is not None:
            raise self.controller.callback_error
        return _callback_response(run_id, request_id)

    def child_status_result(self, run_id: str, request_id: str, request) -> Any:
        self.controller.calls.append(("POST", f"/v1/runs/{run_id}/child-status/{request_id}/result", request))
        if self.controller.callback_errors:
            raise self.controller.callback_errors.pop(0)
        if self.controller.callback_error is not None:
            raise self.controller.callback_error
        return _callback_response(run_id, request_id)


class RemoteController:
    def __init__(self) -> None:
        self.calls: list[tuple[str, str, Any]] = []
        self.events: list[WorkerEvent | None] = []
        self.snapshot: WorkerRunSnapshot | None = None
        self.callback_error: BaseException | None = None
        self.callback_errors: list[BaseException] = []
        self.cancel_error: BaseException | None = None
        self.cleanup_error: BaseException | None = None
        self.stream_close_count = 0
        self.condition = threading.Condition()

    def factory(self, _worker: WorkerProfile) -> ControlledRemoteClient:
        return ControlledRemoteClient(self)

    def push(self, event: WorkerEvent | None) -> None:
        with self.condition:
            self.events.append(event)
            self.condition.notify_all()

    def next_event(self) -> WorkerEvent | None:
        with self.condition:
            while not self.events:
                self.condition.wait(timeout=0.25)
            event = self.events.pop(0)
            if event is None:
                self.stream_close_count += 1
                self.condition.notify_all()
            return event

    def wait_for_post(self) -> None:
        deadline = time.monotonic() + 2
        while time.monotonic() < deadline:
            if any(call[0] == "POST" for call in self.calls):
                return
            time.sleep(0.01)
        raise AssertionError("timed out waiting for node POST")

    def wait_for_stream_close(self) -> None:
        deadline = time.monotonic() + 2
        with self.condition:
            while self.stream_close_count == 0:
                remaining = deadline - time.monotonic()
                if remaining <= 0:
                    raise AssertionError("timed out waiting for stream close")
                self.condition.wait(timeout=remaining)


def _event(event_type: str, sequence: int, payload: dict[str, Any] | None = None) -> WorkerEvent:
    return WorkerEvent(
        run_id="run-1",
        sequence=sequence,
        event_type=event_type,
        timestamp=datetime.now(timezone.utc),
        worker_id="worker-a",
        execution_profile_id="remote-fast",
        payload=dict(payload or {}),
        node_id="plan" if event_type.startswith("node_") else None,
        node_attempt=1 if event_type.startswith("node_") else None,
    )


def _snapshot(events: list[WorkerEvent], *, status: str = "ready") -> WorkerRunSnapshot:
    return WorkerRunSnapshot(
        run_id="run-1",
        status=status,
        execution_profile_id="remote-fast",
        protocol_version="v1",
        worker_id="worker-a",
        worker_version="1.2.3",
        mapped_project_path="/srv/project",
        last_sequence=events[-1].sequence if events else 0,
        events=events,
    )


def _callback_response(run_id: str, request_id: str) -> Any:
    from attractor.execution.worker_models import WorkerCallbackResponse

    return WorkerCallbackResponse(run_id=run_id, request_id=request_id, status="accepted")


class RecordingInterviewer:
    def __init__(self, answer: Answer) -> None:
        self.answer = answer
        self.questions: list[Question] = []

    def ask(self, question: Question) -> Answer:
        self.questions.append(question)
        return self.answer


def test_remote_runner_opens_event_stream_and_does_not_post_node_before_run_ready() -> None:
    controller = RemoteController()
    surfaced: list[dict[str, Any]] = []
    runner = RemoteHandlerRunner(
        profile=_profile(),
        worker=_worker(),
        admission=_admission(),
        emit=surfaced.append,
        client_factory=controller.factory,
    )
    results: list[Outcome] = []
    thread = threading.Thread(target=lambda: results.append(runner("plan", "Plan", Context())), daemon=True)

    thread.start()
    time.sleep(0.05)

    assert controller.calls == [("GET", "/v1/runs/run-1/events", {"after": 2, "last_event_id": None})]

    controller.push(_event("run_ready", 3, {"status": "ready"}))
    controller.wait_for_post()
    controller.push(
        _event(
            "node_result",
            4,
            {
                "outcome": {
                    "status": "success",
                    "context_updates": {"context.answer": 42},
                }
            },
        )
    )
    controller.push(None)
    thread.join(timeout=2)

    assert [call[0] for call in controller.calls] == ["GET", "POST"]
    post_request = controller.calls[1][2]
    assert isinstance(post_request, WorkerNodeRequest)
    assert post_request.node_execution_id == "run-1:plan:1"
    assert post_request.node_id == "plan"
    assert post_request.attempt == 1
    assert post_request.context == {}
    assert post_request.payload["prompt"] == "Plan"
    assert post_request.payload["node_metadata"] == {"node_id": "plan", "attrs": {}}
    assert post_request.payload["runtime_references"]["run_id"] == "run-1"
    assert post_request.payload["runtime_references"]["execution_profile_id"] == "remote-fast"
    assert post_request.payload["runtime_references"]["worker_id"] == "worker-a"
    assert post_request.payload["options"] == {"max_attempts": 0}
    assert results == [Outcome(status=OutcomeStatus.SUCCESS, context_updates={"context.answer": 42})]
    assert [event["event"] for event in surfaced] == ["run_ready", "node_result"]


def test_remote_runner_admission_ready_still_waits_for_run_ready_event_before_node_post() -> None:
    controller = RemoteController()
    runner = RemoteHandlerRunner(
        profile=_profile(),
        worker=_worker(),
        admission=_admission(status="ready"),
        emit=lambda _event: None,
        client_factory=controller.factory,
    )
    results: list[Outcome] = []
    thread = threading.Thread(target=lambda: results.append(runner("plan", "Plan", Context())), daemon=True)

    thread.start()
    time.sleep(0.05)

    assert controller.calls == [("GET", "/v1/runs/run-1/events", {"after": 2, "last_event_id": None})]
    assert not any(call[0] == "POST" for call in controller.calls)

    controller.push(_event("run_ready", 3, {"status": "ready"}))
    controller.wait_for_post()
    controller.push(_event("node_result", 4, {"outcome": {"status": "success"}}))
    controller.push(None)
    thread.join(timeout=2)

    assert results == [Outcome(status=OutcomeStatus.SUCCESS)]
    assert [call[0] for call in controller.calls] == ["GET", "POST"]


def test_remote_runner_applies_node_result_context_payload_before_returning_outcome() -> None:
    controller = RemoteController()
    runner = RemoteHandlerRunner(
        profile=_profile(),
        worker=_worker(),
        admission=_admission(),
        emit=lambda _event: None,
        client_factory=controller.factory,
    )
    context = Context()
    results: list[Outcome] = []
    thread = threading.Thread(target=lambda: results.append(runner("plan", "Plan", context)), daemon=True)

    thread.start()
    time.sleep(0.05)
    controller.push(_event("run_ready", 3, {"status": "ready"}))
    controller.wait_for_post()
    controller.push(
        _event(
            "node_result",
            4,
            {
                "outcome": {"status": "success", "context_updates": {}},
                "context": {"context.remote.route": "branch-only"},
            },
        )
    )
    controller.push(None)
    thread.join(timeout=2)

    assert results == [Outcome(status=OutcomeStatus.SUCCESS)]
    assert context.get("context.remote.route") == "branch-only"


def test_remote_runner_treats_run_failed_before_ready_as_preparation_failure_without_node_post() -> None:
    controller = RemoteController()
    runner = RemoteHandlerRunner(
        profile=_profile(),
        worker=_worker(),
        admission=_admission(),
        emit=lambda _event: None,
        client_factory=controller.factory,
    )
    errors: list[BaseException] = []
    thread = threading.Thread(
        target=lambda: _capture_error(errors, lambda: runner("plan", "Plan", Context())),
        daemon=True,
    )

    thread.start()
    time.sleep(0.05)
    controller.push(_event("run_failed", 3, {"code": "prepare_failed", "message": "image pull failed"}))
    controller.push(None)
    thread.join(timeout=2)

    assert len(errors) == 1
    assert isinstance(errors[0], RemotePreparationFailed)
    assert "image pull failed" in str(errors[0])
    assert [call[0] for call in controller.calls] == ["GET"]


def test_remote_runner_fails_when_event_stream_closes_before_run_ready() -> None:
    controller = RemoteController()
    surfaced: list[dict[str, Any]] = []
    runner = RemoteHandlerRunner(
        profile=_profile(),
        worker=_worker(),
        admission=_admission(),
        emit=surfaced.append,
        client_factory=controller.factory,
    )
    errors: list[BaseException] = []
    thread = threading.Thread(
        target=lambda: _capture_error(errors, lambda: runner("plan", "Plan", Context())),
        daemon=True,
    )

    thread.start()
    time.sleep(0.05)
    controller.push(None)
    thread.join(timeout=2)

    assert not thread.is_alive()
    assert len(errors) == 1
    assert isinstance(errors[0], RemotePreparationFailed)
    assert "closed before run_ready" in str(errors[0])
    assert [call[0] for call in controller.calls] == ["GET"]
    assert [event["event"] for event in surfaced] == ["run_failed"]
    assert surfaced[-1]["payload"]["code"] == "worker_event_stream_closed"


def test_remote_runner_preserves_modeled_fail_outcome_from_node_result() -> None:
    controller = RemoteController()
    runner = RemoteHandlerRunner(
        profile=_profile(),
        worker=_worker(),
        admission=_admission(),
        emit=lambda _event: None,
        client_factory=controller.factory,
    )
    results: list[Outcome] = []
    thread = threading.Thread(target=lambda: results.append(runner("plan", "Plan", Context())), daemon=True)

    thread.start()
    time.sleep(0.05)
    controller.push(_event("run_ready", 3, {"status": "ready"}))
    controller.wait_for_post()
    controller.push(
        _event(
            "node_result",
            4,
            {
                "outcome": {
                    "status": "fail",
                    "failure_reason": "modeled fail",
                    "failure_kind": "business",
                    "retryable": False,
                    "context_updates": {"context.needs_fix": True},
                    "notes": "worker returned a valid Outcome",
                },
                "context": {"context.needs_fix": True},
                "runtime_metadata": {"runtime_id": "runtime-run-1"},
            },
        )
    )
    controller.push(None)
    thread.join(timeout=2)

    assert results == [
        Outcome(
            status=OutcomeStatus.FAIL,
            context_updates={"context.needs_fix": True},
            failure_reason="modeled fail",
            notes="worker returned a valid Outcome",
            retryable=False,
            failure_kind=FailureKind.BUSINESS,
        )
    ]
    assert [call[0] for call in controller.calls] == ["GET", "POST"]


def test_remote_runner_converts_node_failed_event_to_runtime_failure() -> None:
    controller = RemoteController()
    runner = RemoteHandlerRunner(
        profile=_profile(),
        worker=_worker(),
        admission=_admission(),
        emit=lambda _event: None,
        client_factory=controller.factory,
    )
    errors: list[BaseException] = []
    thread = threading.Thread(
        target=lambda: _capture_error(errors, lambda: runner("plan", "Plan", Context())),
        daemon=True,
    )

    thread.start()
    time.sleep(0.05)
    controller.push(_event("run_ready", 3, {"status": "ready"}))
    controller.wait_for_post()
    controller.push(_event("node_failed", 4, {"code": "process_crashed", "message": "worker process crashed"}))
    controller.push(None)
    thread.join(timeout=2)

    assert len(errors) == 1
    assert isinstance(errors[0], ExecutionLaunchError)
    assert "worker process crashed" in str(errors[0])


def test_remote_runner_converts_run_failed_after_ready_to_runtime_failure() -> None:
    controller = RemoteController()
    runner = RemoteHandlerRunner(
        profile=_profile(),
        worker=_worker(),
        admission=_admission(),
        emit=lambda _event: None,
        client_factory=controller.factory,
    )
    errors: list[BaseException] = []
    thread = threading.Thread(
        target=lambda: _capture_error(errors, lambda: runner("plan", "Plan", Context())),
        daemon=True,
    )

    thread.start()
    time.sleep(0.05)
    controller.push(_event("run_ready", 3, {"status": "ready"}))
    controller.wait_for_post()
    controller.push(_event("run_failed", 4, {"code": "runtime_lost", "message": "worker lost runtime"}))
    controller.push(None)
    thread.join(timeout=2)

    assert len(errors) == 1
    assert isinstance(errors[0], ExecutionLaunchError)
    assert "worker lost runtime" in str(errors[0])


def test_remote_runner_fails_when_event_stream_closes_before_required_node_result() -> None:
    controller = RemoteController()
    surfaced: list[dict[str, Any]] = []
    runner = RemoteHandlerRunner(
        profile=_profile(),
        worker=_worker(),
        admission=_admission(),
        emit=surfaced.append,
        client_factory=controller.factory,
    )
    errors: list[BaseException] = []
    thread = threading.Thread(
        target=lambda: _capture_error(errors, lambda: runner("plan", "Plan", Context())),
        daemon=True,
    )

    thread.start()
    time.sleep(0.05)
    controller.push(_event("run_ready", 3, {"status": "ready"}))
    controller.wait_for_post()
    controller.push(None)
    thread.join(timeout=2)

    assert not thread.is_alive()
    assert len(errors) == 1
    assert isinstance(errors[0], ExecutionLaunchError)
    assert "closed before required node result" in str(errors[0])
    assert [event["event"] for event in surfaced] == ["run_ready", "run_failed"]
    assert surfaced[-1]["payload"]["code"] == "worker_event_stream_closed"
    assert "run-1:plan:1" in surfaced[-1]["payload"]["message"]


def test_remote_runner_fails_next_node_when_event_stream_closes_between_nodes() -> None:
    controller = RemoteController()
    surfaced: list[dict[str, Any]] = []
    runner = RemoteHandlerRunner(
        profile=_profile(),
        worker=_worker(),
        admission=_admission(),
        emit=surfaced.append,
        client_factory=controller.factory,
    )
    results: list[Outcome] = []
    first_thread = threading.Thread(target=lambda: results.append(runner("plan", "Plan", Context())), daemon=True)

    first_thread.start()
    time.sleep(0.05)
    controller.push(_event("run_ready", 3, {"status": "ready"}))
    controller.wait_for_post()
    controller.push(_event("node_result", 4, {"outcome": {"status": "success"}}))
    first_thread.join(timeout=2)
    controller.push(None)
    controller.wait_for_stream_close()

    with pytest.raises(RemoteActiveRunFailed, match="event stream closed"):
        runner("build", "Build", Context())

    assert results == [Outcome(status=OutcomeStatus.SUCCESS)]
    assert [call for call in controller.calls if call[0] == "POST" and call[1].endswith("/nodes")] == [
        controller.calls[1]
    ]
    assert [event["event"] for event in surfaced] == ["run_ready", "node_result", "run_failed"]
    assert surfaced[-1]["payload"]["code"] == "worker_event_stream_closed"


def test_remote_runner_ignores_duplicate_worker_events_before_emitting_or_applying() -> None:
    controller = RemoteController()
    surfaced: list[dict[str, Any]] = []
    runner = RemoteHandlerRunner(
        profile=_profile(),
        worker=_worker(),
        admission=_admission(),
        emit=surfaced.append,
        client_factory=controller.factory,
    )
    results: list[Outcome] = []
    thread = threading.Thread(target=lambda: results.append(runner("plan", "Plan", Context())), daemon=True)

    thread.start()
    time.sleep(0.05)
    ready = _event("run_ready", 3, {"status": "ready"})
    controller.push(ready)
    controller.push(ready)
    controller.wait_for_post()
    result = _event("node_result", 4, {"outcome": {"status": "success"}})
    controller.push(result)
    controller.push(result)
    controller.push(None)
    thread.join(timeout=2)

    assert results == [Outcome(status=OutcomeStatus.SUCCESS)]
    assert [event["event"] for event in surfaced] == ["run_ready", "node_result"]


def test_remote_runner_fails_active_run_when_sequence_gap_cannot_be_proven_by_snapshot() -> None:
    controller = RemoteController()
    surfaced: list[dict[str, Any]] = []
    runner = RemoteHandlerRunner(
        profile=_profile(),
        worker=_worker(),
        admission=_admission(),
        emit=surfaced.append,
        client_factory=controller.factory,
    )
    errors: list[BaseException] = []
    thread = threading.Thread(
        target=lambda: _capture_error(errors, lambda: runner("plan", "Plan", Context())),
        daemon=True,
    )

    thread.start()
    time.sleep(0.05)
    ready = _event("run_ready", 3, {"status": "ready"})
    controller.push(ready)
    controller.wait_for_post()
    controller.snapshot = _snapshot([ready])
    controller.push(_event("node_result", 5, {"outcome": {"status": "success"}}))
    controller.push(None)
    thread.join(timeout=2)

    assert len(errors) == 1
    assert isinstance(errors[0], ExecutionLaunchError)
    assert "worker snapshot did not prove" in str(errors[0])
    assert [event["event"] for event in surfaced] == ["run_ready", "run_failed"]
    assert surfaced[-1]["payload"]["code"] == "worker_event_sequence_gap"
    assert ("GET", "/v1/runs/run-1", None) in controller.calls


def test_remote_runner_recovers_sequence_gap_when_snapshot_provides_missing_required_events() -> None:
    controller = RemoteController()
    surfaced: list[dict[str, Any]] = []
    runner = RemoteHandlerRunner(
        profile=_profile(),
        worker=_worker(),
        admission=_admission(),
        emit=surfaced.append,
        client_factory=controller.factory,
    )
    results: list[Outcome] = []
    thread = threading.Thread(target=lambda: results.append(runner("plan", "Plan", Context())), daemon=True)

    thread.start()
    time.sleep(0.05)
    ready = _event("run_ready", 3, {"status": "ready"})
    started = _event("node_started", 4, {"status": "started"})
    controller.push(ready)
    controller.wait_for_post()
    controller.snapshot = _snapshot([ready, started])
    controller.push(_event("node_result", 5, {"outcome": {"status": "success", "context_updates": {"context.done": True}}}))
    controller.push(None)
    thread.join(timeout=2)

    assert results == [Outcome(status=OutcomeStatus.SUCCESS, context_updates={"context.done": True})]
    assert [event["event"] for event in surfaced] == ["run_ready", "node_started", "node_result"]
    assert ("GET", "/v1/runs/run-1", None) in controller.calls


@pytest.mark.parametrize("failure_event_type", ["run_failed", "run_canceled"])
def test_remote_runner_stops_after_recovered_run_failure_before_later_node_result(failure_event_type: str) -> None:
    controller = RemoteController()
    surfaced: list[dict[str, Any]] = []
    runner = RemoteHandlerRunner(
        profile=_profile(),
        worker=_worker(),
        admission=_admission(),
        emit=surfaced.append,
        client_factory=controller.factory,
    )
    errors: list[BaseException] = []
    thread = threading.Thread(
        target=lambda: _capture_error(errors, lambda: runner("plan", "Plan", Context())),
        daemon=True,
    )

    thread.start()
    time.sleep(0.05)
    ready = _event("run_ready", 3, {"status": "ready"})
    recovered_failure = _event(failure_event_type, 4, {"code": "worker_stopped", "message": "worker stopped"})
    controller.push(ready)
    controller.wait_for_post()
    controller.snapshot = _snapshot([ready, recovered_failure], status="failed")
    controller.push(_event("node_result", 5, {"outcome": {"status": "success"}}))
    controller.push(None)
    thread.join(timeout=2)

    assert len(errors) == 1
    assert "worker stopped" in str(errors[0])
    assert [event["event"] for event in surfaced] == ["run_ready", failure_event_type]
    assert surfaced[-1]["payload"]["code"] == "worker_stopped"
    assert ("GET", "/v1/runs/run-1", None) in controller.calls


def test_remote_runner_stops_after_recovered_node_failed_before_later_node_result() -> None:
    controller = RemoteController()
    surfaced: list[dict[str, Any]] = []
    runner = RemoteHandlerRunner(
        profile=_profile(),
        worker=_worker(),
        admission=_admission(),
        emit=surfaced.append,
        client_factory=controller.factory,
    )
    errors: list[BaseException] = []
    thread = threading.Thread(
        target=lambda: _capture_error(errors, lambda: runner("plan", "Plan", Context())),
        daemon=True,
    )

    thread.start()
    time.sleep(0.05)
    ready = _event("run_ready", 3, {"status": "ready"})
    recovered_failure = _event(
        "node_failed",
        4,
        {"node_execution_id": "run-1:plan:1", "code": "process_crashed", "message": "worker process crashed"},
    )
    controller.push(ready)
    controller.wait_for_post()
    controller.snapshot = _snapshot([ready, recovered_failure], status="failed")
    controller.push(_event("node_result", 5, {"outcome": {"status": "success"}}))
    controller.push(None)
    thread.join(timeout=2)

    assert len(errors) == 1
    assert "worker process crashed" in str(errors[0])
    assert [event["event"] for event in surfaced] == ["run_ready", "node_failed"]
    assert surfaced[-1]["payload"]["code"] == "process_crashed"
    assert ("GET", "/v1/runs/run-1", None) in controller.calls


def test_remote_runner_answers_worker_human_gate_request_through_interviewer_callback() -> None:
    controller = RemoteController()
    surfaced: list[dict[str, Any]] = []
    interviewer = RecordingInterviewer(Answer(value="YES", selected_values=["YES"]))
    runner = RemoteHandlerRunner(
        profile=_profile(),
        worker=_worker(),
        admission=_admission(),
        emit=surfaced.append,
        interviewer=interviewer,
        client_factory=controller.factory,
    )
    results: list[Outcome] = []
    thread = threading.Thread(target=lambda: results.append(runner("plan", "Plan", Context())), daemon=True)

    thread.start()
    time.sleep(0.05)
    controller.push(_event("run_ready", 3, {"status": "ready"}))
    controller.wait_for_post()
    controller.push(
        _event(
            "human_gate_request",
            4,
            {
                "gate_id": "gate-1",
                "question": {
                    "text": "Continue?",
                    "type": "YES_NO",
                    "metadata": {"node_id": "review"},
                },
            },
        )
    )
    controller.push(_event("node_result", 5, {"outcome": {"status": "success"}}))
    controller.push(None)
    thread.join(timeout=2)

    assert results == [Outcome(status=OutcomeStatus.SUCCESS)]
    assert [event["event"] for event in surfaced] == ["run_ready", "human_gate_request", "node_result"]
    assert interviewer.questions[0].text == "Continue?"
    assert interviewer.questions[0].metadata["gate_id"] == "gate-1"
    callback_request = controller.calls[2][2]
    assert controller.calls[2][1] == "/v1/runs/run-1/human-gates/gate-1/answer"
    assert callback_request.payload == {"value": "YES", "text": "", "selected_values": ["YES"]}


def test_remote_runner_delivers_worker_child_run_and_status_callbacks() -> None:
    controller = RemoteController()
    child_requests = []
    status_requests = []

    def launch_child(request):
        child_requests.append(request)
        return ChildRunResult(
            run_id=request.child_run_id,
            status="completed",
            outcome="success",
            completed_nodes=["done"],
            route_trace=["start", "done"],
        )

    def resolve_status(run_id: str) -> ChildRunResult | None:
        status_requests.append(run_id)
        return ChildRunResult(run_id=run_id, status="running", current_node="work")

    runner = RemoteHandlerRunner(
        profile=_profile(),
        worker=_worker(),
        admission=_admission(),
        emit=lambda _event: None,
        child_run_launcher=launch_child,
        child_status_resolver=resolve_status,
        client_factory=controller.factory,
    )
    thread = threading.Thread(target=lambda: runner("plan", "Plan", Context()), daemon=True)

    thread.start()
    time.sleep(0.05)
    controller.push(_event("run_ready", 3, {"status": "ready"}))
    controller.wait_for_post()
    controller.push(
        _event(
            "child_run_request",
            4,
            {
                "request_id": "child-req",
                "child_run_id": "child-1",
                "child_graph": {"graph_id": "child", "nodes": {}, "edges": [], "graph_attrs": {}, "defaults": {}, "subgraphs": []},
                "child_flow_name": "Child",
                "child_flow_path": "/tmp/child.dot",
                "child_workdir": "/tmp/child",
                "parent_context": {"context.parent": True},
                "parent_run_id": "run-1",
                "parent_node_id": "plan",
                "root_run_id": "run-1",
            },
        )
    )
    controller.push(_event("child_status_request", 5, {"request_id": "status-req", "run_id": "child-1"}))
    controller.push(_event("node_result", 6, {"outcome": {"status": "success"}}))
    controller.push(None)
    thread.join(timeout=2)

    assert child_requests[0].child_run_id == "child-1"
    assert child_requests[0].parent_context.get("context.parent") is True
    assert status_requests == ["child-1"]
    child_callback = controller.calls[2]
    status_callback = controller.calls[3]
    assert child_callback[1] == "/v1/runs/run-1/child-runs/child-req/result"
    assert child_callback[2].payload["run_id"] == "child-1"
    assert child_callback[2].payload["status"] == "completed"
    assert child_callback[2].payload["completed_nodes"] == ["done"]
    assert status_callback[1] == "/v1/runs/run-1/child-status/status-req/result"
    assert status_callback[2].payload == {
        "run_id": "child-1",
        "status": "running",
        "outcome": None,
        "outcome_reason_code": None,
        "outcome_reason_message": None,
        "current_node": "work",
        "completed_nodes": [],
        "route_trace": [],
        "failure_reason": "",
    }


def test_remote_runner_retries_retryable_callback_delivery_before_continuing() -> None:
    controller = RemoteController()
    controller.callback_errors = [
        WorkerAPIError("temporarily_unavailable", "try again", 503, retryable=True),
        WorkerAPIError("temporarily_unavailable", "try again", 503, retryable=True),
    ]
    runner = RemoteHandlerRunner(
        profile=_profile(),
        worker=_worker(),
        admission=_admission(),
        emit=lambda _event: None,
        interviewer=RecordingInterviewer(Answer(value="YES")),
        client_factory=controller.factory,
    )
    results: list[Outcome] = []
    thread = threading.Thread(target=lambda: results.append(runner("plan", "Plan", Context())), daemon=True)

    thread.start()
    time.sleep(0.05)
    controller.push(_event("run_ready", 3, {"status": "ready"}))
    controller.wait_for_post()
    controller.push(_event("human_gate_request", 4, {"gate_id": "gate-1", "question": {"text": "Continue?", "type": "YES_NO"}}))
    controller.push(_event("node_result", 5, {"outcome": {"status": "success"}}))
    controller.push(None)
    thread.join(timeout=2)

    callback_calls = [call for call in controller.calls if call[1] == "/v1/runs/run-1/human-gates/gate-1/answer"]
    assert results == [Outcome(status=OutcomeStatus.SUCCESS)]
    assert len(callback_calls) == 3


def test_remote_runner_does_not_retry_conflicting_callback_delivery() -> None:
    controller = RemoteController()
    controller.callback_error = WorkerAPIError("conflict", "callback conflict", 409, retryable=False)
    runner = RemoteHandlerRunner(
        profile=_profile(),
        worker=_worker(),
        admission=_admission(),
        emit=lambda _event: None,
        interviewer=RecordingInterviewer(Answer(value="YES")),
        client_factory=controller.factory,
    )
    errors: list[BaseException] = []
    thread = threading.Thread(
        target=lambda: _capture_error(errors, lambda: runner("plan", "Plan", Context())),
        daemon=True,
    )

    thread.start()
    time.sleep(0.05)
    controller.push(_event("run_ready", 3, {"status": "ready"}))
    controller.wait_for_post()
    controller.push(_event("human_gate_request", 4, {"gate_id": "gate-1", "question": {"text": "Continue?", "type": "YES_NO"}}))
    controller.push(None)
    thread.join(timeout=2)

    callback_calls = [call for call in controller.calls if call[1] == "/v1/runs/run-1/human-gates/gate-1/answer"]
    assert len(errors) == 1
    assert len(callback_calls) == 1


def test_remote_runner_callback_delivery_failure_fails_active_node_and_history() -> None:
    controller = RemoteController()
    controller.callback_error = RuntimeError("worker rejected callback")
    surfaced: list[dict[str, Any]] = []
    runner = RemoteHandlerRunner(
        profile=_profile(),
        worker=_worker(),
        admission=_admission(),
        emit=surfaced.append,
        interviewer=RecordingInterviewer(Answer(value="YES")),
        client_factory=controller.factory,
    )
    errors: list[BaseException] = []
    thread = threading.Thread(
        target=lambda: _capture_error(errors, lambda: runner("plan", "Plan", Context())),
        daemon=True,
    )

    thread.start()
    time.sleep(0.05)
    controller.push(_event("run_ready", 3, {"status": "ready"}))
    controller.wait_for_post()
    controller.push(_event("human_gate_request", 4, {"gate_id": "gate-1", "question": {"text": "Continue?", "type": "YES_NO"}}))
    controller.push(None)
    thread.join(timeout=2)

    assert len(errors) == 1
    assert isinstance(errors[0], ExecutionLaunchError)
    assert "callback delivery" in str(errors[0])
    assert [event["event"] for event in surfaced] == ["run_ready", "human_gate_request", "run_failed"]
    assert surfaced[-1]["payload"]["code"] == "worker_callback_delivery_failed"
    assert surfaced[-1]["payload"]["request_id"] == "gate-1"


def test_remote_runner_cancel_sends_worker_cancel_and_stops_new_node_submission() -> None:
    controller = RemoteController()
    surfaced: list[dict[str, Any]] = []
    runner = RemoteHandlerRunner(
        profile=_profile(),
        worker=_worker(),
        admission=_admission(),
        emit=surfaced.append,
        client_factory=controller.factory,
    )
    errors: list[BaseException] = []
    thread = threading.Thread(
        target=lambda: _capture_error(errors, lambda: runner("plan", "Plan", Context())),
        daemon=True,
    )

    thread.start()
    time.sleep(0.05)
    runner.cancel()
    controller.push(_event("run_canceling", 3, {"status": "canceling"}))
    controller.push(_event("run_canceled", 4, {"status": "canceled", "message": "aborted_by_user"}))
    controller.push(None)
    thread.join(timeout=2)

    assert len(errors) == 1
    assert isinstance(errors[0], RemotePreparationFailed)
    assert ("POST", "/v1/runs/run-1/cancel", None) in controller.calls
    assert not any(call[1] == "/v1/runs/run-1/nodes" for call in controller.calls)
    assert [event["event"] for event in surfaced] == ["run_canceling", "run_canceled"]


def test_remote_runner_preserves_worker_failure_precedence_during_cancellation() -> None:
    controller = RemoteController()
    runner = RemoteHandlerRunner(
        profile=_profile(),
        worker=_worker(),
        admission=_admission(),
        emit=lambda _event: None,
        client_factory=controller.factory,
    )
    errors: list[BaseException] = []
    thread = threading.Thread(
        target=lambda: _capture_error(errors, lambda: runner("plan", "Plan", Context())),
        daemon=True,
    )

    thread.start()
    controller.push(_event("run_ready", 3, {"status": "ready"}))
    time.sleep(0.05)
    runner.cancel()
    controller.push(
        _event(
            "node_failed",
            4,
            {"node_execution_id": "run-1:plan:1", "code": "process_crashed", "message": "worker process crashed"},
        )
    )
    controller.push(None)
    thread.join(timeout=2)

    assert len(errors) == 1
    assert isinstance(errors[0], RemoteActiveRunFailed)
    assert getattr(errors[0], "code", None) == "remote_worker_node_failed"
    assert "worker process crashed" in str(errors[0])


def test_remote_runner_control_cancel_stops_node_submission_after_ready() -> None:
    controller = RemoteController()
    cancel_requested = False
    runner = RemoteHandlerRunner(
        profile=_profile(),
        worker=_worker(),
        admission=_admission(),
        emit=lambda _event: None,
        client_factory=controller.factory,
    )
    runner.set_control(lambda: "abort" if cancel_requested else None)
    errors: list[BaseException] = []
    thread = threading.Thread(
        target=lambda: _capture_error(errors, lambda: runner("plan", "Plan", Context())),
        daemon=True,
    )

    thread.start()
    time.sleep(0.05)
    cancel_requested = True
    controller.push(_event("run_ready", 3, {"status": "ready"}))
    controller.push(None)
    thread.join(timeout=2)

    assert len(errors) == 1
    assert isinstance(errors[0], RemoteActiveRunFailed)
    assert not any(call[1] == "/v1/runs/run-1/nodes" for call in controller.calls)


def test_remote_runner_close_deletes_remote_run_and_surfaces_cleanup_failure() -> None:
    controller = RemoteController()
    runner = RemoteHandlerRunner(
        profile=_profile(),
        worker=_worker(),
        admission=_admission(),
        emit=lambda _event: None,
        client_factory=controller.factory,
    )

    runner.close()

    assert ("DELETE", "/v1/runs/run-1", None) in controller.calls

    controller = RemoteController()
    controller.cleanup_error = WorkerAPIError("cleanup_failed", "cleanup exploded", 500, retryable=False)
    runner = RemoteHandlerRunner(
        profile=_profile(),
        worker=_worker(),
        admission=_admission(),
        emit=lambda _event: None,
        client_factory=controller.factory,
    )

    with pytest.raises(WorkerAPIError, match="cleanup exploded"):
        runner.close()


def _capture_error(errors: list[BaseException], action) -> None:
    try:
        action()
    except BaseException as exc:  # noqa: BLE001
        errors.append(exc)
