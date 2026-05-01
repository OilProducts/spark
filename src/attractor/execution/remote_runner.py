from __future__ import annotations

from collections.abc import Callable
import os
from pathlib import Path
import threading
import time
from typing import TYPE_CHECKING, Any

from attractor.engine import Context, Outcome, OutcomeStatus
from attractor.engine.outcome import FailureKind
from attractor.handlers.base import ChildRunResult, ChildRunRequest
from attractor.interviewer.base import Interviewer
from attractor.llm_runtime import RUNTIME_LAUNCH_MODEL_KEY
from attractor.transforms.runtime_preamble import (
    RUNTIME_RETRY_ATTEMPT_KEY,
    RUNTIME_RETRY_MAX_ATTEMPTS_KEY,
)

from .errors import ExecutionLaunchError, RemoteActiveRunFailed, WorkerAPIError
from .metadata import RemoteLaunchAdmission
from .models import ExecutionProfile, WorkerProfile
from .worker_models import WorkerCallbackRequest, WorkerEvent, WorkerNodeRequest, WorkerRunSnapshot

if TYPE_CHECKING:
    from .remote_client import RemoteWorkerClient


class RemotePreparationFailed(ExecutionLaunchError):
    """Remote run preparation failed before the worker emitted run_ready."""

    abort_before_node_result = True


RemoteClientFactory = Callable[[WorkerProfile], Any]
_CALLBACK_DELIVERY_MAX_ATTEMPTS = 3
_CALLBACK_DELIVERY_RETRY_DELAY_SECONDS = 0.05


