from __future__ import annotations

from dataclasses import dataclass
from typing import Any


class ExecutionProfileError(ValueError):
    """Base class for execution profile configuration and selection failures."""


@dataclass(frozen=True)
class ExecutionProfileFieldError:
    field: str
    message: str
    profile_id: str | None = None


class ExecutionProfileConfigError(ExecutionProfileError):
    def __init__(self, message: str, *, field_errors: tuple[ExecutionProfileFieldError, ...] = ()) -> None:
        super().__init__(message)
        self.field_errors = field_errors


class ExecutionProfileSelectionError(ExecutionProfileError):
    pass


class ExecutionLaunchError(RuntimeError):
    def __init__(
        self,
        message: str,
        *,
        code: str | None = None,
        retryable: bool | None = None,
        details: dict[str, Any] | None = None,
        status_code: int | None = None,
    ) -> None:
        super().__init__(message)
        self.message = message
        self.code = code
        self.retryable = retryable
        self.details = dict(details or {})
        self.status_code = status_code


class ExecutionProtocolError(ExecutionLaunchError):
    pass
