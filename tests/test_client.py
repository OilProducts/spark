from __future__ import annotations

import logging

import pytest

import unified_llm


class _RecordingAdapter:
    def __init__(self, name: str) -> None:
        self.name = name
        self.complete_requests: list[unified_llm.Request] = []
        self.stream_requests: list[unified_llm.Request] = []

    async def complete(self, request: unified_llm.Request) -> unified_llm.Response:
        self.complete_requests.append(request)
        return unified_llm.Response(
            provider=self.name,
            model=request.model,
            message=unified_llm.Message.assistant(f"{self.name}:{request.provider}"),
        )

    def stream(self, request: unified_llm.Request):
        self.stream_requests.append(request)

        async def _events():
            yield unified_llm.StreamEvent(
                type=unified_llm.StreamEventType.TEXT_DELTA,
                delta=f"{self.name}:{request.provider}",
            )

        return _events()


class _FailingCompleteAdapter:
    def __init__(self) -> None:
        self.name = "failing"
        self.complete_requests: list[unified_llm.Request] = []

    async def complete(self, request: unified_llm.Request) -> unified_llm.Response:
        self.complete_requests.append(request)
        raise unified_llm.ConfigurationError("bad config")


class _ClosingStream:
    def __init__(
        self,
        events: list[unified_llm.StreamEvent],
        error: BaseException | None = None,
    ) -> None:
        self._events = iter(events)
        self.error = error
        self.closed = False

    def __aiter__(self) -> _ClosingStream:
        return self

    async def __anext__(self) -> unified_llm.StreamEvent:
        try:
            return next(self._events)
        except StopIteration:
            raise StopAsyncIteration from None

    async def aclose(self) -> None:
        self.closed = True
        if self.error is not None:
            raise self.error


class _ClosingStreamAdapter:
    def __init__(self, error: BaseException | None = None) -> None:
        self.name = "failing"
        self.error = error
        self.complete_requests: list[unified_llm.Request] = []
        self.stream_requests: list[unified_llm.Request] = []
        self.last_stream: _ClosingStream | None = None

    def stream(self, request: unified_llm.Request) -> _ClosingStream:
        self.stream_requests.append(request)
        self.last_stream = _ClosingStream(
            [
                unified_llm.StreamEvent(
                    type=unified_llm.StreamEventType.TEXT_DELTA,
                    delta="partial",
                )
            ],
            error=self.error,
        )
        return self.last_stream


class _FailingStream:
    def __init__(self, error: BaseException) -> None:
        self.error = error
        self.closed = False

    def __aiter__(self) -> _FailingStream:
        return self

    async def __anext__(self) -> unified_llm.StreamEvent:
        raise self.error

    async def aclose(self) -> None:
        self.closed = True


class _FailingStreamAdapter:
    def __init__(self, error: BaseException) -> None:
        self.name = "failing"
        self.error = error
        self.complete_requests: list[unified_llm.Request] = []
        self.stream_requests: list[unified_llm.Request] = []
        self.last_stream: _FailingStream | None = None

    def stream(self, request: unified_llm.Request) -> _FailingStream:
        self.stream_requests.append(request)
        self.last_stream = _FailingStream(self.error)
        return self.last_stream


class _CloseRecorder:
    def __init__(self, name: str, error: BaseException | None = None) -> None:
        self.name = name
        self.error = error
        self.closed = False

    async def close(self) -> None:
        self.closed = True
        if self.error is not None:
            raise self.error


class _InitRecorderAdapter:
    def __init__(self, name: str, error: BaseException | None = None) -> None:
        self.name = name
        self.error = error
        self.initialize_calls = 0

    def initialize(self) -> None:
        self.initialize_calls += 1
        if self.error is not None:
            raise self.error


class _ToolChoiceRecorderAdapter:
    def __init__(self, name: str) -> None:
        self.name = name
        self.calls: list[str] = []

    def supports_tool_choice(self, mode: str) -> bool:
        self.calls.append(mode)
        return mode == "required"


class _EnvInitRecorderAdapter:
    instances: list[_EnvInitRecorderAdapter] = []
    name = "OPENAI"

    def __init__(self, **config: object) -> None:
        self.config = dict(config)
        self.initialize_calls = 0
        type(self).instances.append(self)

    def initialize(self) -> None:
        self.initialize_calls += 1


