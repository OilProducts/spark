from __future__ import annotations

import asyncio
import logging

import pytest

import agent
import unified_llm


async def _next_event(stream) -> agent.SessionEvent:
    return await asyncio.wait_for(anext(stream), timeout=1)


def _assert_structured_model_error(
    payload: dict,
    *,
    kind: str,
    name: str,
    message: str,
    retryable: bool,
    provider: str = "fake-provider",
    model: str = "fake-model",
) -> None:
    error = payload["error"]
    assert payload["message"] == message
    assert payload["error_kind"] == kind
    assert payload["name"] == name
    assert payload["code"] == error.get("error_code", kind)
    assert payload["retryable"] is retryable
    assert payload["provider"] == provider
    assert payload["model"] == model
    assert error["kind"] == kind
    assert error["name"] == name
    assert error["message"] == message
    assert error["retryable"] is retryable
    assert error["provider"] == provider
    assert error["model"] == model


class _PromptProfile(agent.ProviderProfile):
    def build_system_prompt(self, environment, project_docs):
        return "Session system prompt"


class _FailingCompleteClient:
    def __init__(self, error: BaseException) -> None:
        self.error = error
        self.requests: list[unified_llm.Request] = []

    async def complete(self, request: unified_llm.Request) -> unified_llm.Response:
        self.requests.append(request)
        raise self.error


class _FailingStreamingClient:
    def __init__(self, error: BaseException) -> None:
        self.error = error
        self.requests: list[unified_llm.Request] = []

    def stream(self, request: unified_llm.Request):
        self.requests.append(request)

        async def _events():
            yield unified_llm.StreamEvent(
                type=unified_llm.StreamEventType.STREAM_START,
                response=unified_llm.Response(
                    id="resp-1",
                    model="fake-model",
                    provider="fake-provider",
                ),
            )
            yield unified_llm.StreamEvent(
                type=unified_llm.StreamEventType.TEXT_DELTA,
                delta="partial",
            )
            raise self.error

        return _events()

    async def complete(self, request: unified_llm.Request) -> unified_llm.Response:
        raise AssertionError("streaming sessions must not call complete()")


@pytest.mark.asyncio
async def test_session_process_input_authentication_error_emits_error_and_closes(
    caplog: pytest.LogCaptureFixture,
) -> None:
    client = _FailingCompleteClient(unified_llm.AuthenticationError("invalid key"))
    session = agent.Session(
        profile=_PromptProfile(id="fake-provider", model="fake-model"),
        llm_client=client,
    )
    stream = session.events()

    start_event = await _next_event(stream)
    assert start_event.kind == agent.EventKind.SESSION_START

    with caplog.at_level(logging.ERROR, logger="agent.session"):
        with pytest.raises(unified_llm.AuthenticationError, match="invalid key"):
            await session.process_input("Question")

    user_input_event = await _next_event(stream)
    assert user_input_event.kind == agent.EventKind.USER_INPUT
    assert user_input_event.data == {"content": "Question"}
    error_event = await _next_event(stream)
    assert error_event.kind == agent.EventKind.ERROR
    _assert_structured_model_error(
        error_event.data,
        kind="authentication",
        name="AuthenticationError",
        message="invalid key",
        retryable=False,
    )
    assert error_event.data["final_state"]["state"] == "closed"
    assert error_event.data["final_state"]["reason"] == "unrecoverable_error"
    end_event = await _next_event(stream)
    assert end_event.kind == agent.EventKind.SESSION_END
    assert end_event.data == {"state": "closed"}

    assert session.state == agent.SessionState.CLOSED
    assert [turn.text for turn in session.history] == ["Question"]
    assert len(client.requests) == 1
    assert any(
        record.levelno >= logging.ERROR
        and "Authentication error while processing session input" in record.message
        for record in caplog.records
    )


@pytest.mark.asyncio
async def test_session_process_input_context_length_error_emits_warning_and_stays_open(
    caplog: pytest.LogCaptureFixture,
) -> None:
    client = _FailingCompleteClient(
        unified_llm.ContextLengthError("too many tokens in the prompt")
    )
    session = agent.Session(
        profile=_PromptProfile(id="fake-provider", model="fake-model"),
        llm_client=client,
    )
    stream = session.events()

    start_event = await _next_event(stream)
    assert start_event.kind == agent.EventKind.SESSION_START

    with caplog.at_level(logging.WARNING, logger="agent.session"):
        with pytest.raises(
            unified_llm.ContextLengthError,
            match="too many tokens in the prompt",
        ):
            await session.process_input("Question")

    user_input_event = await _next_event(stream)
    assert user_input_event.kind == agent.EventKind.USER_INPUT
    assert user_input_event.data == {"content": "Question"}
    warning_event = await _next_event(stream)
    assert warning_event.kind == agent.EventKind.WARNING
    _assert_structured_model_error(
        warning_event.data,
        kind="context_length",
        name="ContextLengthError",
        message="too many tokens in the prompt",
        retryable=False,
    )
    assert "final_state" not in warning_event.data
    processing_end_event = await _next_event(stream)
    assert processing_end_event.kind == agent.EventKind.PROCESSING_END
    assert processing_end_event.data == {"state": "idle"}

    assert session.state == agent.SessionState.IDLE
    assert [turn.text for turn in session.history] == ["Question"]
    assert len(client.requests) == 1
    assert any(
        record.levelno >= logging.WARNING
        and "Context length error while processing session input" in record.message
        for record in caplog.records
    )


