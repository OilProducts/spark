from __future__ import annotations

from datetime import datetime, timezone
from typing import Any, Literal

from pydantic import BaseModel, ConfigDict, Field

WORKER_PROTOCOL_VERSION = "v1"
DEFAULT_WORKER_VERSION = "0.1.0"

WorkerRunStatus = Literal["preparing", "ready", "failed", "canceling", "canceled", "closed"]
WorkerOrphanCleanupStatus = Literal["disabled", "observing", "closed", "failed"]

WORKER_EVENT_TYPES = (
    "run_started",
    "run_preparing",
    "image_pull_started",
    "image_pull_progress",
    "container_creating",
    "run_ready",
    "node_started",
    "node_event",
    "human_gate_request",
    "child_run_request",
    "child_status_request",
    "node_result",
    "node_failed",
    "run_canceling",
    "run_canceled",
    "run_failed",
    "run_closed",
    "cleanup_failed",
    "worker_log",
)


class WorkerModel(BaseModel):
    model_config = ConfigDict(extra="forbid")


class WorkerErrorBody(WorkerModel):
    code: str
    message: str
    retryable: bool = False
    details: dict[str, Any] = Field(default_factory=dict)


class WorkerErrorResponse(WorkerModel):
    error: WorkerErrorBody


class WorkerHealthResponse(WorkerModel):
    worker_id: str
    worker_version: str
    protocol_version: str = WORKER_PROTOCOL_VERSION
    status: str
    capabilities: dict[str, Any] = Field(default_factory=dict)
    diagnostics: dict[str, Any] | None = None


class WorkerInfoResponse(WorkerModel):
    worker_id: str
    worker_version: str
    protocol_version: str = WORKER_PROTOCOL_VERSION
    status: str
    capabilities: dict[str, Any] = Field(default_factory=dict)
    supported_images: list[str] = Field(default_factory=list)
    policies: dict[str, Any] = Field(default_factory=dict)
    resource_labels: dict[str, str] = Field(default_factory=dict)
    version_details: dict[str, Any] = Field(default_factory=dict)
    validation_details: dict[str, Any] = Field(default_factory=dict)
    metadata: dict[str, Any] = Field(default_factory=dict)


class WorkerRunAdmissionRequest(WorkerModel):
    run_id: str = Field(min_length=1)
    execution_profile_id: str = Field(min_length=1)
    protocol_version: str = WORKER_PROTOCOL_VERSION
    image: str | None = None
    mapped_project_path: str = Field(min_length=1)
    worker_runtime_root: str | None = None
    capabilities: dict[str, Any] = Field(default_factory=dict)
    resources: dict[str, Any] = Field(default_factory=dict)
    metadata: dict[str, Any] = Field(default_factory=dict)


class WorkerRunAdmissionResponse(WorkerModel):
    run_id: str
    worker_id: str
    status: WorkerRunStatus
    event_url: str
    last_sequence: int = Field(ge=0)
    accepted: bool = True


class WorkerNodeRequest(WorkerModel):
    node_execution_id: str | None = Field(default=None, min_length=1)
    node_id: str = Field(min_length=1)
    attempt: int = Field(default=1, ge=1)
    payload: dict[str, Any] = Field(default_factory=dict)
    context: dict[str, Any] = Field(default_factory=dict)


class WorkerNodeAcceptedResponse(WorkerModel):
    run_id: str
    node_execution_id: str
    node_id: str
    attempt: int
    status: str = "accepted"


class WorkerCallbackRequest(WorkerModel):
    payload: dict[str, Any] = Field(default_factory=dict)


class WorkerCallbackResponse(WorkerModel):
    run_id: str
    request_id: str
    status: str = "accepted"


class WorkerCancelResponse(WorkerModel):
    run_id: str
    status: WorkerRunStatus


class WorkerCleanupResponse(WorkerModel):
    run_id: str
    status: WorkerRunStatus = "closed"
    deleted: bool


class WorkerRuntimeHandle(WorkerModel):
    runtime_id: str
    container_id: str | None = None
    worker_project_path: str
    details: dict[str, Any] = Field(default_factory=dict)


class WorkerOrphanCleanupSnapshot(WorkerModel):
    enabled: bool
    ttl_seconds: float | None = None
    status: WorkerOrphanCleanupStatus
    last_control_plane_seen_at: datetime
    eligible_at: datetime | None = None
    last_attempted_at: datetime | None = None
    last_error: WorkerErrorBody | None = None


class WorkerEvent(WorkerModel):
    run_id: str
    sequence: int
    event_type: str
    timestamp: datetime
    worker_id: str
    execution_profile_id: str
    payload: dict[str, Any] = Field(default_factory=dict)
    node_id: str | None = None
    node_attempt: int | None = None

    @classmethod
    def create(
        cls,
        *,
        run_id: str,
        sequence: int,
        event_type: str,
        worker_id: str,
        execution_profile_id: str,
        payload: dict[str, Any] | None = None,
        node_id: str | None = None,
        node_attempt: int | None = None,
    ) -> WorkerEvent:
        return cls(
            run_id=run_id,
            sequence=sequence,
            event_type=event_type,
            timestamp=datetime.now(timezone.utc),
            worker_id=worker_id,
            execution_profile_id=execution_profile_id,
            payload=dict(payload or {}),
            node_id=node_id,
            node_attempt=node_attempt,
        )


class WorkerRunSnapshot(WorkerModel):
    run_id: str
    status: WorkerRunStatus
    execution_profile_id: str
    protocol_version: str
    worker_id: str
    worker_version: str
    image: str | None = None
    mapped_project_path: str
    worker_runtime_root: str | None = None
    runtime: WorkerRuntimeHandle | None = None
    runtime_id: str | None = None
    container_id: str | None = None
    active_node: WorkerNodeRequest | None = None
    last_sequence: int = Field(ge=0)
    worker_capabilities: dict[str, Any] = Field(default_factory=dict)
    capabilities: dict[str, Any] = Field(default_factory=dict)
    resources: dict[str, Any] = Field(default_factory=dict)
    metadata: dict[str, Any] = Field(default_factory=dict)
    last_error: WorkerErrorBody | None = None
    orphan_cleanup: WorkerOrphanCleanupSnapshot | None = None
    events: list[WorkerEvent] = Field(default_factory=list)
    nodes: dict[str, WorkerNodeRequest] = Field(default_factory=dict)
    callbacks: dict[str, dict[str, Any]] = Field(default_factory=dict)