class RemoteHandlerRunner:
    """Control-plane runner that delegates node execution to an admitted v1 worker run."""

    def __init__(
        self,
        *,
        profile: ExecutionProfile,
        worker: WorkerProfile,
        admission: RemoteLaunchAdmission,
        emit: Callable[[dict], None],
        graph: Any | None = None,
        interviewer: Interviewer | None = None,
        child_run_launcher: Callable[[ChildRunRequest], ChildRunResult] | None = None,
        child_status_resolver: Callable[[str], ChildRunResult | None] | None = None,
        client_factory: RemoteClientFactory | None = None,
    ) -> None:
        self.profile = profile
        self.worker = worker
        self.admission = admission
        self.emit = emit
        self.graph = graph
        self.interviewer = interviewer
        self.child_run_launcher = child_run_launcher
        self.child_status_resolver = child_status_resolver
        self._client_factory = client_factory or _default_client_factory
        self._condition = threading.Condition()
        self._ready = False
        self._closed = False
        self._stream_started = False
        self._last_sequence = admission.admission.last_sequence
        self._run_failure: str | None = None
        self._run_failure_code: str | None = None
        self._stream_closed_after_ready: str | None = None
        self._terminal_event_observed = False
        self._node_events: dict[str, WorkerEvent] = {}
        self._pending_node_results: set[str] = set()
        self._stream_thread: threading.Thread | None = None
        self._stream_client: Any | None = None
        self._control: Callable[[], str | None] | None = None
        self._cancel_requested = False
        self._cancel_sent = False
        self._cleanup_done = False
        self._logs_root: Path | None = None

    def __call__(
        self,
        node_id: str,
        prompt: str,
        context: Context,
        *,
        emit_event: Callable[..., None] | None = None,
    ) -> Outcome:
        del emit_event
        self.start()
        self._wait_for_ready()
        self._raise_if_cancel_requested()
        self._raise_if_stream_closed_after_ready()
        attempt = _resolve_attempt(context)
        node_execution_id = f"{self.admission.admission.run_id}:{node_id}:{attempt}"
        request = WorkerNodeRequest(
            node_execution_id=node_execution_id,
            node_id=node_id,
            attempt=attempt,
            payload=_node_request_payload(
                profile=self.profile,
                worker=self.worker,
                admission=self.admission,
                graph=self.graph,
                node_id=node_id,
                prompt=prompt,
                context=context,
                logs_root=self._logs_root,
            ),
            context=dict(context.values),
        )
        with self._condition:
            if self._run_failure is not None:
                raise RemoteActiveRunFailed(self._run_failure, code=self._run_failure_code)
            if self._cancel_requested or self._control_cancel_requested():
                self._cancel_requested = True
                raise RemoteActiveRunFailed("aborted_by_user", code="user_cancel_requested")
            self._pending_node_results.add(node_execution_id)
        with self._client_factory(self.worker) as client:
            try:
                client.submit_node(self.admission.admission.run_id, request)
            except WorkerAPIError as exc:
                with self._condition:
                    self._pending_node_results.discard(node_execution_id)
                raise ExecutionLaunchError(f"Remote worker {self.worker.id!r} rejected node {node_id!r}: {exc.message}") from exc
        event = self._wait_for_node_event(node_execution_id)
        if event.event_type == "node_failed":
            raise RemoteActiveRunFailed(_worker_failure_message(event), code="remote_worker_node_failed")
        _apply_worker_context_payload(event, context)
        return _outcome_from_worker_event(event)

    def start(self) -> None:
        with self._condition:
            if self._stream_started:
                return
            self._stream_started = True
            self._stream_thread = threading.Thread(
                target=self._stream_events,
                name=f"spark-remote-runner-{self.admission.admission.run_id}",
                daemon=True,
            )
            self._stream_thread.start()

    def close(self) -> None:
        cleanup_error: BaseException | None = None
        client: Any | None = None
        stream_thread: threading.Thread | None = None
        with self._condition:
            self._closed = True
            client = self._stream_client
            stream_thread = self._stream_thread
            self._condition.notify_all()
        closer = getattr(client, "close", None)
        if callable(closer):
            closer()
        if stream_thread is not None and stream_thread is not threading.current_thread():
            stream_thread.join(timeout=1)
        try:
            self._cleanup_remote_run()
        except Exception as exc:  # noqa: BLE001
            cleanup_error = exc
        if cleanup_error is not None:
            raise cleanup_error

    def set_logs_root(self, logs_root: str | Path | None) -> None:
        self._logs_root = Path(logs_root) if logs_root is not None else None

    def set_control(self, control: Callable[[], str | None] | None) -> None:
        self._control = control

    def cancel(self) -> None:
        with self._condition:
            if self._cancel_sent:
                self._cancel_requested = True
                self._condition.notify_all()
                return
            self._cancel_requested = True
            self._cancel_sent = True
            self._condition.notify_all()
        with self._client_factory(self.worker) as client:
            client.cancel_run(self.admission.admission.run_id)

    def _cleanup_remote_run(self) -> None:
        with self._condition:
            if self._cleanup_done:
                return
        with self._client_factory(self.worker) as client:
            client.cleanup_run(self.admission.admission.run_id)
        with self._condition:
            self._cleanup_done = True

    def _stream_events(self) -> None:
        try:
            with self._client_factory(self.worker) as client:
                with self._condition:
                    self._stream_client = client
                for event in client.stream_events(self.admission.admission.run_id, after=self._last_sequence):
                    self._handle_event(event, client)
                    with self._condition:
                        if self._closed:
                            return
        except Exception as exc:  # noqa: BLE001
            with self._condition:
                if self._run_failure is None:
                    self._run_failure = str(exc)
                    self._run_failure_code = "remote_worker_transport_failed"
                self._condition.notify_all()
        finally:
            self._fail_if_stream_closed_without_required_result()
            with self._condition:
                self._stream_client = None
                self._condition.notify_all()

    def _fail_if_stream_closed_without_required_result(self) -> None:
        with self._condition:
            if self._closed or self._run_failure is not None:
                return
            if not self._ready:
                message = "Remote worker event stream closed before run_ready."
            elif self._pending_node_results:
                pending = ", ".join(sorted(self._pending_node_results))
                message = f"Remote worker event stream closed before required node result for {pending}."
            elif not self._terminal_event_observed:
                self._stream_closed_after_ready = "Remote worker event stream closed for an active run after run_ready."
                self._condition.notify_all()
                return
            else:
                return
            self._run_failure = message
            self._run_failure_code = "remote_worker_event_stream_closed"
            failure_event = _stream_closed_failure_event(
                message,
                run_id=self.admission.admission.run_id,
                last_sequence=self._last_sequence,
                worker_id=self.worker.id,
                execution_profile_id=self.profile.id,
            )
        self.emit(failure_event)
        with self._condition:
            self._condition.notify_all()

    def _raise_if_stream_closed_after_ready(self) -> None:
        with self._condition:
            message = self._stream_closed_after_ready
            if message is None or self._run_failure is not None:
                return
            self._run_failure = message
            self._run_failure_code = "remote_worker_event_stream_closed"
            failure_event = _stream_closed_failure_event(
                message,
                run_id=self.admission.admission.run_id,
                last_sequence=self._last_sequence,
                worker_id=self.worker.id,
                execution_profile_id=self.profile.id,
            )
            self._condition.notify_all()
        self.emit(failure_event)
        raise RemoteActiveRunFailed(message, code="remote_worker_event_stream_closed")

    def _handle_event(self, event: WorkerEvent, client: RemoteWorkerClient) -> None:
        with self._condition:
            if event.sequence <= self._last_sequence:
                return
            if self._run_failure is not None:
                return
            expected_sequence = self._last_sequence + 1
        if event.sequence != expected_sequence and not self._recover_sequence_gap(event, client):
            return
        with self._condition:
            if self._run_failure is not None:
                return
        self._apply_event(event)
        self._deliver_callback_if_required(event, client)

    def _recover_sequence_gap(self, event: WorkerEvent, client: RemoteWorkerClient) -> bool:
        run_id = self.admission.admission.run_id
        with self._condition:
            last_sequence = self._last_sequence
        try:
            snapshot = client.get_run(run_id)
        except Exception as exc:  # noqa: BLE001
            self._fail_sequence_gap(
                f"Remote worker event stream skipped from sequence {last_sequence} to {event.sequence}, "
                f"and run snapshot recovery failed: {exc}",
                event,
            )
            return False

        recovered = _recoverable_gap_events(snapshot, after=last_sequence, before=event.sequence)
        if recovered is None:
            self._fail_sequence_gap(
                f"Remote worker event stream skipped from sequence {last_sequence} to {event.sequence}; "
                "worker snapshot did not prove that all required events were preserved.",
                event,
            )
            return False
        for recovered_event in recovered:
            self._apply_event(recovered_event)
            self._deliver_callback_if_required(recovered_event, client)
            with self._condition:
                if self._run_failure is not None:
                    return False
        with self._condition:
            return event.sequence == self._last_sequence + 1 or event.sequence <= self._last_sequence

    def _fail_sequence_gap(self, message: str, event: WorkerEvent) -> None:
        with self._condition:
            last_sequence = self._last_sequence
        failure_event = _sequence_gap_failure_event(message, event, after=last_sequence)
        self.emit(failure_event)
        with self._condition:
            if self._run_failure is None:
                self._run_failure = message
                self._run_failure_code = "remote_worker_event_sequence_gap"
            self._condition.notify_all()

    def _deliver_callback_if_required(self, event: WorkerEvent, client: RemoteWorkerClient) -> None:
        if event.event_type not in {"human_gate_request", "child_run_request", "child_status_request"}:
            return
        try:
            if event.event_type == "human_gate_request":
                self._deliver_human_gate_answer(event, client)
            elif event.event_type == "child_run_request":
                self._deliver_child_run_result(event, client)
            else:
                self._deliver_child_status_result(event, client)
        except Exception as exc:  # noqa: BLE001
            self._fail_callback_delivery(event, exc)

    def _deliver_human_gate_answer(self, event: WorkerEvent, client: RemoteWorkerClient) -> None:
        if self.interviewer is None:
            raise ExecutionLaunchError("Remote worker requested a human gate but no interviewer is available.")
        from attractor.handlers.execution_container import answer_to_payload, question_from_payload

        question_payload = event.payload.get("question")
        if not isinstance(question_payload, dict):
            question_payload = {
                "text": str(event.payload.get("prompt") or event.payload.get("text") or ""),
                "type": str(event.payload.get("type") or "FREEFORM"),
                "options": event.payload.get("options") if isinstance(event.payload.get("options"), list) else [],
                "stage": str(event.payload.get("stage") or event.node_id or ""),
                "metadata": dict(event.payload.get("metadata") or {}),
            }
        else:
            question_payload = dict(question_payload)
        metadata = dict(question_payload.get("metadata") or {})
        metadata.setdefault("gate_id", _human_gate_request_id(event))
        if event.node_id:
            metadata.setdefault("node_id", event.node_id)
        question_payload["metadata"] = metadata
        answer = self.interviewer.ask(question_from_payload(question_payload))
        self._deliver_worker_callback(
            lambda: client.answer_human_gate(
                event.run_id,
                _human_gate_request_id(event),
                WorkerCallbackRequest(payload=answer_to_payload(answer)),
            )
        )

    def _deliver_child_run_result(self, event: WorkerEvent, client: RemoteWorkerClient) -> None:
        if self.child_run_launcher is None:
            raise ExecutionLaunchError("Remote worker requested a child run but no child-run launcher is available.")
        from attractor.handlers.execution_container import child_run_request_from_payload, child_run_result_to_payload

        result = self.child_run_launcher(child_run_request_from_payload(event.payload))
        self._deliver_worker_callback(
            lambda: client.child_run_result(
                event.run_id,
                _child_run_request_id(event),
                WorkerCallbackRequest(payload=child_run_result_to_payload(result)),
            )
        )

    def _deliver_child_status_result(self, event: WorkerEvent, client: RemoteWorkerClient) -> None:
        if self.child_status_resolver is None:
            raise ExecutionLaunchError("Remote worker requested child status but no child-status resolver is available.")
        from attractor.handlers.execution_container import child_run_result_to_payload

        child_run_id = str(event.payload.get("run_id") or "")
        result = self.child_status_resolver(child_run_id) if child_run_id else None
        self._deliver_worker_callback(
            lambda: client.child_status_result(
                event.run_id,
                _child_status_request_id(event),
                WorkerCallbackRequest(payload=child_run_result_to_payload(result) if result is not None else {}),
            )
        )

    def _deliver_worker_callback(self, deliver: Callable[[], Any]) -> Any:
        last_error: Exception | None = None
        for attempt in range(1, _CALLBACK_DELIVERY_MAX_ATTEMPTS + 1):
            try:
                return deliver()
            except Exception as exc:  # noqa: BLE001
                last_error = exc
                if attempt >= _CALLBACK_DELIVERY_MAX_ATTEMPTS or not _is_retryable_callback_error(exc):
                    raise
                time.sleep(_CALLBACK_DELIVERY_RETRY_DELAY_SECONDS * attempt)
        if last_error is not None:
            raise last_error
        return None

    def _fail_callback_delivery(self, event: WorkerEvent, exc: Exception) -> None:
        message = f"Remote worker callback delivery for {event.event_type} failed: {exc}"
        self.emit(_callback_delivery_failure_event(message, event))
        with self._condition:
            if self._run_failure is None:
                self._run_failure = message
                self._run_failure_code = "remote_worker_callback_delivery_failed"
            self._condition.notify_all()

    def _apply_event(self, event: WorkerEvent) -> None:
        with self._condition:
            if event.sequence <= self._last_sequence:
                return
            if event.sequence != self._last_sequence + 1:
                raise ExecutionLaunchError(
                    f"Remote worker event sequence advanced from {self._last_sequence} to {event.sequence} without recovery."
                )
        self.emit(_normal_run_event_from_worker_event(event))
        with self._condition:
            self._last_sequence = event.sequence
            if event.event_type == "run_ready":
                self._ready = True
            elif event.event_type in {"run_failed", "run_canceled", "run_closed"}:
                self._terminal_event_observed = True
                self._run_failure = _worker_failure_message(event)
                self._run_failure_code = f"remote_worker_{event.event_type}"
            elif event.event_type in {"node_result", "node_failed"}:
                node_execution_id = _node_execution_id(event)
                self._node_events[node_execution_id] = event
                self._pending_node_results.discard(node_execution_id)
                if event.event_type == "node_failed":
                    self._run_failure = _worker_failure_message(event)
                    self._run_failure_code = "remote_worker_node_failed"
            self._condition.notify_all()

    def _wait_for_ready(self) -> None:
        with self._condition:
            while not self._ready:
                if self._run_failure is not None:
                    raise RemotePreparationFailed(self._run_failure, code=self._run_failure_code)
                if self._closed:
                    raise ExecutionLaunchError("Remote runner closed before run_ready.")
                self._condition.wait(timeout=0.25)

    def _wait_for_node_event(self, node_execution_id: str) -> WorkerEvent:
        with self._condition:
            while True:
                event = self._node_events.get(node_execution_id)
                if event is not None:
                    return event
                if self._run_failure is not None:
                    raise RemoteActiveRunFailed(self._run_failure, code=self._run_failure_code)
                if self._closed:
                    raise ExecutionLaunchError("Remote runner closed before node result.")
                self._condition.wait(timeout=0.25)

    def _raise_if_cancel_requested(self) -> None:
        with self._condition:
            if self._cancel_requested or self._control_cancel_requested():
                self._cancel_requested = True
                raise RemoteActiveRunFailed("aborted_by_user", code="user_cancel_requested")

    def _control_cancel_requested(self) -> bool:
        if self._control is None:
            return False
        try:
            return self._control() == "abort"
        except Exception:  # noqa: BLE001
            return False


