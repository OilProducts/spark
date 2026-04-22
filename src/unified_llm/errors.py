from __future__ import annotations

import json
import logging
import re
from collections.abc import Mapping
from typing import Any

logger = logging.getLogger(__name__)


class SDKError(Exception):
    default_retryable: bool | None = None

    def __init__(
        self,
        message: str = "",
        *,
        retryable: bool | None = None,
        cause: BaseException | None = None,
        **fields: Any,
    ) -> None:
        super().__init__(message)
        self.__dict__.update(fields)
        self.message = message
        if retryable is None:
            retryable = getattr(self.__class__, "default_retryable", None)
        self.retryable = retryable
        self.cause = cause
        if cause is not None:
            self.__cause__ = cause
            self.__suppress_context__ = True


class ProviderError(SDKError):
    default_retryable = True

    def __init__(
        self,
        message: str = "",
        *,
        provider: str | None = None,
        status_code: int | None = None,
        error_code: str | None = None,
        retryable: bool | None = None,
        retry_after: float | None = None,
        raw: Any = None,
        cause: BaseException | None = None,
        **fields: Any,
    ) -> None:
        super().__init__(
            message,
            retryable=retryable,
            cause=cause,
            provider=provider,
            status_code=status_code,
            error_code=error_code,
            retry_after=retry_after,
            raw=raw,
            **fields,
        )
        self.provider = provider
        self.status_code = status_code
        self.error_code = error_code
        self.retry_after = retry_after
        self.raw = raw
        self.cause = cause
        if cause is not None:
            self.__cause__ = cause
            self.__suppress_context__ = True


class AuthenticationError(ProviderError):
    default_retryable = False


class AccessDeniedError(ProviderError):
    default_retryable = False


class NotFoundError(ProviderError):
    default_retryable = False


class InvalidRequestError(ProviderError):
    default_retryable = False


class RateLimitError(ProviderError):
    default_retryable = True


class ServerError(ProviderError):
    default_retryable = True


class ContentFilterError(ProviderError):
    default_retryable = False


class ContextLengthError(ProviderError):
    default_retryable = False


class QuotaExceededError(ProviderError):
    default_retryable = False


class RequestTimeoutError(ProviderError):
    default_retryable = False


class AbortError(SDKError):
    default_retryable = False


class NetworkError(SDKError):
    default_retryable = True


class StreamError(SDKError):
    default_retryable = True


class InvalidToolCallError(SDKError):
    default_retryable = False


class UnsupportedToolChoiceError(SDKError):
    default_retryable = False


class NoObjectGeneratedError(SDKError):
    default_retryable = False


class ConfigurationError(SDKError):
    default_retryable = False


def _coerce_status_code(status_code: int | str | None) -> int | None:
    if status_code is None:
        return None
    if isinstance(status_code, int):
        return status_code
    try:
        return int(status_code)
    except (TypeError, ValueError):
        logger.debug("Unexpected provider status code %r", status_code)
        return None


def _coerce_text(value: Any, *, field_name: str) -> str | None:
    if value is None:
        return None
    if isinstance(value, str):
        text = value.strip()
        return text or None
    if isinstance(value, (bytes, bytearray)):
        try:
            text = bytes(value).decode("utf-8")
        except UnicodeDecodeError:
            logger.debug(
                "Unable to decode %s from provider error body as UTF-8",
                field_name,
                exc_info=True,
            )
            text = bytes(value).decode("utf-8", errors="replace")
        text = text.strip()
        return text or None
    logger.debug(
        "Unexpected %s type in provider error body: %s",
        field_name,
        type(value).__name__,
    )
    return None


def _first_text_value(mapping: Mapping[str, Any], field_names: tuple[str, ...]) -> str | None:
    for field_name in field_names:
        if field_name not in mapping:
            continue
        value = _coerce_text(mapping[field_name], field_name=field_name)
        if value is not None:
            return value
    return None


def _parse_json_error_body(text: str) -> Mapping[str, Any] | None:
    try:
        parsed = json.loads(text)
    except json.JSONDecodeError:
        logger.debug("Failed to parse provider error body as JSON", exc_info=True)
        return None
    if isinstance(parsed, Mapping):
        return parsed
    logger.debug(
        "Parsed provider error body JSON is not an object: %s",
        type(parsed).__name__,
    )
    return None


