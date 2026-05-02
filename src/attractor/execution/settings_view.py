from __future__ import annotations

from collections.abc import Callable
from pathlib import Path
from typing import Any, Protocol

from .config import EXECUTION_PROFILES_FILENAME, load_execution_profile_config
from .errors import ExecutionLaunchError, ExecutionProfileConfigError, ExecutionProtocolError, WorkerAPIError
from .models import ExecutionProfile, WorkerProfile
from .modes import EXECUTION_MODES
from .remote_client import RemoteWorkerClient
from .worker_models import WORKER_PROTOCOL_VERSION, WorkerHealthResponse, WorkerInfoResponse


class HasConfigDir(Protocol):
    config_dir: Path


WorkerClientFactory = Callable[[WorkerProfile], RemoteWorkerClient]


def public_execution_placement_settings(
    settings: HasConfigDir,
    *,
    client_factory: WorkerClientFactory | None = None,
) -> dict[str, Any]:
    config_path = Path(settings.config_dir) / EXECUTION_PROFILES_FILENAME
    validation_errors: list[dict[str, Any]] = []
    config_loaded = True
    try:
        graph = load_execution_profile_config(settings)
        profiles = [_serialize_profile(profile) for profile in sorted(graph.profiles.values(), key=lambda item: item.id or "")]
        workers = [
            _serialize_worker(worker, client_factory=client_factory)
            for worker in sorted(graph.workers.values(), key=lambda item: item.id)
        ]
        default_execution_profile_id = graph.default_execution_profile_id
        synthesized_native_default = graph.synthesized_native_default
    except ExecutionProfileConfigError as exc:
        config_loaded = False
        profiles = []
        workers = []
        default_execution_profile_id = None
        synthesized_native_default = False
        validation_errors = _serialize_config_errors(exc)

    return {
        "execution_modes": list(EXECUTION_MODES),
        "protocol": {
            "expected_worker_protocol_version": WORKER_PROTOCOL_VERSION,
        },
        "config": {
            "filename": EXECUTION_PROFILES_FILENAME,
            "path": str(config_path),
            "exists": config_path.exists(),
            "loaded": config_loaded,
            "synthesized_native_default": synthesized_native_default,
        },
        "default_execution_profile_id": default_execution_profile_id,
        "profiles": profiles,
        "workers": workers,
        "validation_errors": validation_errors,
    }


def _serialize_profile(profile: ExecutionProfile) -> dict[str, Any]:
    capabilities: Any
    if isinstance(profile.capabilities, dict):
        capabilities = dict(profile.capabilities)
    else:
        capabilities = list(profile.capabilities)
    return {
        "id": profile.id,
        "label": profile.label,
        "mode": profile.mode,
        "enabled": profile.enabled,
        "worker_id": profile.worker_id,
        "image": profile.image,
        "control_project_root": _stringify_path(profile.control_project_root),
        "worker_project_root": _stringify_path(profile.worker_project_root),
        "worker_runtime_root": _stringify_path(profile.worker_runtime_root),
        "capabilities": capabilities,
        "metadata": dict(profile.metadata),
    }


def _serialize_worker(worker: WorkerProfile, *, client_factory: WorkerClientFactory | None) -> dict[str, Any]:
    health_payload, health_error = _read_worker_health(worker, client_factory=client_factory)
    info_payload, info_error = _read_worker_info(worker, client_factory=client_factory)
    compatibility = _worker_compatibility(worker, health_payload, info_payload)
    return {
        "id": worker.id,
        "label": worker.label,
        "base_url": worker.base_url,
        "auth_token_env": worker.auth_token_env,
        "enabled": worker.enabled,
        "capabilities": _capabilities_payload(worker.capabilities),
        "metadata": dict(worker.metadata),
        "health": health_payload,
        "health_error": health_error,
        "worker_info": info_payload,
        "worker_info_error": info_error,
        "status": _first_text(
            _nested_text(health_payload, "status"),
            _nested_text(info_payload, "status"),
            "disabled" if not worker.enabled else "unknown",
        ),
        "versions": {
            "worker_version": _first_text(
                _nested_text(info_payload, "worker_version"),
                _nested_text(health_payload, "worker_version"),
            ),
            "protocol_version": _first_text(
                _nested_text(info_payload, "protocol_version"),
                _nested_text(health_payload, "protocol_version"),
            ),
            "expected_protocol_version": WORKER_PROTOCOL_VERSION,
        },
        "protocol_compatible": compatibility["compatible"],
        "compatibility": compatibility,
    }


