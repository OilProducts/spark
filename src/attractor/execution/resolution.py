from __future__ import annotations

from dataclasses import dataclass
from typing import Protocol

from .config import (
    IMPLEMENTATION_NATIVE_PROFILE_ID,
    load_execution_profile_config,
)
from .errors import ExecutionProfileSelectionError
from .models import ExecutionProfile, WorkerProfile
from .modes import EXECUTION_MODE_NATIVE


class HasConfigDir(Protocol):
    config_dir: object


@dataclass(frozen=True)
class ExecutionProfileSelection:
    profile: ExecutionProfile
    selected_profile_id: str | None
    selection_source: str
    worker: WorkerProfile | None = None


def resolve_execution_profile_by_id(
    settings: HasConfigDir,
    *,
    explicit_profile_id: str | None = None,
    project_default_profile_id: str | None = None,
    spark_default_profile_id: str | None = None,
) -> ExecutionProfileSelection:
    graph = load_execution_profile_config(
        settings,
        explicit_profile_id=explicit_profile_id,
        project_default_profile_id=project_default_profile_id,
        spark_default_profile_id=spark_default_profile_id,
    )
    selected_profile_id, selection_source = _selected_profile_id(
        explicit_profile_id=explicit_profile_id,
        project_default_profile_id=project_default_profile_id,
        spark_default_profile_id=spark_default_profile_id,
        graph_default_profile_id=graph.default_execution_profile_id,
    )
    if selected_profile_id is None:
        profile = _implementation_native_profile()
        return ExecutionProfileSelection(
            profile=profile,
            selected_profile_id=profile.id,
            selection_source="implementation_default",
        )

    profile = graph.profiles.get(selected_profile_id)
    if profile is None:
        raise ExecutionProfileSelectionError(
            f"selected execution profile {selected_profile_id!r} does not exist"
        )
    if not profile.enabled:
        raise ExecutionProfileSelectionError(
            f"selected execution profile {selected_profile_id!r} is disabled"
        )
    return ExecutionProfileSelection(
        profile=profile,
        selected_profile_id=selected_profile_id,
        selection_source=selection_source,
        worker=graph.workers.get(profile.worker_id or "") if profile.is_remote_worker else None,
    )


def _selected_profile_id(
    *,
    explicit_profile_id: str | None,
    project_default_profile_id: str | None,
    spark_default_profile_id: str | None,
    graph_default_profile_id: str | None,
) -> tuple[str | None, str]:
    for source, value in (
        ("explicit", explicit_profile_id),
        ("project_default", project_default_profile_id),
        ("spark_default", spark_default_profile_id),
        ("spark_default", graph_default_profile_id),
    ):
        normalized = _normalize_profile_id(value)
        if normalized:
            return normalized, source
    return None, "implementation_default"


def _normalize_profile_id(value: str | None) -> str | None:
    normalized = str(value or "").strip()
    return normalized or None


def _implementation_native_profile() -> ExecutionProfile:
    return ExecutionProfile(
        id=IMPLEMENTATION_NATIVE_PROFILE_ID,
        label="Native",
        mode=EXECUTION_MODE_NATIVE,
    )