def test_client_from_env_registers_env_providers_in_order_and_passes_optional_config(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setenv("OPENAI_API_KEY", "openai-key")
    monkeypatch.setenv("OPENAI_BASE_URL", "https://openai.example/v1")
    monkeypatch.setenv("OPENAI_ORG_ID", "org-123")
    monkeypatch.setenv("OPENAI_PROJECT_ID", "project-456")
    monkeypatch.setenv("ANTHROPIC_API_KEY", "anthropic-key")
    monkeypatch.setenv("ANTHROPIC_BASE_URL", "https://anthropic.example/v1")
    monkeypatch.delenv("GEMINI_API_KEY", raising=False)
    monkeypatch.setenv("GOOGLE_API_KEY", "gemini-google-key")
    monkeypatch.setenv("GEMINI_BASE_URL", "https://gemini.example/v1")
    monkeypatch.setenv("OPENROUTER_API_KEY", "openrouter-key")
    monkeypatch.setenv("OPENROUTER_BASE_URL", "https://openrouter.example/api/v1")
    monkeypatch.setenv("OPENROUTER_HTTP_REFERER", "https://spark.example")
    monkeypatch.setenv("OPENROUTER_TITLE", "Spark")
    monkeypatch.setenv("LITELLM_BASE_URL", "https://litellm.example/v1")
    monkeypatch.delenv("LITELLM_API_KEY", raising=False)

    client = unified_llm.Client.from_env()

    assert list(client.providers) == ["openai", "anthropic", "gemini", "openrouter", "litellm"]
    assert client.default_provider == "openai"
    assert client.providers["openai"].config == {
        "api_key": "openai-key",
        "base_url": "https://openai.example/v1",
        "organization": "org-123",
        "project": "project-456",
    }
    assert client.providers["anthropic"].config == {
        "api_key": "anthropic-key",
        "base_url": "https://anthropic.example/v1",
    }
    assert client.providers["gemini"].config == {
        "api_key": "gemini-google-key",
        "base_url": "https://gemini.example/v1beta",
    }
    assert client.providers["openrouter"].config == {
        "api_key": "openrouter-key",
        "base_url": "https://openrouter.example/api/v1",
    }
    assert client.providers["openrouter"].default_headers == {
        "HTTP-Referer": "https://spark.example",
        "X-Title": "Spark",
    }
    assert client.providers["litellm"].config == {
        "api_key": None,
        "base_url": "https://litellm.example/v1",
    }


def test_client_from_env_selects_the_first_registered_provider_as_default(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.delenv("OPENAI_API_KEY", raising=False)
    monkeypatch.delenv("OPENROUTER_API_KEY", raising=False)
    monkeypatch.delenv("LITELLM_BASE_URL", raising=False)
    monkeypatch.setenv("ANTHROPIC_API_KEY", "anthropic-key")
    monkeypatch.setenv("GEMINI_API_KEY", "gemini-key")

    client = unified_llm.Client.from_env()

    assert list(client.providers) == ["anthropic", "gemini"]
    assert client.default_provider == "anthropic"


@pytest.mark.asyncio
async def test_client_complete_routes_by_explicit_provider_without_touching_model_strings(
) -> None:
    openai = _RecordingAdapter("openai")
    anthropic = _RecordingAdapter("anthropic")
    client = unified_llm.Client(
        providers={"OpenAI": openai, "ANTHROPIC": anthropic},
        default_provider="AnThRoPiC",
    )
    request = unified_llm.Request(
        model="gPt-5.2",
        messages=[unified_llm.Message.user("hello")],
        provider="OpEnAi",
        provider_options={
            "openai": {"reasoning": {"effort": "high"}},
            "anthropic": {"beta_headers": ["ignore-me"]},
        },
    )

    response = await client.complete(request)

    assert list(client.providers) == ["openai", "anthropic"]
    assert client.default_provider == "anthropic"
    assert response.provider == "openai"
    assert response.model == "gPt-5.2"
    assert response.text == "openai:openai"
    assert request.provider == "OpEnAi"
    assert len(openai.complete_requests) == 1
    assert anthropic.complete_requests == []
    assert openai.complete_requests[0].provider == "openai"
    assert openai.complete_requests[0].model == "gPt-5.2"
    assert openai.complete_requests[0].provider_options == request.provider_options
    assert openai.complete_requests[0] is not request


@pytest.mark.asyncio
async def test_client_complete_uses_the_default_provider_and_exposes_the_selected_provider_name(
) -> None:
    openai = _RecordingAdapter("openai")
    anthropic = _RecordingAdapter("anthropic")
    client = unified_llm.Client(
        providers={"openai": openai, "anthropic": anthropic},
        default_provider="AnThRoPiC",
    )
    request = unified_llm.Request(
        model="gpt-5.2",
        messages=[unified_llm.Message.user("hello")],
    )

    response = await client.complete(request)

    assert response.provider == "anthropic"
    assert response.model == "gpt-5.2"
    assert response.text == "anthropic:anthropic"
    assert request.provider is None
    assert client.default_provider == "anthropic"
    assert openai.complete_requests == []
    assert anthropic.complete_requests[0].provider == "anthropic"
    assert anthropic.complete_requests[0] is not request


def test_client_initializes_adapters_during_explicit_registration() -> None:
    openai = _InitRecorderAdapter("OpenAI")

    client = unified_llm.Client(
        providers={"OpenAI": openai},
        default_provider="OPENAI",
    )

    assert list(client.providers) == ["openai"]
    assert client.default_provider == "openai"
    assert openai.initialize_calls == 1


def test_client_initializes_env_adapters_during_from_env(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    import unified_llm.adapters as adapters_module

    monkeypatch.delenv("OPENAI_BASE_URL", raising=False)
    monkeypatch.delenv("OPENAI_ORG_ID", raising=False)
    monkeypatch.delenv("OPENAI_PROJECT_ID", raising=False)
    monkeypatch.delenv("ANTHROPIC_API_KEY", raising=False)
    monkeypatch.delenv("ANTHROPIC_BASE_URL", raising=False)
    monkeypatch.delenv("GEMINI_API_KEY", raising=False)
    monkeypatch.delenv("GEMINI_BASE_URL", raising=False)
    monkeypatch.delenv("GOOGLE_API_KEY", raising=False)
    monkeypatch.setenv("OPENAI_API_KEY", "openai-key")
    monkeypatch.setattr(adapters_module, "OpenAIAdapter", _EnvInitRecorderAdapter)
    _EnvInitRecorderAdapter.instances.clear()

    client = unified_llm.Client.from_env()

    assert list(client.providers) == ["openai"]
    assert client.default_provider == "openai"
    assert len(_EnvInitRecorderAdapter.instances) == 1
    assert _EnvInitRecorderAdapter.instances[0].config == {"api_key": "openai-key"}
    assert _EnvInitRecorderAdapter.instances[0].initialize_calls == 1


def test_client_initialization_propagates_sdk_errors_without_logging(
    caplog: pytest.LogCaptureFixture,
) -> None:
    adapter = _InitRecorderAdapter(
        "broken",
        error=unified_llm.ConfigurationError("unable to initialize"),
    )

    with caplog.at_level(logging.ERROR, logger="unified_llm.client"):
        with pytest.raises(unified_llm.ConfigurationError, match="unable to initialize"):
            unified_llm.Client(providers={"BROKEN": adapter})

    assert adapter.initialize_calls == 1
    assert not any(
        record.name == "unified_llm.client"
        and "Unexpected error initializing provider broken" in record.message
        for record in caplog.records
    )


def test_client_initialization_logs_unexpected_failures_and_re_raises(
    caplog: pytest.LogCaptureFixture,
    capsys: pytest.CaptureFixture[str],
) -> None:
    adapter = _InitRecorderAdapter("broken", error=RuntimeError("boom"))

    with caplog.at_level(logging.ERROR, logger="unified_llm.client"):
        with pytest.raises(RuntimeError, match="boom"):
            unified_llm.Client(providers={"BROKEN": adapter})

    captured = capsys.readouterr()

    assert captured.out == ""
    assert captured.err == ""
    assert adapter.initialize_calls == 1
    assert any(
        record.name == "unified_llm.client"
        and "Unexpected error initializing provider broken" in record.message
        for record in caplog.records
    )


@pytest.mark.asyncio
async def test_client_complete_raises_configuration_error_for_unknown_provider(
    ) -> None:
    openai = _RecordingAdapter("openai")
    client = unified_llm.Client(
        providers={"OpenAI": openai},
        default_provider="openai",
    )
    request = unified_llm.Request(
        model="gpt-5.2",
        messages=[unified_llm.Message.user("hello")],
        provider="MiSsInG",
    )

    with pytest.raises(unified_llm.ConfigurationError, match="Unknown provider 'missing'"):
        await client.complete(request)

    assert openai.complete_requests == []


@pytest.mark.asyncio
async def test_client_complete_propagates_sdk_errors_without_retry() -> None:
    adapter = _FailingCompleteAdapter()
    client = unified_llm.Client(providers={"fake": adapter}, default_provider="fake")
    request = unified_llm.Request(
        model="gpt-5.2",
        messages=[unified_llm.Message.user("hello")],
    )

    with pytest.raises(unified_llm.ConfigurationError, match="bad config"):
        await client.complete(request)

    assert len(adapter.complete_requests) == 1


@pytest.mark.asyncio
async def test_client_stream_propagates_sdk_errors_without_retry() -> None:
    adapter = _FailingStreamAdapter(unified_llm.ConfigurationError("bad config"))
    client = unified_llm.Client(providers={"fake": adapter}, default_provider="fake")
    request = unified_llm.Request(
        model="gpt-5.2",
        messages=[unified_llm.Message.user("hello")],
    )

    stream = client.stream(request)

    with pytest.raises(unified_llm.ConfigurationError, match="bad config"):
        await stream.__anext__()

    assert len(adapter.stream_requests) == 1
    assert adapter.last_stream is not None
    assert adapter.last_stream.closed is True


def test_client_supports_tool_choice_delegates_and_falls_back_when_missing() -> None:
    supported = _ToolChoiceRecorderAdapter("supported")
    unsupported = _RecordingAdapter("unsupported")
    client = unified_llm.Client(
        providers={"OpenAI": supported, "ANTHROPIC": unsupported},
        default_provider="OpenAI",
    )

    assert client.supports_tool_choice("required") is True
    assert client.supports_tool_choice("auto") is False
    assert client.supports_tool_choice("required", provider="ANTHROPIC") is False
    assert supported.calls == ["required", "auto"]
    assert list(client.providers) == ["openai", "anthropic"]
    assert client.default_provider == "openai"


@pytest.mark.asyncio
async def test_client_stream_uses_default_provider_and_keeps_request_state_untouched(
) -> None:
    openai = _RecordingAdapter("openai")
    anthropic = _RecordingAdapter("anthropic")
    client = unified_llm.Client(
        providers={"openai": openai, "anthropic": anthropic},
        default_provider="openai",
    )
    request = unified_llm.Request(
        model="claude-opus-4-6",
        messages=[unified_llm.Message.user("hello")],
    )

    stream = client.stream(request)

    assert openai.stream_requests == []

    events = [event async for event in stream]

    assert request.provider is None
    assert openai.stream_requests[0].provider == "openai"
    assert openai.stream_requests[0] is not request
    assert anthropic.stream_requests == []
    assert events == [
        unified_llm.StreamEvent(
            type=unified_llm.StreamEventType.TEXT_DELTA,
            delta="openai:openai",
        ),
        unified_llm.StreamEvent(
            type=unified_llm.StreamEventType.FINISH,
            finish_reason=unified_llm.FinishReason(reason="stop"),
            usage=unified_llm.Usage(),
            response=unified_llm.Response(
                provider="openai",
                model="claude-opus-4-6",
                finish_reason=unified_llm.FinishReason(reason="stop"),
                message=unified_llm.Message.assistant("openai:openai"),
            ),
        ),
    ]


@pytest.mark.asyncio
async def test_client_stream_can_be_closed_and_logs_close_failures(
    caplog: pytest.LogCaptureFixture,
    capsys: pytest.CaptureFixture[str],
) -> None:
    adapter = _ClosingStreamAdapter(error=RuntimeError("boom"))
    client = unified_llm.Client(providers={"fake": adapter}, default_provider="fake")
    request = unified_llm.Request(
        model="gpt-5.2",
        messages=[unified_llm.Message.user("hello")],
    )

    stream = client.stream(request)
    first_event = await stream.__anext__()

    assert first_event.type == unified_llm.StreamEventType.TEXT_DELTA
    assert adapter.stream_requests[0].provider == "fake"
    assert adapter.last_stream is not None
    assert adapter.last_stream.closed is False

    with caplog.at_level(logging.ERROR, logger="unified_llm.streaming"):
        await stream.close()

    captured = capsys.readouterr()

    assert captured.out == ""
    assert captured.err == ""
    assert adapter.last_stream.closed is True
    assert any(
        record.name == "unified_llm.streaming"
        and "Unexpected error closing stream iterator" in record.message
        for record in caplog.records
    )


@pytest.mark.asyncio
async def test_client_stream_can_be_closed_before_iteration_without_invoking_adapter(
    capsys: pytest.CaptureFixture[str],
) -> None:
    adapter = _ClosingStreamAdapter()
    client = unified_llm.Client(providers={"fake": adapter}, default_provider="fake")
    request = unified_llm.Request(
        model="gpt-5.2",
        messages=[unified_llm.Message.user("hello")],
    )

    stream = client.stream(request)
    await stream.close()

    captured = capsys.readouterr()

    assert captured.out == ""
    assert captured.err == ""
    assert adapter.stream_requests == []
    assert adapter.last_stream is None

    with pytest.raises(StopAsyncIteration):
        await stream.__anext__()


@pytest.mark.asyncio
async def test_client_complete_raises_configuration_error_when_no_provider_can_be_resolved(
) -> None:
    openai = _RecordingAdapter("openai")
    client = unified_llm.Client(providers={"openai": openai})
    request = unified_llm.Request(
        model="gpt-5.2",
        messages=[unified_llm.Message.user("hello")],
    )

    with pytest.raises(unified_llm.ConfigurationError):
        await client.complete(request)

    assert openai.complete_requests == []


def test_get_default_client_initializes_from_env_lazily(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    previous = unified_llm.get_default_client()
    try:
        unified_llm.set_default_client(None)
        monkeypatch.delenv("OPENAI_API_KEY", raising=False)
        monkeypatch.setenv("ANTHROPIC_API_KEY", "anthropic-key")
        monkeypatch.delenv("GEMINI_API_KEY", raising=False)
        monkeypatch.delenv("GOOGLE_API_KEY", raising=False)

        client = unified_llm.get_default_client()

        assert list(client.providers) == ["anthropic"]
        assert client.default_provider == "anthropic"
        assert client.providers["anthropic"].config == {"api_key": "anthropic-key"}
        assert unified_llm.get_default_client() is client
    finally:
        unified_llm.set_default_client(previous)


@pytest.mark.asyncio
async def test_client_close_logs_unexpected_failures_and_continues_closing_remaining_adapters(
    caplog: pytest.LogCaptureFixture,
) -> None:
    good = _CloseRecorder("good")
    bad = _CloseRecorder("bad", error=RuntimeError("boom"))
    client = unified_llm.Client(providers={"good": good, "bad": bad})

    with caplog.at_level(logging.ERROR, logger="unified_llm.client"):
        await client.close()

    assert good.closed is True
    assert bad.closed is True
    assert any(
        record.name == "unified_llm.client"
        and "Unexpected error closing provider bad" in record.message
        for record in caplog.records
    )


@pytest.mark.asyncio
async def test_client_close_preserves_actionable_sdk_errors_from_close_hooks(
) -> None:
    good = _CloseRecorder("good")
    failing = _CloseRecorder(
        "failing",
        error=unified_llm.ConfigurationError("unable to close"),
    )
    client = unified_llm.Client(providers={"good": good, "failing": failing})

    with pytest.raises(unified_llm.ConfigurationError, match="unable to close"):
        await client.close()

    assert good.closed is True
    assert failing.closed is True