def _extract_error_details_from_mapping(
    mapping: Mapping[str, Any],
) -> tuple[str | None, str | None]:
    nested_error = mapping.get("error")
    message: str | None = None
    error_code: str | None = None

    if isinstance(nested_error, Mapping):
        message = _first_text_value(nested_error, ("message", "detail", "description"))
        error_code = _first_text_value(nested_error, ("code", "type", "error_code"))
    elif isinstance(nested_error, str):
        message = _coerce_text(nested_error, field_name="error")
    elif nested_error is not None:
        logger.debug(
            "Unexpected nested error type in provider error body: %s",
            type(nested_error).__name__,
        )

    if message is None:
        message = _first_text_value(
            mapping,
            ("message", "detail", "description", "error_description"),
        )
    if error_code is None:
        error_code = _first_text_value(mapping, ("error_code", "code", "type"))

    return message, error_code


def _extract_error_details_from_raw(raw: Any) -> tuple[str | None, str | None]:
    if raw is None:
        return None, None
    if isinstance(raw, Mapping):
        return _extract_error_details_from_mapping(raw)
    if isinstance(raw, (bytes, bytearray)):
        try:
            text = bytes(raw).decode("utf-8")
        except UnicodeDecodeError:
            logger.debug(
                "Unable to decode provider error body as UTF-8",
                exc_info=True,
            )
            text = bytes(raw).decode("utf-8", errors="replace")
        return _extract_error_details_from_raw(text)
    if isinstance(raw, str):
        stripped = raw.lstrip()
        if stripped.startswith("{") or stripped.startswith("["):
            parsed = _parse_json_error_body(raw)
            if parsed is not None:
                return _extract_error_details_from_mapping(parsed)
            return raw.strip() or raw, None
        return raw.strip() or raw, None
    logger.debug(
        "Unexpected provider error body type: %s",
        type(raw).__name__,
    )
    return None, None


def _resolve_error_details(
    raw: Any,
    message: str | None = None,
    error_code: str | None = None,
) -> tuple[str | None, str | None]:
    resolved_message = _coerce_text(message, field_name="message")
    resolved_error_code = _coerce_text(error_code, field_name="error_code")
    if resolved_message is not None and resolved_error_code is not None:
        return resolved_message, resolved_error_code

    raw_message, raw_error_code = _extract_error_details_from_raw(raw)
    if resolved_message is None:
        resolved_message = raw_message
    if resolved_error_code is None:
        resolved_error_code = raw_error_code
    return resolved_message, resolved_error_code


def _normalize_message_text(message: str | None, error_code: str | None) -> str | None:
    parts = []
    for field_name, value in (("message", message), ("error_code", error_code)):
        text = _coerce_text(value, field_name=field_name)
        if text is not None:
            parts.append(text)
    if not parts:
        return None
    normalized = " ".join(parts).lower()
    return re.sub(r"[^a-z0-9]+", " ", normalized).strip() or None


def classify_provider_error_message(
    message: str | None = None,
    error_code: str | None = None,
) -> type[ProviderError] | None:
    normalized = _normalize_message_text(message, error_code)
    if normalized is None:
        return None

    for needle, error_type in _MESSAGE_CLASSIFICATION_RULES:
        if needle in normalized:
            return error_type
    return None


def _classify_http_status_code(status_code: int | str | None) -> tuple[type[ProviderError], bool]:
    normalized = _coerce_status_code(status_code)
    if normalized is None:
        return ProviderError, True
    if normalized == 400:
        return InvalidRequestError, True
    if normalized == 401:
        return AuthenticationError, False
    if normalized == 403:
        return AccessDeniedError, False
    if normalized == 404:
        return NotFoundError, False
    if normalized == 408:
        return RequestTimeoutError, False
    if normalized == 413:
        return ContextLengthError, False
    if normalized == 422:
        return InvalidRequestError, True
    if normalized == 429:
        return RateLimitError, True
    if 500 <= normalized <= 599:
        return ServerError, True
    return ProviderError, True