def _normal_run_event_from_worker_event(event: WorkerEvent) -> dict[str, Any]:
    payload = {
        "type": "remote_worker_event",
        "event": event.event_type,
        "worker_sequence": event.sequence,
        "worker_id": event.worker_id,
        "execution_profile_id": event.execution_profile_id,
        "payload": dict(event.payload),
    }
    if event.node_id is not None:
        payload["node_id"] = event.node_id
    if event.node_attempt is not None:
        payload["node_attempt"] = event.node_attempt
    return payload


def _sequence_gap_failure_event(message: str, event: WorkerEvent, *, after: int) -> dict[str, Any]:
    payload = _normal_run_event_from_worker_event(event)
    payload["event"] = "run_failed"
    payload["payload"] = {
        "code": "worker_event_sequence_gap",
        "message": message,
        "missed_after_sequence": after,
        "received_event": event.event_type,
    }
    return payload


def _callback_delivery_failure_event(message: str, event: WorkerEvent) -> dict[str, Any]:
    payload = _normal_run_event_from_worker_event(event)
    payload["event"] = "run_failed"
    payload["payload"] = {
        "code": "worker_callback_delivery_failed",
        "message": message,
        "request_event": event.event_type,
        "request_id": _callback_request_id(event),
    }
    return payload


def _stream_closed_failure_event(
    message: str,
    *,
    run_id: str,
    last_sequence: int,
    worker_id: str,
    execution_profile_id: str,
) -> dict[str, Any]:
    return {
        "type": "remote_worker_event",
        "event": "run_failed",
        "worker_sequence": last_sequence,
        "worker_id": worker_id,
        "execution_profile_id": execution_profile_id,
        "payload": {
            "code": "worker_event_stream_closed",
            "message": message,
            "last_sequence": last_sequence,
            "run_id": run_id,
        },
    }


