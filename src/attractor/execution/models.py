from __future__ import annotations

from dataclasses import dataclass, field
from pathlib import Path
from typing import Any, Mapping

from .modes import (
    EXECUTION_MODE_LOCAL_CONTAINER,
    EXECUTION_MODE_NATIVE,
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
class ExecutionProfile:
    mode: ExecutionMode = EXECUTION_MODE_NATIVE
    id: str | None = None
    label: str | None = None
    enabled: bool = True
    image: str | None = None
    capabilities: Mapping[str, Any] = field(default_factory=dict)
    metadata: Mapping[str, Any] = field(default_factory=dict)

    def __post_init__(self) -> None:
        object.__setattr__(self, "mode", normalize_execution_mode(self.mode))
        if self.id is not None:
            object.__setattr__(self, "id", _require_id(self.id, field_name="execution profile id"))
        if self.label is not None:
            object.__setattr__(self, "label", _require_text(self.label, field_name="execution profile label"))
        object.__setattr__(self, "image", _normalize_optional_text(self.image))
        object.__setattr__(self, "capabilities", _copy_capabilities(self.capabilities))
        object.__setattr__(self, "metadata", _copy_metadata(self.metadata))

    @property
    def is_native(self) -> bool:
        return self.mode == EXECUTION_MODE_NATIVE

    @property
    def is_container(self) -> bool:
        return self.mode == EXECUTION_MODE_LOCAL_CONTAINER


@dataclass(frozen=True)
class ExecutionProfileGraph:
    profiles: Mapping[str, ExecutionProfile] = field(default_factory=dict)
    default_execution_profile_id: str | None = None
    synthesized_native_default: bool = False

    def __post_init__(self) -> None:
        profiles = dict(self.profiles)
        object.__setattr__(self, "profiles", profiles)
        object.__setattr__(self, "default_execution_profile_id", _normalize_optional_text(self.default_execution_profile_id))


@dataclass(frozen=True)
class ExecutionLaunchMetadata:
    execution_mode: ExecutionMode
    execution_profile_id: str | None = None
    resolved_image: str | None = None
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
        return payload


def build_launch_metadata(
    profile: ExecutionProfile,
    *,
    control_project_path: str | Path | None = None,
) -> ExecutionLaunchMetadata:
    del control_project_path
    return ExecutionLaunchMetadata(
        execution_mode=profile.mode,
        execution_profile_id=profile.id,
        resolved_image=profile.image if profile.mode == EXECUTION_MODE_LOCAL_CONTAINER else None,
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
