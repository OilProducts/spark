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
    worker_id: str | None = None


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

    @classmethod
    def from_worker_api(cls, message: str, exc: WorkerAPIError) -> "ExecutionLaunchError":
        return cls(
            message,
            code=exc.code,
            retryable=exc.retryable,
            details=exc.details,
            status_code=exc.status_code,
        )


class ExecutionProtocolError(ExecutionLaunchError):
    pass


class RemoteActiveRunFailed(ExecutionLaunchError):
    """Remote execution failed without producing a trustworthy node Outcome."""

    abort_before_node_result = True


@dataclass(frozen=True)
class WorkerAPIError(Exception):
    code: str
    message: str
    status_code: int
    retryable: bool = False
    details: dict[str, Any] | None = None

    def as_payload(self) -> dict[str, Any]:
        return {
            "error": {
                "code": self.code,
                "message": self.message,
                "retryable": self.retryable,
                "details": dict(self.details or {}),
            }
        }


def worker_error_payload(
    code: str,
    message: str,
    *,
    retryable: bool = False,
    details: dict[str, Any] | None = None,
) -> dict[str, Any]:
    return {
        "error": {
            "code": code,
            "message": message,
            "retryable": retryable,
            "details": dict(details or {}),
        }
    }
