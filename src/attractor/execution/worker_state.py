from __future__ import annotations

from dataclasses import dataclass, field
from datetime import datetime, timedelta, timezone
import os
import threading
from typing import Any, Iterable

from attractor.engine.outcome import FailureKind, OutcomeStatus

from .errors import WorkerAPIError
from .worker_bridge import WorkerNodeProcessCallbacks
from .worker_models import (
    WORKER_PROTOCOL_VERSION,
    WorkerErrorBody,
    WorkerEvent,
    WorkerNodeRequest,
    WorkerOrphanCleanupSnapshot,
    WorkerRunAdmissionRequest,
    WorkerRunSnapshot,
    WorkerRunStatus,
    WorkerRuntimeHandle,
)
from .worker_runtime import (
    InProcessWorkerRuntime,
    WorkerRuntime,
    WorkerRuntimeCleanupError,
    WorkerRuntimePreparationError,
)


@dataclass(frozen=True)
class WorkerOrphanCleanupPolicy:
    enabled: bool = False
    ttl_seconds: float | None = None


@dataclass
class WorkerRunRecord:
    request: WorkerRunAdmissionRequest
    status: WorkerRunStatus = "preparing"
    admission_status: WorkerRunStatus = "preparing"
    admission_last_sequence: int = 0
    events: list[WorkerEvent] = field(default_factory=list)
    event_condition: threading.Condition = field(default_factory=threading.Condition)
    nodes: dict[str, WorkerNodeRequest] = field(default_factory=dict)
    node_responses: dict[str, dict[str, Any]] = field(default_factory=dict)
    callbacks: dict[str, dict[str, Any]] = field(default_factory=dict)
    runtime: WorkerRuntimeHandle | None = None
    active_node_key: str | None = None
    last_error: WorkerErrorBody | None = None
    cleanup_done: bool = False
    admitted_at: datetime = field(default_factory=lambda: datetime.now(timezone.utc))
    last_control_plane_seen_at: datetime = field(default_factory=lambda: datetime.now(timezone.utc))
    orphan_cleanup_last_attempted_at: datetime | None = None
    orphan_cleanup_failed: bool = False


class WorkerEventStore:
    def append(
        self,
        record: WorkerRunRecord,
        *,
        event_type: str,
        worker_id: str,
        payload: dict[str, Any] | None = None,
        node_id: str | None = None,
        node_attempt: int | None = None,
    ) -> WorkerEvent:
        with record.event_condition:
            event = WorkerEvent.create(
                run_id=record.request.run_id,
                sequence=len(record.events) + 1,
                event_type=event_type,
                worker_id=worker_id,
                execution_profile_id=record.request.execution_profile_id,
                payload=payload,
                node_id=node_id,
                node_attempt=node_attempt,
            )
            record.events.append(event)
            record.event_condition.notify_all()
            return event

    def after(self, record: WorkerRunRecord, sequence: int) -> list[WorkerEvent]:
        with record.event_condition:
            return [event for event in record.events if event.sequence > sequence]

    def wait_for_event_after(self, record: WorkerRunRecord, sequence: int, timeout: float) -> None:
        with record.event_condition:
            if not any(event.sequence > sequence for event in record.events):
                record.event_condition.wait(timeout=timeout)