def _recoverable_gap_events(snapshot: WorkerRunSnapshot, *, after: int, before: int) -> list[WorkerEvent] | None:
    gap_events = sorted(
        (event for event in snapshot.events if after < event.sequence < before),
        key=lambda item: item.sequence,
    )
    expected_sequences = set(range(after + 1, before))
    recovered_sequences = {event.sequence for event in gap_events}
    if recovered_sequences != expected_sequences:
        return None
    return gap_events


def _is_retryable_callback_error(exc: Exception) -> bool:
    if isinstance(exc, WorkerAPIError):
        return exc.retryable
    if isinstance(exc, ExecutionLaunchError):
        return exc.retryable is not False
    return False


def _default_client_factory(worker: WorkerProfile) -> RemoteWorkerClient:
    from . import metadata as remote_metadata

    return remote_metadata.RemoteWorkerClient(worker)


def _node_execution_id(event: WorkerEvent) -> str:
    value = event.payload.get("node_execution_id")
    if isinstance(value, str) and value.strip():
        return value
    if event.node_id:
        return f"{event.run_id}:{event.node_id}:{event.node_attempt or 1}"
    return ""


def _callback_request_id(event: WorkerEvent) -> str:
    if event.event_type == "human_gate_request":
        return _human_gate_request_id(event)
    if event.event_type == "child_run_request":
        return _child_run_request_id(event)
    if event.event_type == "child_status_request":
        return _child_status_request_id(event)
    return ""


