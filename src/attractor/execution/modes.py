from __future__ import annotations

from typing import Literal


EXECUTION_MODE_NATIVE = "native"
EXECUTION_MODE_LOCAL_CONTAINER = "local_container"
EXECUTION_MODE_REMOTE_WORKER = "remote_worker"

ExecutionMode = Literal["native", "local_container", "remote_worker"]
EXECUTION_MODES: tuple[ExecutionMode, ...] = (
    EXECUTION_MODE_NATIVE,
    EXECUTION_MODE_LOCAL_CONTAINER,
    EXECUTION_MODE_REMOTE_WORKER,
)


def normalize_execution_mode(value: str) -> ExecutionMode:
    normalized = str(value or "").strip().lower()
    if normalized not in EXECUTION_MODES:
        raise ValueError(
            "execution mode must be one of: "
            + ", ".join(EXECUTION_MODES)
        )
    return normalized  # type: ignore[return-value]
