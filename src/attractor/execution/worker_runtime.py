from __future__ import annotations

from dataclasses import dataclass
from typing import Any, Iterable, Protocol

from .worker_bridge import SubprocessWorkerNodeProcess, WorkerNodeProcessCallbacks, run_node_process
from .worker_models import WorkerRunAdmissionRequest, WorkerRuntimeHandle


@dataclass(frozen=True)
class WorkerRuntimePreparationError(Exception):
    code: str
    message: str
    retryable: bool = False
    details: dict[str, Any] | None = None


@dataclass(frozen=True)
class WorkerRuntimeCleanupError(Exception):
    code: str
    message: str
    retryable: bool = False
    details: dict[str, Any] | None = None


class WorkerRuntime(Protocol):
    def prepare_run(self, request: WorkerRunAdmissionRequest) -> WorkerRuntimeHandle:
        ...

    def cancel_run(self, handle: WorkerRuntimeHandle | None) -> None:
        ...

    def cleanup_run(self, handle: WorkerRuntimeHandle | None) -> None:
        ...

    def run_node(
        self,
        handle: WorkerRuntimeHandle,
        request: dict[str, Any],
        callbacks: WorkerNodeProcessCallbacks,
    ) -> dict[str, Any]:
        ...


class InProcessWorkerRuntime:
    """Deterministic runtime adapter for worker API lifecycle tests and local dev."""

    def __init__(self, *, policies: dict[str, Any] | None = None) -> None:
        self.policies = dict(policies or {})
        self.prepared: dict[str, WorkerRuntimeHandle] = {}
        self.cleaned: set[str] = set()

    def prepare_run(self, request: WorkerRunAdmissionRequest) -> WorkerRuntimeHandle:
        configured_failure = _failure_for_run(self.policies.get("preparation_failures"), request.run_id)
        if configured_failure is not None:
            raise WorkerRuntimePreparationError(**configured_failure)

        unavailable_paths = _string_set(self.policies.get("unavailable_mapped_project_paths"))
        if request.mapped_project_path in unavailable_paths:
            raise WorkerRuntimePreparationError(
                "mapped_path_unavailable",
                "Mapped project path is not available on this worker.",
                retryable=True,
                details={"mapped_project_path": request.mapped_project_path},
            )

        for policy_key, code, message in (
            ("unavailable_mounts", "mount_unavailable", "Required mount is not available on this worker."),
            ("unavailable_devices", "device_unavailable", "Required device is not available on this worker."),
            ("unavailable_resources", "resource_unavailable", "Required resource is not available on this worker."),
        ):
            unavailable = _string_set(self.policies.get(policy_key))
            requested = _string_set(request.resources.get(policy_key.removeprefix("unavailable_")))
            missing = sorted(requested & unavailable)
            if missing:
                raise WorkerRuntimePreparationError(code, message, retryable=True, details={"missing": missing})

        image_pull_failures = self.policies.get("image_pull_failures")
        if _policy_matches(image_pull_failures, request.image):
            raise WorkerRuntimePreparationError(
                "image_unavailable",
                "Requested image could not be pulled or found on this worker.",
                retryable=True,
                details={"image": request.image},
            )

        if self.policies.get("container_create_failure"):
            raise WorkerRuntimePreparationError(
                "runtime_create_failed",
                "Worker runtime could not create or reuse a run container.",
                retryable=True,
            )

        handle = WorkerRuntimeHandle(
            runtime_id=f"runtime-{request.run_id}",
            container_id=f"container-{request.run_id}",
            worker_project_path=request.mapped_project_path,
            details={"image": request.image} if request.image else {},
        )
        self.prepared[request.run_id] = handle
        return handle

    def cancel_run(self, handle: WorkerRuntimeHandle | None) -> None:
        return None

    def cleanup_run(self, handle: WorkerRuntimeHandle | None) -> None:
        if self.policies.get("cleanup_failure"):
            raise WorkerRuntimeCleanupError(
                "cleanup_failed",
                "Worker runtime cleanup failed.",
                retryable=True,
                details={"runtime_id": handle.runtime_id if handle else None},
            )
        if handle is not None:
            self.cleaned.add(handle.runtime_id)

    def run_node(
        self,
        handle: WorkerRuntimeHandle,
        request: dict[str, Any],
        callbacks: WorkerNodeProcessCallbacks,
    ) -> dict[str, Any]:
        configured_messages = self.policies.get("node_process_messages")
        if isinstance(configured_messages, dict):
            messages = configured_messages.get(request.get("node_execution_id")) or configured_messages.get(request.get("node_id"))
        else:
            messages = configured_messages
        if isinstance(messages, Iterable) and not isinstance(messages, (str, bytes, dict)):
            final: dict[str, Any] | None = None
            for message in messages:
                if not isinstance(message, dict):
                    continue
                kind = str(message.get("type") or "")
                if kind == "result":
                    final = dict(message)
                elif kind in {"event", "progress", "log"}:
                    event_payload = message.get("payload") if isinstance(message.get("payload"), dict) else {}
                    callbacks.emit_process_event(
                        "worker_log" if kind == "log" else "node_event",
                        {
                            "process_message_type": kind,
                            "process_event_type": str(message.get("event_type") or kind),
                            **dict(event_payload),
                        },
                    )
                elif kind in {"human_gate_request", "child_run_request", "child_status_request"}:
                    callbacks.emit_process_event(kind, {key: value for key, value in message.items() if key != "type"})
            if final is not None:
                return final

        return {
            "type": "result",
            "outcome": {
                "status": "success",
                "preferred_label": "",
                "suggested_next_ids": [],
                "context_updates": {},
                "notes": "",
                "failure_reason": "",
            },
            "context": dict(request.get("context") or {}),
            "runtime_metadata": {"runtime_id": handle.runtime_id, "process": "in_process"},
        }


