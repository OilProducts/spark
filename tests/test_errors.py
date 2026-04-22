from __future__ import annotations

import logging

import pytest

import unified_llm


def test_public_error_exports_and_hierarchy_are_available() -> None:
    public_names = {
        "AbortError",
        "AccessDeniedError",
        "AuthenticationError",
        "ConfigurationError",
        "ContentFilterError",
        "ContextLengthError",
        "ProviderError",
        "QuotaExceededError",
        "RateLimitError",
        "RequestTimeoutError",
        "SDKError",
        "ServerError",
        "StreamError",
        "UnsupportedToolChoiceError",
        "classify_provider_error_message",
        "error_from_grpc_code",
        "error_from_status_code",
    }

    for name in public_names:
        assert hasattr(unified_llm, name)

    for builtin_name in {"ConnectionError", "PermissionError", "TimeoutError"}:
        assert not hasattr(unified_llm, builtin_name)

    provider_error_names = {
        "AccessDeniedError",
        "AuthenticationError",
        "ContentFilterError",
        "ContextLengthError",
        "InvalidRequestError",
        "NotFoundError",
        "ProviderError",
        "QuotaExceededError",
        "RateLimitError",
        "RequestTimeoutError",
        "ServerError",
    }
    sdk_error_names = {
        "AbortError",
        "ConfigurationError",
        "InvalidToolCallError",
        "NetworkError",
        "NoObjectGeneratedError",
        "StreamError",
        "UnsupportedToolChoiceError",
    }

    for name in provider_error_names:
        assert issubclass(getattr(unified_llm, name), unified_llm.SDKError)
        assert issubclass(getattr(unified_llm, name), unified_llm.ProviderError)

    for name in sdk_error_names:
        assert issubclass(getattr(unified_llm, name), unified_llm.SDKError)
        assert not issubclass(getattr(unified_llm, name), unified_llm.ProviderError)


def test_sdk_error_base_exposes_retryable_and_cause_fields() -> None:
    error = unified_llm.SDKError("boom")

    assert error.message == "boom"
    assert error.retryable is None
    assert error.cause is None


def test_provider_error_preserves_metadata_and_exception_chaining() -> None:
    cause = ValueError("bad gateway")
    raw = {"error": {"message": "backend failed", "code": "backend_error"}}

    error = unified_llm.ProviderError(
        "backend failed",
        provider="openai",
        status_code=503,
        error_code="backend_error",
        retry_after=12.5,
        raw=raw,
        cause=cause,
    )

    assert error.message == "backend failed"
    assert error.provider == "openai"
    assert error.status_code == 503
    assert error.error_code == "backend_error"
    assert error.retryable is True
    assert error.retry_after == 12.5
    assert error.raw == raw
    assert error.cause is cause
    assert error.__cause__ is cause
    assert error.__suppress_context__ is True


@pytest.mark.parametrize(
    ("error_type", "expected_retryable"),
    [
        (unified_llm.ProviderError, True),
        (unified_llm.AuthenticationError, False),
        (unified_llm.AccessDeniedError, False),
        (unified_llm.NotFoundError, False),
        (unified_llm.InvalidRequestError, False),
        (unified_llm.RateLimitError, True),
        (unified_llm.ServerError, True),
        (unified_llm.ContentFilterError, False),
        (unified_llm.ContextLengthError, False),
        (unified_llm.QuotaExceededError, False),
        (unified_llm.RequestTimeoutError, False),
        (unified_llm.AbortError, False),
        (unified_llm.NetworkError, True),
        (unified_llm.StreamError, True),
        (unified_llm.InvalidToolCallError, False),
        (unified_llm.UnsupportedToolChoiceError, False),
        (unified_llm.NoObjectGeneratedError, False),
        (unified_llm.ConfigurationError, False),
    ],
)
def test_error_retryability_flags_follow_the_spec(
    error_type,
    expected_retryable: bool,
) -> None:
    kwargs = {"provider": "openai"} if issubclass(error_type, unified_llm.ProviderError) else {}
    error = error_type("boom", **kwargs)

    assert error.retryable is expected_retryable


@pytest.mark.parametrize(
    ("message", "error_code", "expected_type"),
    [
        ("the requested model does not exist", None, unified_llm.NotFoundError),
        ("invalid key provided", None, unified_llm.AuthenticationError),
        ("too many tokens in the prompt", None, unified_llm.ContextLengthError),
        ("content filter safety block", None, unified_llm.ContentFilterError),
        (None, "NOT_FOUND", unified_llm.NotFoundError),
        (None, None, None),
    ],
)
def test_message_classifier_identifies_ambiguous_error_bodies(
    message: str | None,
    error_code: str | None,
    expected_type,
) -> None:
    assert unified_llm.classify_provider_error_message(message, error_code) is expected_type


@pytest.mark.parametrize(
    ("status_code", "message", "expected_type", "expected_retryable"),
    [
        (400, "malformed request", unified_llm.InvalidRequestError, False),
        (401, "invalid key", unified_llm.AuthenticationError, False),
        (403, "insufficient permissions", unified_llm.AccessDeniedError, False),
        (404, "model missing", unified_llm.NotFoundError, False),
        (408, "request timed out", unified_llm.RequestTimeoutError, False),
        (413, "input too large", unified_llm.ContextLengthError, False),
        (422, "invalid payload", unified_llm.InvalidRequestError, False),
        (429, "rate limited", unified_llm.RateLimitError, True),
        (503, "temporary failure", unified_llm.ServerError, True),
        (599, "gateway failure", unified_llm.ServerError, True),
    ],
)
def test_http_status_code_translation_matches_the_spec(
    status_code: int,
    message: str,
    expected_type,
    expected_retryable: bool,
) -> None:
    error = unified_llm.error_from_status_code(
        status_code,
        message=message,
        provider="openai",
    )

    assert isinstance(error, expected_type)
    assert error.message == message
    assert error.status_code == status_code
    assert error.provider == "openai"
    assert error.retryable is expected_retryable
    assert error.raw is None