class WorkerState:
    def __init__(
        self,
        *,
        worker_id: str,
        worker_version: str,
        supported_images: Iterable[str] | None = None,
        policies: dict[str, Any] | None = None,
        capabilities: dict[str, Any] | None = None,
        runtime: WorkerRuntime | None = None,
    ) -> None:
        self.worker_id = worker_id
        self.worker_version = worker_version
        self.runs: dict[str, WorkerRunRecord] = {}
        self.supported_images = tuple(supported_images or ())
        self.policies = dict(policies or {})
        self.capabilities = dict(capabilities or {})
        self.runtime = runtime if runtime is not None else InProcessWorkerRuntime(policies=self.policies)
        self.orphan_cleanup_policy = _orphan_cleanup_policy(self.policies)
        self.event_store = WorkerEventStore()
        self._lock = threading.RLock()

    def admit_run(self, request: WorkerRunAdmissionRequest) -> tuple[WorkerRunRecord, WorkerRunStatus, int]:
        with self._lock:
            existing = self.runs.get(request.run_id)
            if existing is not None:
                existing.last_control_plane_seen_at = _utcnow()
                if existing.request == request:
                    return existing, existing.admission_status, existing.admission_last_sequence
                raise WorkerAPIError(
                    "conflict",
                    "Run id was already admitted with different parameters.",
                    409,
                    details={"run_id": request.run_id},
                )
            self._validate_admission(request)
            now = _utcnow()
            record = WorkerRunRecord(request=request, admitted_at=now, last_control_plane_seen_at=now)
            self.runs[request.run_id] = record
            self.add_event(record, "run_started", {"status": "preparing"})
            self.add_event(record, "run_preparing", {"status": "preparing"})
            record.admission_status = record.status
            record.admission_last_sequence = len(record.events)
            self.prepare_run(record)
            return record, record.admission_status, record.admission_last_sequence

    def prepare_run(self, record: WorkerRunRecord) -> None:
        if record.status != "preparing":
            return
        try:
            if record.request.image:
                self.add_event(record, "image_pull_started", {"image": record.request.image})
                self.add_event(
                    record,
                    "image_pull_progress",
                    {"image": record.request.image, "status": "resolved"},
                )
            self.add_event(
                record,
                "container_creating",
                {"image": record.request.image, "mapped_project_path": record.request.mapped_project_path},
            )
            record.runtime = self.runtime.prepare_run(record.request)
        except WorkerRuntimePreparationError as exc:
            record.status = "failed"
            record.last_error = WorkerErrorBody(
                code=exc.code,
                message=exc.message,
                retryable=exc.retryable,
                details=dict(exc.details or {}),
            )
            self.add_event(record, "run_failed", record.last_error.model_dump(mode="json"))
            return
        record.status = "ready"
        self.add_event(
            record,
            "run_ready",
            {
                "status": "ready",
                "runtime_id": record.runtime.runtime_id,
                "container_id": record.runtime.container_id,
            },
        )

    def _validate_admission(self, request: WorkerRunAdmissionRequest) -> None:
        if request.protocol_version != WORKER_PROTOCOL_VERSION:
            raise WorkerAPIError(
                "unsupported_protocol",
                "Worker only supports protocol version v1.",
                400,
                details={"protocol_version": request.protocol_version},
            )
        if self.supported_images and request.image not in self.supported_images:
            raise WorkerAPIError(
                "unsupported_image",
                "Requested image is not supported by this worker.",
                400,
                details={"image": request.image, "supported_images": list(self.supported_images)},
            )
        allowed_path_prefixes = _string_list_policy(self.policies.get("mapped_project_path_prefixes"))
        if allowed_path_prefixes and not any(
            _path_is_within_prefix(request.mapped_project_path, prefix)
            for prefix in allowed_path_prefixes
        ):
            raise WorkerAPIError(
                "mapped_path_denied",
                "Mapped project path is outside worker policy.",
                400,
                details={"mapped_project_path": request.mapped_project_path, "allowed_prefixes": allowed_path_prefixes},
            )
        if self.policies.get("require_existing_mapped_project_path") and not os.path.isdir(request.mapped_project_path):
            raise WorkerAPIError(
                "mapped_path_unavailable",
                "Mapped project path is not available on this worker.",
                400,
                retryable=True,
                details={"mapped_project_path": request.mapped_project_path},
            )
        required_capabilities = _capability_policy(self.policies.get("required_capabilities"))
        missing_required_capabilities = {
            key: expected
            for key, expected in required_capabilities.items()
            if request.capabilities.get(key) != expected
        }
        if missing_required_capabilities:
            raise WorkerAPIError(
                "missing_required_capabilities",
                "Run admission is missing required worker capabilities.",
                400,
                details={"capabilities": missing_required_capabilities},
            )
        unsupported_capabilities = {
            key: expected
            for key, expected in request.capabilities.items()
            if self.capabilities.get(key) != expected
        } if self.capabilities else {}
        if unsupported_capabilities:
            raise WorkerAPIError(
                "unsupported_capabilities",
                "Requested capabilities are not supported by this worker.",
                400,
                details={"capabilities": unsupported_capabilities},
            )
        unavailable_resources = _resource_policy(self.policies.get("unavailable_resources"))
        blocked_resources = {
            key: value
            for key, value in request.resources.items()
            if key in unavailable_resources and unavailable_resources[key] == value
        }
        if blocked_resources:
            raise WorkerAPIError(
                "resource_unavailable",
                "Requested resources are not available on this worker.",
                409,
                retryable=True,
                details={"resources": blocked_resources},
            )

    def require_run(self, run_id: str) -> WorkerRunRecord:
        record = self.runs.get(run_id)
        if record is None:
            raise WorkerAPIError("not_found", "Worker run was not found.", 404, details={"run_id": run_id})
        return record

    def observe_control_plane(self, run_id: str) -> WorkerRunRecord:
        with self._lock:
            record = self.require_run(run_id)
            record.last_control_plane_seen_at = _utcnow()
            return record

    def accept_node(self, run_id: str, request: WorkerNodeRequest) -> str:
        with self._lock:
            record = self.require_run(run_id)
            record.last_control_plane_seen_at = _utcnow()
            if record.status != "ready":
                raise WorkerAPIError(
                    "run_not_ready",
                    "Node execution requires run_ready.",
                    409,
                    details={"run_id": run_id, "status": record.status},
                )
            key = _node_execution_key(request)
            existing = record.nodes.get(key)
            if existing is not None:
                if existing == request:
                    return key
                raise WorkerAPIError(
                    "conflict",
                    "Node request id was already accepted with different parameters.",
                    409,
                    details={
                        "run_id": run_id,
                        "node_execution_id": key,
                        "node_id": request.node_id,
                        "attempt": request.attempt,
                    },
                )
            if record.active_node_key is not None:
                active = record.nodes.get(record.active_node_key)
                raise WorkerAPIError(
                    "active_node_exists",
                    "Worker run already has an active node execution.",
                    409,
                    retryable=True,
                    details={
                        "run_id": run_id,
                        "active_node_execution_id": record.active_node_key,
                        "active_node_id": active.node_id if active else None,
                    },
                )
            record.nodes[key] = request
            record.active_node_key = key
            self.add_event(
                record,
                "node_started",
                {"status": "started", "payload": request.payload, "context": request.context},
                node_id=request.node_id,
                node_attempt=request.attempt,
            )
            thread = threading.Thread(
                target=self._run_accepted_node,
                args=(record, key, request),
                name=f"spark-worker-node-{run_id}-{key}",
                daemon=True,
            )
            thread.start()
            return key

    def _run_accepted_node(self, record: WorkerRunRecord, key: str, request: WorkerNodeRequest) -> None:
        try:
            assert record.runtime is not None
            process_result = self.runtime.run_node(
                record.runtime,
                _process_request(record, key, request),
                _StateNodeProcessCallbacks(self, record, request),
            )
            response_payload = _node_result_payload(process_result)
            record.node_responses[key] = response_payload
            self.add_event(
                record,
                "node_result",
                response_payload,
                node_id=request.node_id,
                node_attempt=request.attempt,
            )
        except WorkerAPIError as exc:
            failure_payload = {
                "code": exc.code,
                "message": exc.message,
                "retryable": exc.retryable,
                "details": dict(exc.details or {}),
            }
            record.node_responses[key] = {"failure": failure_payload}
            self.add_event(
                record,
                "node_failed",
                failure_payload,
                node_id=request.node_id,
                node_attempt=request.attempt,
            )
        finally:
            with self._lock:
                if record.active_node_key == key:
                    record.active_node_key = None

    def cancel_run(self, run_id: str) -> WorkerRunRecord:
        with self._lock:
            record = self.require_run(run_id)
            record.last_control_plane_seen_at = _utcnow()
            if record.status in {"canceled", "closed"}:
                return record
            if record.status == "failed":
                return record
            record.status = "canceling"
            self.add_event(record, "run_canceling", {"status": "canceling"})
            self.runtime.cancel_run(record.runtime)
            record.status = "canceled"
            record.active_node_key = None
            self.add_event(record, "run_canceled", {"status": "canceled"})
            return record

    def cleanup_run(self, run_id: str) -> tuple[WorkerRunStatus, bool]:
        with self._lock:
            record = self.runs.get(run_id)
            if record is None:
                return "closed", False
            record.last_control_plane_seen_at = _utcnow()
            return self._cleanup_run(record, reason="control_plane_delete", raise_on_failure=True)

    def sweep_orphaned_runs(self) -> None:
        if not self.orphan_cleanup_policy.enabled or self.orphan_cleanup_policy.ttl_seconds is None:
            return
        with self._lock:
            for record in list(self.runs.values()):
                if not self._orphan_cleanup_due(record):
                    continue
                self._cleanup_run(record, reason="orphaned_control_plane", raise_on_failure=False)

    def _orphan_cleanup_due(self, record: WorkerRunRecord) -> bool:
        if record.cleanup_done or record.orphan_cleanup_failed:
            return False
        if record.status == "closed":
            return False
        eligible_at = self._orphan_cleanup_eligible_at(record)
        return eligible_at is not None and _utcnow() >= eligible_at

    def _orphan_cleanup_eligible_at(self, record: WorkerRunRecord) -> datetime | None:
        ttl_seconds = self.orphan_cleanup_policy.ttl_seconds
        if not self.orphan_cleanup_policy.enabled or ttl_seconds is None:
            return None
        return record.last_control_plane_seen_at + timedelta(seconds=ttl_seconds)

    def _cleanup_run(
        self,
        record: WorkerRunRecord,
        *,
        reason: str,
        raise_on_failure: bool,
    ) -> tuple[WorkerRunStatus, bool]:
        if record.cleanup_done:
            return "closed", False
        if reason == "orphaned_control_plane":
            record.orphan_cleanup_last_attempted_at = _utcnow()
            self.add_event(
                record,
                "worker_log",
                {
                    "message": "Worker orphan cleanup policy is cleaning an admitted run after control-plane silence.",
                    "reason": reason,
                    "policy": self._orphan_cleanup_policy_payload(),
                },
            )
        try:
            self.runtime.cleanup_run(record.runtime)
        except WorkerRuntimeCleanupError as exc:
            record.last_error = WorkerErrorBody(
                code=exc.code,
                message=exc.message,
                retryable=exc.retryable,
                details=dict(exc.details or {}),
            )
            if reason == "orphaned_control_plane":
                record.orphan_cleanup_failed = True
            self.add_event(
                record,
                "cleanup_failed",
                {
                    "reason": reason,
                    "error": record.last_error.model_dump(mode="json"),
                    "policy": self._orphan_cleanup_policy_payload() if reason == "orphaned_control_plane" else {},
                },
            )
            if raise_on_failure:
                raise WorkerAPIError(exc.code, exc.message, 500, retryable=exc.retryable, details=exc.details) from exc
            return record.status, False
        record.status = "closed"
        record.active_node_key = None
        record.cleanup_done = True
        self.add_event(record, "run_closed", {"status": "closed", "reason": reason})
        return "closed", True

    def add_event(
        self,
        record: WorkerRunRecord,
        event_type: str,
        payload: dict[str, Any] | None = None,
        *,
        node_id: str | None = None,
        node_attempt: int | None = None,
    ) -> WorkerEvent:
        return self.event_store.append(
            record,
            event_type=event_type,
            worker_id=self.worker_id,
            payload=payload,
            node_id=node_id,
            node_attempt=node_attempt,
        )

    def snapshot(self, record: WorkerRunRecord) -> WorkerRunSnapshot:
        self.sweep_orphaned_runs()
        active_node = record.nodes.get(record.active_node_key or "")
        events = self.event_store.after(record, 0)
        return WorkerRunSnapshot(
            run_id=record.request.run_id,
            status=record.status,
            execution_profile_id=record.request.execution_profile_id,
            protocol_version=record.request.protocol_version,
            worker_id=self.worker_id,
            worker_version=self.worker_version,
            image=record.request.image,
            mapped_project_path=record.request.mapped_project_path,
            worker_runtime_root=record.request.worker_runtime_root,
            runtime=record.runtime,
            runtime_id=record.runtime.runtime_id if record.runtime else None,
            container_id=record.runtime.container_id if record.runtime else None,
            active_node=active_node,
            last_sequence=len(events),
            worker_capabilities=dict(self.capabilities),
            capabilities=record.request.capabilities,
            resources=record.request.resources,
            metadata=record.request.metadata,
            last_error=record.last_error,
            orphan_cleanup=self._orphan_cleanup_snapshot(record),
            events=events,
            nodes=dict(record.nodes),
            callbacks=dict(record.callbacks),
        )

    def _orphan_cleanup_snapshot(self, record: WorkerRunRecord) -> WorkerOrphanCleanupSnapshot:
        enabled = self.orphan_cleanup_policy.enabled
        eligible_at = self._orphan_cleanup_eligible_at(record)
        if not enabled:
            status = "disabled"
        elif record.orphan_cleanup_failed:
            status = "failed"
        elif record.cleanup_done or record.status == "closed":
            status = "closed"
        else:
            status = "observing"
        return WorkerOrphanCleanupSnapshot(
            enabled=enabled,
            ttl_seconds=self.orphan_cleanup_policy.ttl_seconds,
            status=status,
            last_control_plane_seen_at=record.last_control_plane_seen_at,
            eligible_at=eligible_at,
            last_attempted_at=record.orphan_cleanup_last_attempted_at,
            last_error=record.last_error if status == "failed" else None,
        )

    def _orphan_cleanup_policy_payload(self) -> dict[str, Any]:
        return {
            "enabled": self.orphan_cleanup_policy.enabled,
            "ttl_seconds": self.orphan_cleanup_policy.ttl_seconds,
        }