def _human_gate_request_id(event: WorkerEvent) -> str:
    gate_id = event.payload.get("gate_id")
    question = event.payload.get("question") if isinstance(event.payload.get("question"), dict) else {}
    metadata = question.get("metadata") if isinstance(question.get("metadata"), dict) else {}
    value = gate_id or metadata.get("gate_id") or metadata.get("node_id") or _node_execution_id(event) or event.node_id
    return str(value or "")


def _child_run_request_id(event: WorkerEvent) -> str:
    value = event.payload.get("request_id") or event.payload.get("child_run_id") or _node_execution_id(event) or event.node_id
    return str(value or "")


def _child_status_request_id(event: WorkerEvent) -> str:
    value = event.payload.get("request_id") or event.payload.get("run_id") or _node_execution_id(event) or event.node_id
    return str(value or "")


def _worker_failure_message(event: WorkerEvent) -> str:
    payload = event.payload
    return str(payload.get("message") or payload.get("code") or "remote run preparation failed")


def _outcome_from_worker_event(event: WorkerEvent) -> Outcome:
    raw_outcome = event.payload.get("outcome")
    if not isinstance(raw_outcome, dict):
        raise ExecutionLaunchError("Remote worker node_result did not include an outcome payload.")
    try:
        status = OutcomeStatus(str(raw_outcome.get("status") or ""))
    except ValueError as exc:
        raise ExecutionLaunchError("Remote worker node_result included an invalid outcome status.") from exc
    suggested_next_ids = raw_outcome.get("suggested_next_ids")
    context_updates = raw_outcome.get("context_updates")
    retryable = raw_outcome.get("retryable")
    failure_kind = raw_outcome.get("failure_kind")
    return Outcome(
        status=status,
        preferred_label=str(raw_outcome.get("preferred_label") or ""),
        suggested_next_ids=[str(item) for item in suggested_next_ids] if isinstance(suggested_next_ids, list) else [],
        context_updates=dict(context_updates) if isinstance(context_updates, dict) else {},
        failure_reason=str(raw_outcome.get("failure_reason") or ""),
        notes=str(raw_outcome.get("notes") or ""),
        retryable=retryable if isinstance(retryable, bool) else None,
        failure_kind=FailureKind(str(failure_kind)) if failure_kind is not None else None,
        raw_response_text=str(raw_outcome.get("raw_response_text") or ""),
    )


