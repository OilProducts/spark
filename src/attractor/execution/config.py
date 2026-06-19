from __future__ import annotations

from pathlib import Path
from typing import Any, Mapping, Protocol
import tomllib

from .errors import (
    ExecutionProfileConfigError,
    ExecutionProfileFieldError,
    ExecutionProfileSelectionError,
)
from .models import ExecutionProfile, ExecutionProfileGraph
from .modes import (
    EXECUTION_MODE_LOCAL_CONTAINER,
    EXECUTION_MODE_NATIVE,
    normalize_execution_mode,
)


EXECUTION_PROFILES_FILENAME = "execution-profiles.toml"
IMPLEMENTATION_NATIVE_PROFILE_ID = "native"


class HasConfigDir(Protocol):
    config_dir: Path


def load_execution_profile_config(
    settings: HasConfigDir,
    *,
    explicit_profile_id: str | None = None,
    project_default_profile_id: str | None = None,
    spark_default_profile_id: str | None = None,
) -> ExecutionProfileGraph:
    selected_profile_id = _first_profile_id(explicit_profile_id, project_default_profile_id, spark_default_profile_id)
    config_path = Path(settings.config_dir) / EXECUTION_PROFILES_FILENAME
    if not config_path.exists():
        if selected_profile_id:
            raise ExecutionProfileSelectionError(
                f"execution profile {selected_profile_id!r} was selected, "
                f"but {EXECUTION_PROFILES_FILENAME} does not exist"
            )
        return ExecutionProfileGraph(
            profiles={
                IMPLEMENTATION_NATIVE_PROFILE_ID: ExecutionProfile(
                    id=IMPLEMENTATION_NATIVE_PROFILE_ID,
                    label="Native",
                    mode=EXECUTION_MODE_NATIVE,
                )
            },
            synthesized_native_default=True,
        )

    try:
        raw = tomllib.loads(config_path.read_text(encoding="utf-8"))
    except tomllib.TOMLDecodeError as exc:
        raise ExecutionProfileConfigError(f"invalid {EXECUTION_PROFILES_FILENAME}: {exc}") from exc
    except OSError as exc:
        raise ExecutionProfileConfigError(f"cannot read {EXECUTION_PROFILES_FILENAME}: {exc}") from exc

    graph = _normalize_graph(raw)
    selected_profile_id = _first_profile_id(
        explicit_profile_id,
        project_default_profile_id,
        spark_default_profile_id,
        graph.default_execution_profile_id,
    )
    if selected_profile_id:
        _validate_selected_profile(graph, selected_profile_id)
    return graph


def _normalize_graph(raw: Mapping[str, Any]) -> ExecutionProfileGraph:
    field_errors: list[ExecutionProfileFieldError] = []
    default_execution_profile_id = _load_default_execution_profile_id(
        _table(raw, "defaults", field_errors),
        field_errors,
    )
    profiles = _load_profiles(_table(raw, "profiles", field_errors), field_errors)

    if field_errors:
        message = "invalid execution profile config: " + "; ".join(
            f"{error.field}: {error.message}" for error in field_errors
        )
        raise ExecutionProfileConfigError(message, field_errors=tuple(field_errors))

    return ExecutionProfileGraph(
        profiles=profiles,
        default_execution_profile_id=default_execution_profile_id,
    )


def _load_default_execution_profile_id(
    raw_defaults: Mapping[str, Any],
    field_errors: list[ExecutionProfileFieldError],
) -> str | None:
    if "execution_profile_id" not in raw_defaults:
        return None
    value = _optional_string(raw_defaults["execution_profile_id"])
    if value:
        return value
    field_errors.append(
        ExecutionProfileFieldError(
            field="defaults.execution_profile_id",
            message="execution_profile_id must be a non-empty string",
        )
    )
    return None


def _load_profiles(
    raw_profiles: Mapping[str, Any],
    field_errors: list[ExecutionProfileFieldError],
) -> dict[str, ExecutionProfile]:
    profiles: dict[str, ExecutionProfile] = {}
    for profile_id, raw_profile in raw_profiles.items():
        normalized_id = _normalize_text(profile_id)
        if not normalized_id:
            field_errors.append(
                ExecutionProfileFieldError(
                    field="profiles.<id>",
                    message="profile id must be non-empty",
                )
            )
            continue
        if not isinstance(raw_profile, Mapping):
            field_errors.append(
                ExecutionProfileFieldError(
                    field=f"profiles.{normalized_id}",
                    message="profile must be a table",
                    profile_id=normalized_id,
                )
            )
            continue

        mode = _profile_mode(raw_profile, normalized_id, field_errors)
        enabled = _optional_bool(raw_profile, "enabled", f"profiles.{normalized_id}.enabled", field_errors, profile_id=normalized_id)
        label = _required_profile_text(raw_profile, "label", normalized_id, field_errors)
        capabilities = _optional_capabilities(
            raw_profile,
            f"profiles.{normalized_id}.capabilities",
            field_errors,
            profile_id=normalized_id,
        )
        if mode is None or enabled is None or label is None or capabilities is None:
            continue

        image = _optional_profile_text(raw_profile, "image", normalized_id, field_errors)

        _validate_profile_fields(
            normalized_id,
            mode=mode,
            image=image,
            field_errors=field_errors,
        )
        if any(error.profile_id == normalized_id for error in field_errors):
            continue

        try:
            profiles[normalized_id] = ExecutionProfile(
                id=normalized_id,
                label=label,
                mode=mode,
                enabled=enabled,
                image=image,
                capabilities=capabilities,
                metadata=_optional_mapping(raw_profile.get("metadata")),
            )
        except ValueError as exc:
            field_errors.append(
                ExecutionProfileFieldError(
                    field=f"profiles.{normalized_id}",
                    message=str(exc),
                    profile_id=normalized_id,
                )
            )
    return profiles