class LocalProcessWorkerRuntime(InProcessWorkerRuntime):
    def __init__(
        self,
        *,
        command: Iterable[str] | None = None,
        policies: dict[str, Any] | None = None,
    ) -> None:
        super().__init__(policies=policies)
        self.command = list(command or ["spark-server", "worker", "run-node"])
        self._active_process: SubprocessWorkerNodeProcess | None = None

    def run_node(
        self,
        handle: WorkerRuntimeHandle,
        request: dict[str, Any],
        callbacks: WorkerNodeProcessCallbacks,
    ) -> dict[str, Any]:
        process = SubprocessWorkerNodeProcess(self.command, cwd=handle.worker_project_path)
        self._active_process = process
        try:
            result = run_node_process(
                request=request,
                stdin=process.stdin,
                stdout=process.stdout,
                stderr=process.stderr,
                wait=process.wait,
                callbacks=callbacks,
            )
            return {
                "type": "result",
                "outcome": result.outcome,
                "context": result.context,
                "runtime_metadata": result.runtime_metadata,
            }
        finally:
            if self._active_process is process:
                self._active_process = None

    def cancel_run(self, handle: WorkerRuntimeHandle | None) -> None:
        if self._active_process is not None:
            self._active_process.cancel()
        super().cancel_run(handle)


def _failure_for_run(value: Any, run_id: str) -> dict[str, Any] | None:
    if not isinstance(value, dict):
        return None
    failure = value.get(run_id)
    if failure is True:
        return {"code": "runtime_prepare_failed", "message": "Worker runtime preparation failed.", "retryable": True}
    if isinstance(failure, dict):
        return {
            "code": str(failure.get("code") or "runtime_prepare_failed"),
            "message": str(failure.get("message") or "Worker runtime preparation failed."),
            "retryable": bool(failure.get("retryable", True)),
            "details": dict(failure.get("details") or {}),
        }
    return None


def _string_set(value: Any) -> set[str]:
    if value is None:
        return set()
    if isinstance(value, str):
        return {value} if value else set()
    if isinstance(value, Iterable):
        return {str(item) for item in value if str(item).strip()}
    return set()


def _policy_matches(value: Any, candidate: str | None) -> bool:
    if value is True:
        return True
    if value is None or candidate is None:
        return False
    if isinstance(value, str):
        return value == candidate
    if isinstance(value, Iterable):
        return candidate in {str(item) for item in value}
    return False