def _classify_grpc_code(grpc_code: str | Any) -> tuple[type[ProviderError], bool, str | None]:
    if grpc_code is None:
        return ProviderError, True, None
    if isinstance(grpc_code, str):
        normalized = grpc_code.strip()
        if not normalized:
            return ProviderError, True, None
        code_name = normalized.rsplit(".", 1)[-1].upper()
    else:
        code_name = getattr(grpc_code, "name", None)
        if isinstance(code_name, str) and code_name:
            code_name = code_name.upper()
        else:
            text = str(grpc_code).strip()
            if not text:
                return ProviderError, True, None
            code_name = text.rsplit(".", 1)[-1].upper()

    mapping = {
        "NOT_FOUND": (NotFoundError, False),
        "INVALID_ARGUMENT": (InvalidRequestError, True),
        "UNAUTHENTICATED": (AuthenticationError, False),
        "PERMISSION_DENIED": (AccessDeniedError, False),
        "RESOURCE_EXHAUSTED": (RateLimitError, True),
        "UNAVAILABLE": (ServerError, True),
        "DEADLINE_EXCEEDED": (RequestTimeoutError, False),
        "INTERNAL": (ServerError, True),
    }
    error_type, overrideable = mapping.get(code_name, (ProviderError, True))
    return error_type, overrideable, code_name


def _default_provider_error_message(provider: str | None, status_code: int | None) -> str:
    if provider and status_code is not None:
        return f"{provider} returned HTTP {status_code}"
    if provider:
        return f"{provider} provider error"
    if status_code is not None:
        return f"HTTP {status_code} provider error"
    return "provider error"


_MESSAGE_CLASSIFICATION_RULES: tuple[tuple[str, type[ProviderError]], ...] = (
    ("not found", NotFoundError),
    ("does not exist", NotFoundError),
    ("unauthorized", AuthenticationError),
    ("invalid key", AuthenticationError),
    ("context length", ContextLengthError),
    ("too many tokens", ContextLengthError),
    ("content filter", ContentFilterError),
    ("safety", ContentFilterError),
)


def error_from_status_code(
    status_code: int | str | None,
    message: str | None = None,
    *,
    provider: str | None = None,
    error_code: str | None = None,
    retry_after: float | None = None,
    raw: Any = None,
    cause: BaseException | None = None,
) -> ProviderError:
    resolved_message, resolved_error_code = _resolve_error_details(raw, message, error_code)
    error_type, overrideable = _classify_http_status_code(status_code)
    message_error_type = classify_provider_error_message(resolved_message, resolved_error_code)
    if message_error_type is not None and overrideable:
        error_type = message_error_type

    normalized_status_code = _coerce_status_code(status_code)
    message_text = (
        resolved_message
        or resolved_error_code
        or _default_provider_error_message(provider, normalized_status_code)
    )
    return error_type(
        message_text,
        provider=provider,
        status_code=normalized_status_code,
        error_code=resolved_error_code,
        retry_after=retry_after,
        raw=raw,
        cause=cause,
    )


def error_from_grpc_code(
    grpc_code: str | Any,
    message: str | None = None,
    *,
    provider: str | None = None,
    error_code: str | None = None,
    retry_after: float | None = None,
    raw: Any = None,
    cause: BaseException | None = None,
) -> ProviderError:
    resolved_message, resolved_error_code = _resolve_error_details(raw, message, error_code)
    error_type, overrideable, normalized_code = _classify_grpc_code(grpc_code)
    message_error_type = classify_provider_error_message(resolved_message, resolved_error_code)
    if message_error_type is not None and overrideable:
        error_type = message_error_type

    message_text = (
        resolved_message
        or resolved_error_code
        or normalized_code
        or _default_provider_error_message(provider, None)
    )
    error_code_value = resolved_error_code or normalized_code
    return error_type(
        message_text,
        provider=provider,
        status_code=None,
        error_code=error_code_value,
        retry_after=retry_after,
        raw=raw,
        cause=cause,
    )


__all__ = [
    "AbortError",
    "AccessDeniedError",
    "AuthenticationError",
    "ConfigurationError",
    "ContentFilterError",
    "ContextLengthError",
    "classify_provider_error_message",
    "InvalidRequestError",
    "InvalidToolCallError",
    "NetworkError",
    "NoObjectGeneratedError",
    "NotFoundError",
    "error_from_grpc_code",
    "error_from_status_code",
    "ProviderError",
    "QuotaExceededError",
    "RateLimitError",
    "RequestTimeoutError",
    "SDKError",
    "ServerError",
    "StreamError",
    "UnsupportedToolChoiceError",
]