def _apply_worker_context_payload(event: WorkerEvent, context: Context) -> None:
    payload_context = event.payload.get("context")
    if isinstance(payload_context, dict):
        context.apply_updates(dict(payload_context))


def _resolve_attempt(context: Context) -> int:
    retry_attempt = context.get(RUNTIME_RETRY_ATTEMPT_KEY, 0)
    try:
        return max(1, int(retry_attempt) + 1)
    except (TypeError, ValueError):
        return 1


def _node_request_payload(
    *,
    profile: ExecutionProfile,
    worker: WorkerProfile,
    admission: RemoteLaunchAdmission,
    graph: Any | None,
    node_id: str,
    prompt: str,
    context: Context,
    logs_root: Path | None,
) -> dict[str, Any]:
    from attractor.handlers.execution_container import graph_to_payload

    process_payload = {
        "graph": graph_to_payload(graph) if graph is not None else None,
        "node_id": node_id,
        "prompt": prompt,
        "context": dict(context.snapshot()),
        "context_logs": list(context.logs),
        "logs_root": _worker_logs_root(context, admission, logs_root),
        "working_dir": admission.metadata.mapped_worker_project_path,
        "backend_name": "provider-router",
        "model": str(context.get(RUNTIME_LAUNCH_MODEL_KEY, "") or "") or None,
        "config_dir": os.environ.get("SPARK_CONFIG_DIR"),
    }
    if process_payload["graph"] is None:
        process_payload.pop("graph")
    process_payload.update(
        {
            "node_metadata": _node_metadata(graph, node_id),
            "runtime_references": {
                "run_id": admission.admission.run_id,
                "execution_profile_id": profile.id,
                "execution_mode": profile.mode,
                "worker_id": worker.id,
                "worker_version": admission.worker_info.worker_version,
                "mapped_project_path": admission.metadata.mapped_worker_project_path,
                "worker_runtime_root": admission.metadata.worker_runtime_root,
            },
            "options": _bounded_options(context),
        }
    )
    return process_payload


