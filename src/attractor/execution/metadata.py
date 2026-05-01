from __future__ import annotations

from collections.abc import Callable
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Mapping

from .errors import ExecutionLaunchError, ExecutionProtocolError, WorkerAPIError
from .models import ExecutionLaunchMetadata, ExecutionProfile, WorkerProfile, build_launch_metadata
from .remote_client import RemoteWorkerClient
from .worker_models import (
    WorkerHealthResponse,
    WorkerInfoResponse,
    WorkerRunAdmissionRequest,
    WorkerRunAdmissionResponse,
)


@dataclass(frozen=True)
class RemoteLaunchAdmission:
    metadata: ExecutionLaunchMetadata
    health: WorkerHealthResponse
    worker_info: WorkerInfoResponse
    admission: WorkerRunAdmissionResponse


RemoteClientFactory = Callable[[WorkerProfile], RemoteWorkerClient]


def admit_remote_launch(
    profile: ExecutionProfile,
    worker: WorkerProfile | None,
    *,
    run_id: str,
    control_project_path: str | Path,
    client_factory: RemoteClientFactory | None = None,
) -> RemoteLaunchAdmission:
    if not profile.is_remote_worker:
        raise ExecutionLaunchError("remote launch admission requires a remote_worker profile")
    if worker is None:
        raise ExecutionLaunchError(f"remote_worker profile {profile.id!r} is missing a configured worker")
    if not profile.id:
        raise ExecutionLaunchError("remote_worker profile is missing execution_profile_id")

    metadata = build_launch_metadata(profile, control_project_path=control_project_path)
    if not metadata.mapped_worker_project_path:
        raise ExecutionLaunchError(f"remote_worker profile {profile.id!r} did not resolve a mapped worker project path")
    if not metadata.worker_runtime_root:
        raise ExecutionLaunchError(f"remote_worker profile {profile.id!r} is missing worker_runtime_root")

    factory = client_factory or RemoteWorkerClient
    with factory(worker) as client:
        try:
            health = client.health()
        except WorkerAPIError as exc:
            raise ExecutionLaunchError.from_worker_api(
                f"Remote worker {worker.id!r} health check failed: {exc.message}",
                exc,
            ) from exc
        try:
            worker_info = client.worker_info()
        except WorkerAPIError as exc:
            raise ExecutionLaunchError.from_worker_api(
                f"Remote worker {worker.id!r} worker-info check failed: {exc.message}",
                exc,
            ) from exc
        _validate_worker_metadata(
            worker_id=worker.id,
            health=health,
            worker_info=worker_info,
            required_capabilities=profile.capabilities,
        )
        try:
            admission = client.admit_run(
                WorkerRunAdmissionRequest(
                    run_id=run_id,
                    execution_profile_id=profile.id,
                    image=metadata.resolved_image,
                    mapped_project_path=metadata.mapped_worker_project_path,
                    worker_runtime_root=metadata.worker_runtime_root,
                    capabilities=_capabilities_dict(profile.capabilities),
                    metadata={
                        "worker_id": worker_info.worker_id,
                        "worker_version": worker_info.worker_version,
                        "worker_protocol_version": worker_info.protocol_version,
                        "worker_capabilities": dict(worker_info.capabilities),
                    },
                )
            )
        except WorkerAPIError as exc:
            raise ExecutionLaunchError.from_worker_api(
                f"Remote worker {worker.id!r} rejected run admission: {exc.message}",
                exc,
            ) from exc
    return RemoteLaunchAdmission(
        metadata=metadata,
        health=health,
        worker_info=worker_info,
        admission=admission,
    )


def _validate_worker_metadata(
    *,
    worker_id: str,
    health: WorkerHealthResponse,
    worker_info: WorkerInfoResponse,
    required_capabilities: Mapping[str, Any] | tuple[str, ...],
) -> None:
    if health.worker_id != worker_info.worker_id:
        raise ExecutionProtocolError(
            f"Remote worker {worker_id!r} returned inconsistent worker identity metadata."
        )
    if health.worker_id != worker_id or worker_info.worker_id != worker_id:
        raise ExecutionProtocolError(
            f"Remote worker {worker_id!r} returned mismatched configured worker identity metadata."
        )
    if health.worker_version != worker_info.worker_version:
        raise ExecutionProtocolError(
            f"Remote worker {worker_id!r} returned inconsistent worker version metadata."
        )
    missing = _missing_capabilities(required_capabilities, worker_info.capabilities)
    if missing:
        raise ExecutionProtocolError(
            f"Remote worker {worker_id!r} is missing required launch capabilities: {', '.join(missing)}."
        )


def _missing_capabilities(
    required: Mapping[str, Any] | tuple[str, ...],
    advertised: Mapping[str, Any],
) -> list[str]:
    missing: list[str] = []
    if isinstance(required, Mapping):
        for name, expected in required.items():
            actual = advertised.get(name)
            if isinstance(expected, bool):
                if expected and not actual:
                    missing.append(str(name))
                continue
            if expected is not None and actual != expected:
                missing.append(str(name))
        return missing
    for name in required:
        if not advertised.get(name):
            missing.append(str(name))
    return missing


def _capabilities_dict(value: Mapping[str, Any] | tuple[str, ...]) -> dict[str, Any]:
    if isinstance(value, Mapping):
        return dict(value)
    return {name: True for name in value}