def _string_list_policy(value: Any) -> list[str]:
    if value is None:
        return []
    if isinstance(value, str):
        values = [value]
    elif isinstance(value, Iterable):
        values = list(value)
    else:
        return []
    return [str(item).rstrip("/") for item in values if str(item).strip()]


def _path_is_within_prefix(path: str, prefix: str) -> bool:
    normalized_path = os.path.abspath(os.path.normpath(os.path.expanduser(path)))
    normalized_prefix = os.path.abspath(os.path.normpath(os.path.expanduser(prefix)))
    try:
        return os.path.commonpath([normalized_path, normalized_prefix]) == normalized_prefix
    except ValueError:
        return False


def _capability_policy(value: Any) -> dict[str, Any]:
    if isinstance(value, dict):
        return dict(value)
    if isinstance(value, Iterable) and not isinstance(value, str):
        return {str(item): True for item in value if str(item).strip()}
    if isinstance(value, str) and value.strip():
        return {value.strip(): True}
    return {}


def _resource_policy(value: Any) -> dict[str, Any]:
    return dict(value) if isinstance(value, dict) else {}


def _orphan_cleanup_policy(policies: dict[str, Any]) -> WorkerOrphanCleanupPolicy:
    configured = policies.get("orphan_cleanup")
    ttl_value: Any = policies.get("orphan_cleanup_ttl_seconds")
    enabled = False
    if isinstance(configured, dict):
        enabled = bool(configured.get("enabled", False))
        ttl_value = configured.get("ttl_seconds", ttl_value)
    elif configured is True:
        enabled = True
    elif configured is False or configured is None:
        enabled = False
    if enabled and ttl_value is None:
        ttl_value = 3600
    ttl_seconds = _non_negative_float(ttl_value)
    if enabled and ttl_seconds is None:
        enabled = False
    return WorkerOrphanCleanupPolicy(enabled=enabled, ttl_seconds=ttl_seconds)


