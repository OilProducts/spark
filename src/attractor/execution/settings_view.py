from __future__ import annotations

from pathlib import Path
from typing import Any, Protocol

from .config import EXECUTION_PROFILES_FILENAME, load_execution_profile_config
from .errors import ExecutionProfileConfigError
from .models import ExecutionProfile
from .modes import EXECUTION_MODES


class HasConfigDir(Protocol):
    config_dir: Path


def public_execution_placement_settings(settings: HasConfigDir) -> dict[str, Any]:
    config_path = Path(settings.config_dir) / EXECUTION_PROFILES_FILENAME
    validation_errors: list[dict[str, Any]] = []
    config_loaded = True
    try:
        graph = load_execution_profile_config(settings)
        profiles = [_serialize_profile(profile) for profile in sorted(graph.profiles.values(), key=lambda item: item.id or "")]
        default_execution_profile_id = graph.default_execution_profile_id
        synthesized_native_default = graph.synthesized_native_default
    except ExecutionProfileConfigError as exc:
        config_loaded = False
        profiles = []
        default_execution_profile_id = None
        synthesized_native_default = False
        validation_errors = _serialize_config_errors(exc)

    return {
        "execution_modes": list(EXECUTION_MODES),
        "config": {
            "filename": EXECUTION_PROFILES_FILENAME,
            "path": str(config_path),
            "exists": config_path.exists(),
            "loaded": config_loaded,
            "synthesized_native_default": synthesized_native_default,
        },
        "default_execution_profile_id": default_execution_profile_id,
        "profiles": profiles,
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
        "image": profile.image,
        "capabilities": capabilities,
        "metadata": dict(profile.metadata),
    }


def _serialize_config_errors(exc: ExecutionProfileConfigError) -> list[dict[str, Any]]:
    if not exc.field_errors:
        return [{"field": None, "message": str(exc), "profile_id": None}]
    return [
        {
            "field": error.field,
            "message": error.message,
            "profile_id": error.profile_id,
        }
        for error in exc.field_errors
    ]