def _read_worker_health(
    worker: WorkerProfile,
    *,
    client_factory: WorkerClientFactory | None,
) -> tuple[dict[str, Any] | None, dict[str, Any] | None]:
    if not worker.enabled:
        return None, {"code": "worker_disabled", "message": "Worker is disabled."}
    factory = client_factory or RemoteWorkerClient
    try:
        with factory(worker) as client:
            return _health_payload(client.health()), None
    except (ExecutionLaunchError, ExecutionProtocolError, WorkerAPIError) as exc:
        return None, _error_payload(exc)


def _read_worker_info(
    worker: WorkerProfile,
    *,
    client_factory: WorkerClientFactory | None,
) -> tuple[dict[str, Any] | None, dict[str, Any] | None]:
    if not worker.enabled:
        return None, {"code": "worker_disabled", "message": "Worker is disabled."}
    factory = client_factory or RemoteWorkerClient
    try:
        with factory(worker) as client:
            return _worker_info_payload(client.worker_info()), None
    except (ExecutionLaunchError, ExecutionProtocolError, WorkerAPIError) as exc:
        return None, _error_payload(exc)


def _health_payload(health: WorkerHealthResponse) -> dict[str, Any]:
    return health.model_dump(mode="json")


def _worker_info_payload(worker_info: WorkerInfoResponse) -> dict[str, Any]:
    return worker_info.model_dump(mode="json")


def _worker_compatibility(
    worker: WorkerProfile,
    health: dict[str, Any] | None,
    worker_info: dict[str, Any] | None,
) -> dict[str, Any]:
    signals: list[dict[str, str]] = []
    advertised_protocols = [
        value
        for value in (
            _nested_text(health, "protocol_version"),
            _nested_text(worker_info, "protocol_version"),
        )
        if value
    ]
    identities = [
        value
        for value in (
            _nested_text(health, "worker_id"),
            _nested_text(worker_info, "worker_id"),
        )
        if value
    ]
    versions = [
        value
        for value in (
            _nested_text(health, "worker_version"),
            _nested_text(worker_info, "worker_version"),
        )
        if value
    ]
    if any(protocol != WORKER_PROTOCOL_VERSION for protocol in advertised_protocols):
        signals.append({"code": "protocol_mismatch", "message": f"Expected {WORKER_PROTOCOL_VERSION}."})
    if any(identity != worker.id for identity in identities):
        signals.append({"code": "worker_id_mismatch", "message": f"Expected worker id {worker.id}."})
    if len(set(versions)) > 1:
        signals.append({"code": "version_mismatch", "message": "Health and worker-info versions differ."})
    return {
        "compatible": not signals and bool(health or worker_info),
        "signals": signals,
    }


def _serialize_config_errors(exc: ExecutionProfileConfigError) -> list[dict[str, Any]]:
    if not exc.field_errors:
        return [{"field": None, "message": str(exc), "profile_id": None, "worker_id": None}]
    return [
        {
            "field": error.field,
            "message": error.message,
            "profile_id": error.profile_id,
            "worker_id": error.worker_id,
        }
        for error in exc.field_errors
    ]


def _error_payload(exc: Exception) -> dict[str, Any]:
    if isinstance(exc, WorkerAPIError):
        return {
            "code": exc.code,
            "message": exc.message,
            "status_code": exc.status_code,
            "retryable": exc.retryable,
            "details": dict(exc.details or {}),
        }
    return {"code": exc.__class__.__name__, "message": str(exc)}


def _capabilities_payload(value: Any) -> Any:
    if isinstance(value, dict):
        return dict(value)
    return list(value)


def _stringify_path(value: Path | None) -> str | None:
    return str(value) if value is not None else None


def _nested_text(payload: dict[str, Any] | None, field: str) -> str | None:
    if payload is None:
        return None
    value = payload.get(field)
    return value if isinstance(value, str) and value else None


def _first_text(*values: str | None) -> str | None:
    for value in values:
        if value:
            return value
    return None
