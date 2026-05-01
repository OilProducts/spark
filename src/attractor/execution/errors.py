from __future__ import annotations

from dataclasses import dataclass


class ExecutionProfileError(ValueError):
    """Base class for execution profile configuration and selection failures."""


@dataclass(frozen=True)
class ExecutionProfileFieldError:
    field: str
    message: str
    profile_id: str | None = None
    worker_id: str | None = None


class ExecutionProfileConfigError(ExecutionProfileError):
    def __init__(self, message: str, *, field_errors: tuple[ExecutionProfileFieldError, ...] = ()) -> None:
        super().__init__(message)
        self.field_errors = field_errors


class ExecutionProfileSelectionError(ExecutionProfileError):
    pass


class ExecutionLaunchError(RuntimeError):
    pass
