from __future__ import annotations

from dataclasses import dataclass, field
from pathlib import Path
from typing import Any, Mapping

from .modes import (
    EXECUTION_MODE_LOCAL_CONTAINER,
    EXECUTION_MODE_NATIVE,
    EXECUTION_MODE_REMOTE_WORKER,
    ExecutionMode,
    normalize_execution_mode,
)


def _copy_metadata(value: Mapping[str, Any] | None) -> dict[str, Any]:
    return dict(value or {})


def _copy_capabilities(value: Any) -> dict[str, Any] | tuple[str, ...]:
    if value is None:
        return ()
    if isinstance(value, Mapping):
        return dict(value)
    return tuple(str(item).strip() for item in value if str(item).strip())


@dataclass(frozen=True)
class WorkerProfile:
    id: str
    label: str
    base_url: str
    auth_token_env: str
    enabled: bool = True
    capabilities: Mapping[str, Any] = field(default_factory=dict)
    metadata: Mapping[str, Any] = field(default_factory=dict)

    def __post_init__(self) -> None:
        object.__setattr__(self, "id", _require_id(self.id, field_name="worker id"))
        object.__setattr__(self, "label", _require_text(self.label, field_name="worker label"))
        object.__setattr__(
            self,
            "base_url",
            _require_text(self.base_url, field_name="worker base_url").rstrip("/"),
        )
        object.__setattr__(
            self,
            "auth_token_env",
            _require_text(self.auth_token_env, field_name="worker auth_token_env"),
        )
        object.__setattr__(self, "capabilities", _copy_capabilities(self.capabilities))
        object.__setattr__(self, "metadata", _copy_metadata(self.metadata))


@dataclass(frozen=True)
class ExecutionProfile:
    mode: ExecutionMode = EXECUTION_MODE_NATIVE
    id: str | None = None
    label: str | None = None
    enabled: bool = True
    worker_id: str | None = None
    image: str | None = None
    control_project_root: Path | None = None
    worker_project_root: Path | None = None
    worker_runtime_root: Path | None = None
    capabilities: Mapping[str, Any] = field(default_factory=dict)
    metadata: Mapping[str, Any] = field(default_factory=dict)

    def __post_init__(self) -> None:
        object.__setattr__(self, "mode", normalize_execution_mode(self.mode))
        if self.id is not None:
            object.__setattr__(self, "id", _require_id(self.id, field_name="execution profile id"))
        if self.label is not None:
            object.__setattr__(self, "label", _require_text(self.label, field_name="execution profile label"))
        if self.worker_id is not None:
            object.__setattr__(self, "worker_id", _require_id(self.worker_id, field_name="worker id"))
        object.__setattr__(self, "image", _normalize_optional_text(self.image))
        object.__setattr__(self, "capabilities", _copy_capabilities(self.capabilities))
        object.__setattr__(self, "metadata", _copy_metadata(self.metadata))
        if self.control_project_root is not None:
            object.__setattr__(self, "control_project_root", Path(self.control_project_root))
        if self.worker_project_root is not None:
            object.__setattr__(self, "worker_project_root", Path(self.worker_project_root))
        if self.worker_runtime_root is not None:
            object.__setattr__(self, "worker_runtime_root", Path(self.worker_runtime_root))

    @property
    def is_native(self) -> bool:
        return self.mode == EXECUTION_MODE_NATIVE

    @property
    def is_container(self) -> bool:
        return self.mode == EXECUTION_MODE_LOCAL_CONTAINER

    @property
    def is_remote_worker(self) -> bool:
        return self.mode == EXECUTION_MODE_REMOTE_WORKER


@dataclass(frozen=True)
class ExecutionProfileGraph:
    workers: Mapping[str, WorkerProfile] = field(default_factory=dict)
    profiles: Mapping[str, ExecutionProfile] = field(default_factory=dict)
    default_execution_profile_id: str | None = None
    synthesized_native_default: bool = False

    def __post_init__(self) -> None:
        workers = dict(self.workers)
        profiles = dict(self.profiles)
        object.__setattr__(self, "workers", workers)
        object.__setattr__(self, "profiles", profiles)
        object.__setattr__(self, "default_execution_profile_id", _normalize_optional_text(self.default_execution_profile_id))


@dataclass(frozen=True)
class ExecutionLaunchMetadata:
    execution_mode: ExecutionMode
    execution_profile_id: str | None = None
    resolved_image: str | None = None
    control_project_root: str | None = None
    mapped_worker_project_path: str | None = None
    worker_runtime_root: str | None = None
    capabilities: Mapping[str, Any] = field(default_factory=dict)

    def as_dict(self) -> dict[str, Any]:
        capabilities: Any
        if isinstance(self.capabilities, Mapping):
            capabilities = dict(self.capabilities)
        else:
            capabilities = list(self.capabilities)
        payload: dict[str, Any] = {
            "execution_mode": self.execution_mode,
            "execution_profile_capabilities": capabilities,
        }
        if self.execution_profile_id:
            payload["execution_profile_id"] = self.execution_profile_id
        if self.resolved_image:
            payload["execution_container_image"] = self.resolved_image
        if self.control_project_root:
            payload["execution_control_project_root"] = self.control_project_root
        if self.mapped_worker_project_path:
            payload["execution_mapped_project_path"] = self.mapped_worker_project_path
        if self.worker_runtime_root:
            payload["execution_worker_runtime_root"] = self.worker_runtime_root
        return payload


def build_launch_metadata(
    profile: ExecutionProfile,
    *,
    control_project_path: str | Path | None = None,
    mapped_worker_project_path: str | Path | None = None,
    worker_runtime_root: str | Path | None = None,
) -> ExecutionLaunchMetadata:
    if mapped_worker_project_path is None and profile.is_remote_worker and control_project_path is not None:
        from .paths import map_remote_project_path

        mapped_worker_project_path = map_remote_project_path(profile, control_project_path)
    return ExecutionLaunchMetadata(
        execution_mode=profile.mode,
        execution_profile_id=profile.id,
        resolved_image=(
            profile.image
            if profile.mode in {EXECUTION_MODE_LOCAL_CONTAINER, EXECUTION_MODE_REMOTE_WORKER}
            else None
        ),
        control_project_root=_stringify_path(profile.control_project_root),
        mapped_worker_project_path=_stringify_path(mapped_worker_project_path),
        worker_runtime_root=_stringify_path(worker_runtime_root or profile.worker_runtime_root),
        capabilities=_copy_capabilities(profile.capabilities),
    )


def _require_id(value: str, *, field_name: str) -> str:
    return _require_text(value, field_name=field_name)


def _require_text(value: str, *, field_name: str) -> str:
    normalized = str(value or "").strip()
    if not normalized:
        raise ValueError(f"{field_name} must be non-empty.")
    return normalized


def _normalize_optional_text(value: str | None) -> str | None:
    if value is None:
        return None
    normalized = str(value).strip()
    return normalized or None


def _stringify_path(value: str | Path | None) -> str | None:
    if value is None:
        return None
    normalized = str(value).strip()
    return normalized or None