def _non_negative_float(value: Any) -> float | None:
    if value is None:
        return None
    try:
        parsed = float(value)
    except (TypeError, ValueError):
        return None
    return parsed if parsed >= 0 else None


def _utcnow() -> datetime:
    return datetime.now(timezone.utc)


def _node_execution_key(request: WorkerNodeRequest) -> str:
    return request.node_execution_id or f"{request.node_id}:{request.attempt}"


def _process_request(record: WorkerRunRecord, node_execution_id: str, request: WorkerNodeRequest) -> dict[str, Any]:
    payload = dict(request.payload)
    if payload:
        process_request = payload
    else:
        process_request = {}
    process_request.setdefault("node_execution_id", node_execution_id)
    process_request.setdefault("run_id", record.request.run_id)
    process_request.setdefault("node_id", request.node_id)
    process_request.setdefault("attempt", request.attempt)
    process_request.setdefault("context", dict(request.context))
    process_request.setdefault("metadata", dict(record.request.metadata))
    return process_request


def _node_result_payload(process_result: dict[str, Any]) -> dict[str, Any]:
    outcome = process_result.get("outcome")
    if not isinstance(outcome, dict) or not _is_serialized_outcome(outcome):
        raise WorkerAPIError(
            "invalid_process_result",
            "Worker node process result did not include a valid serialized Outcome.",
            502,
            details={"result": process_result},
        )
    context = process_result.get("context")
    runtime_metadata = process_result.get("runtime_metadata")
    return {
        "outcome": dict(outcome),
        "context": dict(context) if isinstance(context, dict) else {},
        "runtime_metadata": dict(runtime_metadata) if isinstance(runtime_metadata, dict) else {},
    }