@pytest.mark.parametrize(
    (
        "status_code",
        "raw",
        "expected_type",
        "expected_retryable",
        "expected_message",
        "expected_code",
    ),
    [
        (
            500,
            {"error": {"message": "The model does not exist", "code": "not_found"}},
            unified_llm.NotFoundError,
            False,
            "The model does not exist",
            "not_found",
        ),
        (
            500,
            {"error": {"message": "invalid key provided"}},
            unified_llm.AuthenticationError,
            False,
            "invalid key provided",
            None,
        ),
        (
            422,
            {"error": {"message": "too many tokens in the prompt"}},
            unified_llm.ContextLengthError,
            False,
            "too many tokens in the prompt",
            None,
        ),
        (
            429,
            {"error": {"message": "content filter safety block"}},
            unified_llm.ContentFilterError,
            False,
            "content filter safety block",
            None,
        ),
        (
            None,
            {"error": {"message": "provider failure"}},
            unified_llm.ProviderError,
            True,
            "provider failure",
            None,
        ),
    ],
)
def test_http_status_code_translation_uses_raw_message_classification(
    status_code: int | None,
    raw,
    expected_type,
    expected_retryable: bool,
    expected_message: str,
    expected_code: str | None,
) -> None:
    error = unified_llm.error_from_status_code(
        status_code,
        provider="gemini",
        raw=raw,
    )

    assert isinstance(error, expected_type)
    assert error.message == expected_message
    assert error.status_code == status_code
    assert error.provider == "gemini"
    assert error.retryable is expected_retryable
    assert error.raw == raw
    assert error.error_code == expected_code


@pytest.mark.parametrize(
    ("grpc_code", "message", "expected_type", "expected_retryable"),
    [
        ("NOT_FOUND", "missing model", unified_llm.NotFoundError, False),
        ("INVALID_ARGUMENT", "bad request", unified_llm.InvalidRequestError, False),
        ("UNAUTHENTICATED", "invalid key", unified_llm.AuthenticationError, False),
        ("PERMISSION_DENIED", "denied", unified_llm.AccessDeniedError, False),
        ("RESOURCE_EXHAUSTED", "rate limited", unified_llm.RateLimitError, True),
        ("UNAVAILABLE", "temporarily unavailable", unified_llm.ServerError, True),
        ("DEADLINE_EXCEEDED", "deadline exceeded", unified_llm.RequestTimeoutError, False),
        ("INTERNAL", "internal error", unified_llm.ServerError, True),
    ],
)
def test_grpc_code_translation_matches_the_spec(
    grpc_code: str,
    message: str,
    expected_type,
    expected_retryable: bool,
) -> None:
    error = unified_llm.error_from_grpc_code(
        grpc_code,
        message=message,
        provider="gemini",
    )

    assert isinstance(error, expected_type)
    assert error.message == message
    assert error.status_code is None
    assert error.provider == "gemini"
    assert error.retryable is expected_retryable
    assert error.error_code == grpc_code
    assert error.raw is None


@pytest.mark.parametrize(
    ("grpc_code", "raw", "expected_type", "expected_retryable", "expected_message"),
    [
        (
            "INTERNAL",
            {"error": {"message": "model does not exist"}},
            unified_llm.NotFoundError,
            False,
            "model does not exist",
        ),
        (
            "INVALID_ARGUMENT",
            {"error": {"message": "content filter safety block"}},
            unified_llm.ContentFilterError,
            False,
            "content filter safety block",
        ),
        (
            "UNKNOWN",
            {"error": {"message": "unexpected provider failure"}},
            unified_llm.ProviderError,
            True,
            "unexpected provider failure",
        ),
    ],
)
def test_grpc_code_translation_uses_raw_message_classification(
    grpc_code: str,
    raw,
    expected_type,
    expected_retryable: bool,
    expected_message: str,
) -> None:
    error = unified_llm.error_from_grpc_code(
        grpc_code,
        provider="gemini",
        raw=raw,
    )

    assert isinstance(error, expected_type)
    assert error.message == expected_message
    assert error.status_code is None
    assert error.provider == "gemini"
    assert error.retryable is expected_retryable
    assert error.raw == raw
    assert error.error_code == grpc_code


def test_invalid_json_error_body_logs_and_preserves_raw_text(
    caplog: pytest.LogCaptureFixture,
    capsys: pytest.CaptureFixture[str],
) -> None:
    raw_body = '{"error": invalid'

    with caplog.at_level(logging.DEBUG, logger="unified_llm.errors"):
        error = unified_llm.error_from_status_code(
            500,
            provider="openai",
            raw=raw_body,
        )

    assert isinstance(error, unified_llm.ServerError)
    assert error.message == raw_body
    assert error.status_code == 500
    assert error.provider == "openai"
    assert error.retryable is True
    assert error.raw == raw_body
    assert any(record.name == "unified_llm.errors" for record in caplog.records)

    captured = capsys.readouterr()
    assert captured.out == ""
    assert captured.err == ""