def _worker_logs_root(context: Context, admission: RemoteLaunchAdmission, logs_root: Path | None) -> str | None:
    runtime_root = admission.metadata.worker_runtime_root
    run_id = admission.admission.run_id
    if runtime_root:
        return str(Path(runtime_root) / "runs" / run_id / "logs")
    if logs_root is not None:
        return str(logs_root)
    value = context.get("internal.run_logs_root", None)
    if value:
        return str(value)
    return None


def _node_metadata(graph: Any | None, node_id: str) -> dict[str, Any]:
    nodes = getattr(graph, "nodes", None)
    node = nodes.get(node_id) if isinstance(nodes, dict) else None
    attrs = getattr(node, "attrs", {}) if node is not None else {}
    return {
        "node_id": node_id,
        "attrs": {str(key): _jsonish_attr_value(getattr(attr, "value", attr)) for key, attr in dict(attrs).items()},
    }


def _jsonish_attr_value(value: Any) -> Any:
    if value is None or isinstance(value, (str, int, float, bool)):
        return value
    if hasattr(value, "raw"):
        return str(value.raw)
    if isinstance(value, (list, tuple)):
        return [_jsonish_attr_value(item) for item in value]
    if isinstance(value, dict):
        return {str(key): _jsonish_attr_value(item) for key, item in value.items()}
    return str(value)


def _bounded_options(context: Context) -> dict[str, Any]:
    max_attempts = context.get(RUNTIME_RETRY_MAX_ATTEMPTS_KEY, 0)
    try:
        max_attempts_value = max(0, int(max_attempts))
    except (TypeError, ValueError):
        max_attempts_value = 0
    return {"max_attempts": max_attempts_value}