def _is_serialized_outcome(outcome: dict[str, Any]) -> bool:
    try:
        OutcomeStatus(str(outcome.get("status") or ""))
    except ValueError:
        return False
    if "preferred_label" in outcome and not isinstance(outcome["preferred_label"], str):
        return False
    if "suggested_next_ids" in outcome and (
        not isinstance(outcome["suggested_next_ids"], list)
        or not all(isinstance(item, str) for item in outcome["suggested_next_ids"])
    ):
        return False
    if "context_updates" in outcome and not isinstance(outcome["context_updates"], dict):
        return False
    if "notes" in outcome and not isinstance(outcome["notes"], str):
        return False
    if "failure_reason" in outcome and not isinstance(outcome["failure_reason"], str):
        return False
    if "failure_kind" in outcome:
        try:
            FailureKind(str(outcome["failure_kind"]))
        except ValueError:
            return False
    return True


class _StateNodeProcessCallbacks(WorkerNodeProcessCallbacks):
    def __init__(self, state: WorkerState, record: WorkerRunRecord, request: WorkerNodeRequest) -> None:
        self.state = state
        self.record = record
        self.request = request

    def emit_process_event(self, event_type: str, payload: dict[str, Any]) -> None:
        self.state.add_event(
            self.record,
            event_type,
            payload,
            node_id=self.request.node_id,
            node_attempt=self.request.attempt,
        )

    def resolve_process_request(self, request_type: str, payload: dict[str, Any]) -> dict[str, Any] | None:
        callback_id = _callback_id(request_type, payload, self.request)
        if callback_id is None:
            return None
        with self.record.event_condition:
            while callback_id not in self.record.callbacks:
                self.record.event_condition.wait()
            return dict(self.record.callbacks[callback_id])


def _callback_id(request_type: str, payload: dict[str, Any], request: WorkerNodeRequest) -> str | None:
    if request_type == "human_gate_request":
        gate_id = payload.get("gate_id")
        question = payload.get("question") if isinstance(payload.get("question"), dict) else {}
        metadata = question.get("metadata") if isinstance(question.get("metadata"), dict) else {}
        gate_id = gate_id or metadata.get("gate_id") or metadata.get("node_id") or request.node_execution_id or request.node_id
        return f"human_gate:{gate_id}" if gate_id else None
    if request_type == "child_run_request":
        request_id = payload.get("request_id") or payload.get("child_run_id") or request.node_execution_id or request.node_id
        return f"child_run:{request_id}" if request_id else None
    if request_type == "child_status_request":
        request_id = payload.get("request_id") or payload.get("run_id") or request.node_execution_id or request.node_id
        return f"child_status:{request_id}" if request_id else None
    return None