@pytest.mark.asyncio
@pytest.mark.parametrize(
    "error",
    [
        unified_llm.ProviderError(
            "service unavailable",
            provider="fake-provider",
            status_code=500,
        ),
        unified_llm.ProviderError(
            "rate limited",
            provider="fake-provider",
            status_code=429,
        ),
        unified_llm.NetworkError("temporary network failure"),
    ],
    ids=["provider-500", "provider-429", "network"],
)
async def test_session_process_input_transient_sdk_errors_do_not_trigger_retry_loops(
    error: BaseException,
) -> None:
    client = _FailingCompleteClient(error)
    session = agent.Session(
        profile=_PromptProfile(id="fake-provider", model="fake-model"),
        llm_client=client,
    )
    stream = session.events()

    start_event = await _next_event(stream)
    assert start_event.kind == agent.EventKind.SESSION_START

    with pytest.raises(type(error), match=str(error)):
        await session.process_input("Question")

    user_input_event = await _next_event(stream)
    assert user_input_event.kind == agent.EventKind.USER_INPUT
    warning_event = await _next_event(stream)
    assert warning_event.kind == agent.EventKind.WARNING
    assert warning_event.data["message"] == str(error)
    assert warning_event.data["retryable"] is True
    assert warning_event.data["provider"] == "fake-provider"
    assert warning_event.data["model"] == "fake-model"
    processing_end_event = await _next_event(stream)
    assert processing_end_event.kind == agent.EventKind.PROCESSING_END
    assert processing_end_event.data == {"state": "idle"}
    assert len(client.requests) == 1
    assert session.state == agent.SessionState.IDLE
    assert [turn.text for turn in session.history] == ["Question"]


@pytest.mark.asyncio
@pytest.mark.parametrize(
    "error",
    [unified_llm.NetworkError("temporary stream failure"), unified_llm.ProviderError(
        "upstream unavailable",
        provider="fake-provider",
        status_code=503,
    )],
    ids=["network", "provider-503"],
)
async def test_session_process_input_stream_errors_do_not_retry_locally(
    error: BaseException,
) -> None:
    client = _FailingStreamingClient(error)
    session = agent.Session(
        profile=_PromptProfile(
            id="fake-provider",
            model="fake-model",
            supports_streaming=True,
        ),
        llm_client=client,
    )
    stream = session.events()

    start_event = await _next_event(stream)
    assert start_event.kind == agent.EventKind.SESSION_START

    with pytest.raises(type(error), match=str(error)):
        await session.process_input("Question")

    user_input_event = await _next_event(stream)
    assert user_input_event.kind == agent.EventKind.USER_INPUT
    text_start_event = await _next_event(stream)
    assert text_start_event.kind == agent.EventKind.ASSISTANT_TEXT_START
    text_delta_event = await _next_event(stream)
    assert text_delta_event.kind == agent.EventKind.ASSISTANT_TEXT_DELTA
    assert text_delta_event.data["delta"] == "partial"
    warning_event = await _next_event(stream)
    assert warning_event.kind == agent.EventKind.WARNING
    assert warning_event.data["message"] == str(error)
    assert warning_event.data["retryable"] is True
    processing_end_event = await _next_event(stream)
    assert processing_end_event.kind == agent.EventKind.PROCESSING_END
    assert len(client.requests) == 1
    assert session.state == agent.SessionState.IDLE
    assert [turn.text for turn in session.history] == ["Question"]


@pytest.mark.asyncio
async def test_session_process_input_unexpected_exception_logs_and_closes(
    caplog: pytest.LogCaptureFixture,
) -> None:
    client = _FailingCompleteClient(RuntimeError("boom"))
    session = agent.Session(
        profile=_PromptProfile(id="fake-provider", model="fake-model"),
        llm_client=client,
    )
    stream = session.events()

    start_event = await _next_event(stream)
    assert start_event.kind == agent.EventKind.SESSION_START

    with caplog.at_level(logging.ERROR, logger="agent.session"):
        with pytest.raises(RuntimeError, match="boom"):
            await session.process_input("Question")

    user_input_event = await _next_event(stream)
    assert user_input_event.kind == agent.EventKind.USER_INPUT
    assert user_input_event.data == {"content": "Question"}
    error_event = await _next_event(stream)
    assert error_event.kind == agent.EventKind.ERROR
    _assert_structured_model_error(
        error_event.data,
        kind="runtime",
        name="RuntimeError",
        message="boom",
        retryable=False,
    )
    assert error_event.data["final_state"]["state"] == "closed"
    assert error_event.data["final_state"]["reason"] == "unrecoverable_error"
    end_event = await _next_event(stream)
    assert end_event.kind == agent.EventKind.SESSION_END
    assert end_event.data == {"state": "closed"}

    assert session.state == agent.SessionState.CLOSED
    assert [turn.text for turn in session.history] == ["Question"]
    assert len(client.requests) == 1
    assert any(
        record.levelno >= logging.ERROR
        and "Unexpected error while processing session input" in record.message
        for record in caplog.records
    )
