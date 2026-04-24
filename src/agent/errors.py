from __future__ import annotations

from unified_llm.errors import AuthenticationError, ContextLengthError, NetworkError, ProviderError

_TRANSIENT_PROVIDER_STATUS_CODES = frozenset({429, 500, 501, 502, 503})


def _provider_status_code(error: BaseException) -> int | None:
    value = getattr(error, "status_code", None)
    if isinstance(value, int):
        return value
    return None


def is_authentication_error(error: BaseException) -> bool:
    return isinstance(error, AuthenticationError)


def is_context_length_error(error: BaseException) -> bool:
    return isinstance(error, ContextLengthError)


def is_transient_sdk_error(error: BaseException) -> bool:
    if isinstance(error, NetworkError):
        return True

    if isinstance(error, ProviderError):
        if _provider_status_code(error) in _TRANSIENT_PROVIDER_STATUS_CODES:
            return True

    retryable = getattr(error, "retryable", None)
    return retryable is True


__all__ = [
    "is_authentication_error",
    "is_context_length_error",
    "is_transient_sdk_error",
]