def _validate_profile_fields(
    profile_id: str,
    *,
    mode: str,
    image: str | None,
    field_errors: list[ExecutionProfileFieldError],
) -> None:
    if mode == EXECUTION_MODE_LOCAL_CONTAINER and not image:
        _profile_error(field_errors, profile_id, "image", "image is required for local_container profiles")


def _validate_selected_profile(graph: ExecutionProfileGraph, profile_id: str) -> None:
    profile = graph.profiles.get(profile_id)
    if profile is None:
        raise ExecutionProfileSelectionError(f"selected execution profile {profile_id!r} does not exist")
    if not profile.enabled:
        raise ExecutionProfileSelectionError(f"selected execution profile {profile_id!r} is disabled")


def _table(
    raw: Mapping[str, Any],
    key: str,
    field_errors: list[ExecutionProfileFieldError],
) -> Mapping[str, Any]:
    value = raw.get(key, {})
    if isinstance(value, Mapping):
        return value
    field_errors.append(ExecutionProfileFieldError(field=key, message=f"{key} must be a table"))
    return {}


def _profile_mode(
    raw_profile: Mapping[str, Any],
    profile_id: str,
    field_errors: list[ExecutionProfileFieldError],
) -> str | None:
    raw_mode = raw_profile.get("mode")
    if raw_mode is None:
        _profile_error(field_errors, profile_id, "mode", "mode is required")
        return None
    try:
        return normalize_execution_mode(str(raw_mode))
    except ValueError as exc:
        _profile_error(field_errors, profile_id, "mode", str(exc))
        return None


def _optional_bool(
    raw: Mapping[str, Any],
    key: str,
    field: str,
    field_errors: list[ExecutionProfileFieldError],
    *,
    profile_id: str,
) -> bool | None:
    if key not in raw:
        return True
    value = raw[key]
    if isinstance(value, bool):
        return value
    field_errors.append(
        ExecutionProfileFieldError(field=field, message=f"{key} must be a boolean", profile_id=profile_id)
    )
    return None


def _required_profile_text(
    raw: Mapping[str, Any],
    key: str,
    profile_id: str,
    field_errors: list[ExecutionProfileFieldError],
) -> str | None:
    if key not in raw:
        _profile_error(field_errors, profile_id, key, f"{key} is required")
        return None
    value = _optional_string(raw[key])
    if value:
        return value
    message = f"{key} must be a non-empty string" if raw.get(key) is not None else f"{key} is required"
    _profile_error(field_errors, profile_id, key, message)
    return None


def _profile_error(
    field_errors: list[ExecutionProfileFieldError],
    profile_id: str,
    field_name: str,
    message: str,
) -> None:
    field_errors.append(
        ExecutionProfileFieldError(
            field=f"profiles.{profile_id}.{field_name}",
            message=message,
            profile_id=profile_id,
        )
    )


def _optional_capabilities(
    raw: Mapping[str, Any],
    field: str,
    field_errors: list[ExecutionProfileFieldError],
    *,
    profile_id: str | None = None,
) -> tuple[str, ...] | None:
    if "capabilities" not in raw:
        return ()
    value = raw["capabilities"]
    if not isinstance(value, list):
        field_errors.append(
            ExecutionProfileFieldError(
                field=field,
                message="capabilities must be an array of non-empty strings",
                profile_id=profile_id,
            )
        )
        return None

    capabilities: list[str] = []
    for index, item in enumerate(value):
        normalized = _optional_string(item)
        if not normalized:
            field_errors.append(
                ExecutionProfileFieldError(
                    field=f"{field}[{index}]",
                    message="capability must be a non-empty string",
                    profile_id=profile_id,
                )
            )
            return None
        capabilities.append(normalized)
    return tuple(capabilities)


def _optional_mapping(value: Any) -> Mapping[str, Any]:
    return value if isinstance(value, Mapping) else {}


def _optional_profile_text(
    raw: Mapping[str, Any],
    key: str,
    profile_id: str,
    field_errors: list[ExecutionProfileFieldError],
) -> str | None:
    if key not in raw:
        return None
    value = raw[key]
    if value is None:
        return None
    if not isinstance(value, str):
        _profile_error(field_errors, profile_id, key, f"{key} must be a string")
        return None
    normalized = value.strip()
    return normalized or None


def _optional_text(value: Any) -> str | None:
    if value is None:
        return None
    normalized = str(value).strip()
    return normalized or None


def _optional_string(value: Any) -> str | None:
    if not isinstance(value, str):
        return None
    normalized = value.strip()
    return normalized or None


def _normalize_text(value: Any) -> str:
    return str(value or "").strip()


def _first_profile_id(*values: str | None) -> str | None:
    for value in values:
        normalized = _optional_text(value)
        if normalized:
            return normalized
    return None
