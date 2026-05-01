from __future__ import annotations

from attractor.engine.context import Context

from .models import ExecutionProfile


EXECUTION_MODE_CONTEXT_KEY = "_attractor.runtime.execution_mode"
EXECUTION_CONTAINER_IMAGE_CONTEXT_KEY = "_attractor.runtime.execution_container_image"
EXECUTION_PROFILE_ID_CONTEXT_KEY = "_attractor.runtime.execution_profile_id"
EXECUTION_PROFILE_SELECTION_SOURCE_CONTEXT_KEY = "_attractor.runtime.execution_profile_selection_source"


def seed_execution_profile_context(
    context: Context,
    profile: ExecutionProfile,
    *,
    selection_source: str | None = None,
) -> None:
    context.set(EXECUTION_MODE_CONTEXT_KEY, profile.mode)
    context.set(EXECUTION_PROFILE_ID_CONTEXT_KEY, profile.id or "")
    context.set(EXECUTION_PROFILE_SELECTION_SOURCE_CONTEXT_KEY, selection_source or "")
    context.set(EXECUTION_CONTAINER_IMAGE_CONTEXT_KEY, profile.image or "")
