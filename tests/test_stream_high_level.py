from __future__ import annotations

import asyncio
import importlib
import json
import logging
from contextlib import suppress

import httpx
import pytest

import unified_llm
import unified_llm.generation as generation_mod
from unified_llm.errors import AbortError

retry_mod = importlib.import_module("unified_llm.retry")


def _tool_call(
    call_id: str,
    name: str,
    arguments: str | dict[str, object],
    raw_arguments: str | None = None,
) -> unified_llm.ToolCall:
    if raw_arguments is None and isinstance(arguments, str):
        raw_arguments = arguments
    return unified_llm.ToolCall(
        id=call_id,
        name=name,
        arguments=arguments,
        raw_arguments=raw_arguments,
    )


class _SequencedStream:
    def __init__(
        self,
        events: list[object],
        *,
        close_error: BaseException | None = None,
    ) -> None:
        self._events = iter(events)
        self.close_error = close_error
        self.closed = False

    def __aiter__(self) -> _SequencedStream:
        return self

    async def __anext__(self) -> unified_llm.StreamEvent:
        try:
            item = next(self._events)
        except StopIteration:
            raise StopAsyncIteration from None

        if isinstance(item, BaseException):
            raise item
        return item

    async def aclose(self) -> None:
        self.closed = True
        if self.close_error is not None:
            raise self.close_error


class _SequencedStreamAdapter:
    def __init__(self, name: str, behaviors: list[object]) -> None:
        self.name = name
        self._behaviors = list(behaviors)
        self.stream_requests: list[unified_llm.Request] = []
        self.opened_streams: list[_SequencedStream] = []

    def stream(self, request: unified_llm.Request) -> _SequencedStream:
        self.stream_requests.append(request)
        if not self._behaviors:
            raise AssertionError(f"{self.name} received more stream requests than expected")

        behavior = self._behaviors.pop(0)
        if callable(behavior):
            behavior = behavior(request)
        if isinstance(behavior, BaseException):
            raise behavior
        if isinstance(behavior, _SequencedStream):
            self.opened_streams.append(behavior)
            return behavior

        stream = _SequencedStream(list(behavior))
        self.opened_streams.append(stream)
        return stream


def _text_delta(text: str) -> unified_llm.StreamEvent:
    return unified_llm.StreamEvent(
        type=unified_llm.StreamEventType.TEXT_DELTA,
        delta=text,
    )


def _provider_event(raw: object) -> unified_llm.StreamEvent:
    return unified_llm.StreamEvent(
        type=unified_llm.StreamEventType.PROVIDER_EVENT,
        raw=raw,
    )


def _tool_call_event(
    event_type: unified_llm.StreamEventType,
    call_id: str,
    name: str,
    arguments: str | dict[str, object],
) -> unified_llm.StreamEvent:
    return unified_llm.StreamEvent(
        type=event_type,
        tool_call=_tool_call(call_id, name, arguments),
    )


def _sse_event(event: str, payload: dict[str, object]) -> str:
    return f"event: {event}\ndata: {json.dumps(payload, separators=(',', ':'), sort_keys=True)}\n\n"


def _make_anthropic_stream_transport(
    payload: str,
    *,
    headers: dict[str, str] | None = None,
    status_code: int = 200,
) -> tuple[list[httpx.Request], httpx.MockTransport]:
    captured_requests: list[httpx.Request] = []

    async def handler(request: httpx.Request) -> httpx.Response:
        captured_requests.append(request)
        return httpx.Response(
            status_code,
            headers={"content-type": "text/event-stream", **dict(headers or {})},
            content=payload.encode("utf-8"),
        )

    return captured_requests, httpx.MockTransport(handler)


@pytest.mark.asyncio
async def test_stream_exposes_partial_response_and_response_after_completion(
    capsys: pytest.CaptureFixture[str],
) -> None:
    adapter = _SequencedStreamAdapter(
        "fake",
        [
            [
                _text_delta("Hel"),
                _text_delta("lo"),
                _provider_event({"kind": "noise"}),
            ]
        ],
    )
    client = unified_llm.Client(providers={"fake": adapter}, default_provider="fake")
    stream = unified_llm.stream(
        model="gpt-5.2",
        prompt="hello",
        client=client,
    )

    first = await stream.__anext__()
    assert first.type == unified_llm.StreamEventType.TEXT_DELTA
    assert first.delta == "Hel"
    assert stream.partial_response.text == "Hel"

    second = await stream.__anext__()
    assert second.type == unified_llm.StreamEventType.TEXT_DELTA
    assert second.delta == "lo"
    assert stream.partial_response.text == "Hello"

    third = await stream.__anext__()
    assert third.type == unified_llm.StreamEventType.PROVIDER_EVENT

    response = await stream.response()
    await stream.close()

    captured = capsys.readouterr()

    assert captured.out == ""
    assert captured.err == ""
    assert response.text == "Hello"
    assert response.finish_reason.reason == "stop"
    assert stream.partial_response.text == "Hello"
    assert adapter.stream_requests[0].model == "gpt-5.2"
    assert adapter.opened_streams[0].closed is True


@pytest.mark.asyncio
async def test_stream_defaults_omitted_model_from_provider_and_client_default(
    capsys: pytest.CaptureFixture[str],
) -> None:
    explicit_adapter = _SequencedStreamAdapter(
        "openai",
        [[_text_delta("explicit")]],
    )
    explicit_client = unified_llm.Client(
        providers={"openai": explicit_adapter},
        default_provider=None,
    )
    default_adapter = _SequencedStreamAdapter(
        "gemini",
        [[_text_delta("default")]],
    )
    default_client = unified_llm.Client(
        providers={"gemini": default_adapter},
        default_provider="gemini",
    )
    pass_through_adapter = _SequencedStreamAdapter(
        "openai",
        [[_text_delta("custom")]],
    )
    pass_through_client = unified_llm.Client(
        providers={"openai": pass_through_adapter},
        default_provider="openai",
    )

    explicit_stream = unified_llm.stream(
        prompt="hello",
        provider="openai",
        client=explicit_client,
    )
    default_stream = unified_llm.stream(
        prompt="hello",
        client=default_client,
    )
    pass_through_stream = unified_llm.stream(
        model="custom-model",
        prompt="hello",
        provider="openai",
        client=pass_through_client,
    )

    assert (await explicit_stream.response()).text == "explicit"
    assert (await default_stream.response()).text == "default"
    assert (await pass_through_stream.response()).text == "custom"

    captured = capsys.readouterr()

    assert captured.out == ""
    assert captured.err == ""
    assert explicit_adapter.stream_requests[0].model == "gpt-5.2"
    assert explicit_adapter.stream_requests[0].provider == "openai"
    assert default_adapter.stream_requests[0].model == "gemini-3.1-pro-preview"
    assert default_adapter.stream_requests[0].provider == "gemini"
    assert pass_through_adapter.stream_requests[0].model == "custom-model"


@pytest.mark.asyncio
async def test_stream_response_preserves_anthropic_thinking_metadata(
    capsys: pytest.CaptureFixture[str],
) -> None:
    payload = "".join(
        [
            _sse_event(
                "message_start",
                {
                    "type": "message_start",
                    "message": {
                        "id": "msg_stream_metadata",
                        "type": "message",
                        "role": "assistant",
                        "model": "claude-sonnet-4-5",
                        "content": [],
                        "stop_reason": None,
                        "stop_sequence": None,
                        "usage": {
                            "input_tokens": 3,
                            "output_tokens": 0,
                        },
                    },
                },
            ),
            _sse_event(
                "content_block_start",
                {
                    "type": "content_block_start",
                    "index": 0,
                    "content_block": {
                        "type": "thinking",
                        "thinking": "reasoning ",
                        "signature": "sig-123",
                    },
                },
            ),
            _sse_event(
                "content_block_delta",
                {
                    "type": "content_block_delta",
                    "index": 0,
                    "delta": {
                        "type": "thinking_delta",
                        "thinking": "thou",
                    },
                },
            ),
            _sse_event(
                "content_block_delta",
                {
                    "type": "content_block_delta",
                    "index": 0,
                    "delta": {
                        "type": "signature_delta",
                        "signature": "sig-123",
                    },
                },
            ),
            _sse_event(
                "content_block_stop",
                {
                    "type": "content_block_stop",
                    "index": 0,
                },
            ),
            _sse_event(
                "content_block_start",
                {
                    "type": "content_block_start",
                    "index": 1,
                    "content_block": {
                        "type": "redacted_thinking",
                        "data": "opaque-123",
                    },
                },
            ),
            _sse_event(
                "content_block_stop",
                {
                    "type": "content_block_stop",
                    "index": 1,
                },
            ),
            _sse_event(
                "message_delta",
                {
                    "type": "message_delta",
                    "delta": {
                        "stop_reason": "end_turn",
                    },
                    "usage": {
                        "output_tokens": 5,
                        "cache_read_input_tokens": 2,
                        "cache_creation_input_tokens": 1,
                    },
                },
            ),
            _sse_event(
                "message_stop",
                {
                    "type": "message_stop",
                },
            ),
        ]
    )
    captured_requests, transport = _make_anthropic_stream_transport(
        payload,
        headers={
            "anthropic-ratelimit-requests-remaining": "4",
        },
    )
    adapter = unified_llm.AnthropicAdapter(
        api_key="stream-key",
        transport=transport,
    )
    client = unified_llm.Client(
        providers={"anthropic": adapter},
        default_provider="anthropic",
    )
    stream = unified_llm.stream(
        model="claude-sonnet-4-5",
        prompt="hello",
        client=client,
    )

    response = await stream.response()
    await stream.close()

    captured = capsys.readouterr()

    assert captured.out == ""
    assert captured.err == ""
    assert len(captured_requests) == 1
    assert captured_requests[0].method == "POST"
    assert captured_requests[0].url.path == "/v1/messages"
    assert response.provider == "anthropic"
    assert response.model == "claude-sonnet-4-5"
    assert response.finish_reason.reason == "stop"
    assert response.text == ""
    assert response.reasoning == "reasoning thouopaque-123"
    assert response.usage.input_tokens == 3
    assert response.usage.output_tokens == 5
    assert response.usage.total_tokens == 8
    assert response.usage.cache_read_tokens == 2
    assert response.usage.cache_write_tokens == 1
    assert response.rate_limit == unified_llm.RateLimitInfo(
        requests_remaining=4,
        requests_limit=None,
        tokens_remaining=None,
        tokens_limit=None,
        reset_at=None,
    )
    assert isinstance(response.raw, list)
    assert response.raw[-1] == {"type": "message_stop"}
    assert [part.kind for part in response.message.content] == [
        unified_llm.ContentKind.THINKING,
        unified_llm.ContentKind.REDACTED_THINKING,
    ]
    assert response.message.content[0].thinking is not None
    assert response.message.content[0].thinking.signature == "sig-123"
    assert response.message.content[0].thinking.redacted is False
    assert response.message.content[1].thinking is not None
    assert response.message.content[1].thinking.redacted is True
    assert response.message.content[1].thinking.text == "opaque-123"
    assert stream.partial_response.reasoning == response.reasoning
    assert stream.partial_response.message.content[0].thinking is not None
    assert stream.partial_response.message.content[0].thinking.signature == "sig-123"
    assert stream.partial_response.message.content[1].thinking is not None
    assert stream.partial_response.message.content[1].thinking.redacted is True


@pytest.mark.asyncio
async def test_stream_text_stream_filters_non_text_events(
    capsys: pytest.CaptureFixture[str],
) -> None:
    adapter = _SequencedStreamAdapter(
        "fake",
        [
            [
                _text_delta("Hel"),
                _provider_event({"kind": "noise"}),
                _text_delta("lo"),
            ]
        ],
    )
    client = unified_llm.Client(providers={"fake": adapter}, default_provider="fake")
    stream = unified_llm.stream(
        model="gpt-5.2",
        prompt="hello",
        client=client,
    )

    deltas = [delta async for delta in stream.text_stream]
    response = await stream.response()
    await stream.close()

    captured = capsys.readouterr()

    assert captured.out == ""
    assert captured.err == ""
    assert deltas == ["Hel", "lo"]
    assert response.text == "Hello"
    assert adapter.opened_streams[0].closed is True


@pytest.mark.asyncio
async def test_stream_text_stream_closes_provider_stream_when_iteration_is_abandoned(
    capsys: pytest.CaptureFixture[str],
) -> None:
    adapter = _SequencedStreamAdapter(
        "fake",
        [
            [
                _text_delta("Hel"),
                _text_delta("lo"),
                _provider_event({"kind": "noise"}),
            ]
        ],
    )
    client = unified_llm.Client(providers={"fake": adapter}, default_provider="fake")
    stream = unified_llm.stream(
        model="gpt-5.2",
        prompt="hello",
        client=client,
    )

    deltas: list[str] = []
    async for delta in stream.text_stream:
        deltas.append(delta)
        break

    captured = capsys.readouterr()

    assert captured.out == ""
    assert captured.err == ""
    assert deltas == ["Hel"]
    assert stream.partial_response.text == "Hel"
    assert adapter.opened_streams[0].closed is True


@pytest.mark.asyncio
async def test_stream_closes_provider_stream_when_iteration_is_abandoned(
    capsys: pytest.CaptureFixture[str],
) -> None:
    adapter = _SequencedStreamAdapter(
        "fake",
        [
            [
                _text_delta("Hel"),
                _text_delta("lo"),
                _provider_event({"kind": "noise"}),
            ]
        ],
    )
    client = unified_llm.Client(providers={"fake": adapter}, default_provider="fake")
    stream = unified_llm.stream(
        model="gpt-5.2",
        prompt="hello",
        client=client,
    )

    events: list[unified_llm.StreamEvent] = []
    async for event in stream:
        events.append(event)
        break

    captured = capsys.readouterr()

    assert captured.out == ""
    assert captured.err == ""
    assert [event.delta for event in events] == ["Hel"]
    assert stream.partial_response.text == "Hel"
    assert adapter.opened_streams[0].closed is True


@pytest.mark.asyncio
async def test_stream_retries_initial_stream_failures_but_not_after_partial_data(
    monkeypatch: pytest.MonkeyPatch,
    capsys: pytest.CaptureFixture[str],
) -> None:
    sleep_calls: list[float] = []

    async def fake_sleep(delay: float) -> None:
        sleep_calls.append(delay)

    monkeypatch.setattr(retry_mod.random, "uniform", lambda a, b: 1.0)
    monkeypatch.setattr(generation_mod, "_sleep", fake_sleep)

    adapter = _SequencedStreamAdapter(
        "fake",
        [
            [unified_llm.RateLimitError("retry later", provider="fake")],
            [
                _text_delta("Hello"),
            ],
        ],
    )
    client = unified_llm.Client(providers={"fake": adapter}, default_provider="fake")
    stream = unified_llm.stream(
        model="gpt-5.2",
        prompt="hello",
        client=client,
        max_retries=1,
    )

    first = await stream.__anext__()

    captured = capsys.readouterr()

    assert captured.out == ""
    assert captured.err == ""
    assert first.type == unified_llm.StreamEventType.TEXT_DELTA
    assert first.delta == "Hello"
    assert sleep_calls == [1.0]
    assert len(adapter.stream_requests) == 2
    await stream.close()
    assert adapter.opened_streams[0].closed is True
    assert adapter.opened_streams[1].closed is True

    adapter_after_partial = _SequencedStreamAdapter(
        "fake",
        [
            [
                _text_delta("Hel"),
                unified_llm.RateLimitError("retry later", provider="fake"),
            ]
        ],
    )
    partial_client = unified_llm.Client(
        providers={"fake": adapter_after_partial},
        default_provider="fake",
    )
    partial_stream = unified_llm.stream(
        model="gpt-5.2",
        prompt="hello",
        client=partial_client,
        max_retries=1,
    )

    first_partial = await partial_stream.__anext__()
    assert first_partial.delta == "Hel"
    assert partial_stream.partial_response.text == "Hel"

    second_partial = await partial_stream.__anext__()
    assert second_partial.type == unified_llm.StreamEventType.ERROR
    assert isinstance(second_partial.error, unified_llm.RateLimitError)
    assert second_partial.error.message == "retry later"
    assert second_partial.response is not None
    assert second_partial.response.text == "Hel"
    assert second_partial.response.finish_reason.reason == "error"

    response = await partial_stream.response()
    assert response.text == "Hel"
    assert response.finish_reason.reason == "error"
    assert partial_stream.partial_response.text == "Hel"
    assert partial_stream.partial_response.finish_reason.reason == "error"

    await partial_stream.close()

    captured = capsys.readouterr()

    assert captured.out == ""
    assert captured.err == ""
    assert len(adapter_after_partial.stream_requests) == 1
    assert adapter_after_partial.opened_streams[0].closed is True


@pytest.mark.asyncio
async def test_stream_retry_sleep_is_cut_off_by_total_timeout(
    monkeypatch: pytest.MonkeyPatch,
    capsys: pytest.CaptureFixture[str],
) -> None:
    sleep_started = asyncio.Event()
    sleep_cancelled = asyncio.Event()
    sleep_calls: list[float] = []
    blocker = asyncio.Event()

    async def fake_sleep(delay: float) -> None:
        sleep_calls.append(delay)
        sleep_started.set()
        try:
            await blocker.wait()
        except asyncio.CancelledError:
            sleep_cancelled.set()
            raise

    monkeypatch.setattr(retry_mod.random, "uniform", lambda a, b: 1.0)
    monkeypatch.setattr(generation_mod, "_sleep", fake_sleep)

    adapter = _SequencedStreamAdapter(
        "fake",
        [
            [unified_llm.RateLimitError("retry later", provider="fake")],
        ],
    )
    client = unified_llm.Client(providers={"fake": adapter}, default_provider="fake")
    stream = unified_llm.stream(
        model="gpt-5.2",
        prompt="hello",
        client=client,
        timeout=unified_llm.TimeoutConfig(total=0.05),
        max_retries=1,
    )
    task = asyncio.create_task(stream.__anext__())

    try:
        await asyncio.wait_for(sleep_started.wait(), timeout=0.5)

        with pytest.raises(unified_llm.RequestTimeoutError) as excinfo:
            await asyncio.wait_for(task, timeout=0.5)

        captured = capsys.readouterr()

        assert captured.out == ""
        assert captured.err == ""
        assert excinfo.value.scope == "stream"
        assert sleep_calls == [1.0]
        assert sleep_cancelled.is_set()
        assert len(adapter.stream_requests) == 1
    finally:
        if not task.done():
            task.cancel()
            with suppress(asyncio.CancelledError):
                await task
        await stream.close()


@pytest.mark.asyncio
async def test_stream_retry_sleep_is_cut_off_by_abort_signal(
    monkeypatch: pytest.MonkeyPatch,
    capsys: pytest.CaptureFixture[str],
) -> None:
    sleep_started = asyncio.Event()
    sleep_cancelled = asyncio.Event()
    sleep_calls: list[float] = []
    blocker = asyncio.Event()
    controller = unified_llm.AbortController()

    async def fake_sleep(delay: float) -> None:
        sleep_calls.append(delay)
        sleep_started.set()
        try:
            await blocker.wait()
        except asyncio.CancelledError:
            sleep_cancelled.set()
            raise

    monkeypatch.setattr(retry_mod.random, "uniform", lambda a, b: 1.0)
    monkeypatch.setattr(generation_mod, "_sleep", fake_sleep)

    adapter = _SequencedStreamAdapter(
        "fake",
        [
            [unified_llm.RateLimitError("retry later", provider="fake")],
        ],
    )
    client = unified_llm.Client(providers={"fake": adapter}, default_provider="fake")
    stream = unified_llm.stream(
        model="gpt-5.2",
        prompt="hello",
        client=client,
        abort_signal=controller.signal,
        max_retries=1,
    )
    task = asyncio.create_task(stream.__anext__())

    try:
        await asyncio.wait_for(sleep_started.wait(), timeout=0.5)
        controller.abort("stop now")

        with pytest.raises(AbortError) as excinfo:
            await asyncio.wait_for(task, timeout=0.5)

        captured = capsys.readouterr()

        assert captured.out == ""
        assert captured.err == ""
        assert excinfo.value.scope == "stream"
        assert excinfo.value.reason == "stop now"
        assert sleep_calls == [1.0]
        assert sleep_cancelled.is_set()
        assert len(adapter.stream_requests) == 1
    finally:
        if not task.done():
            task.cancel()
            with suppress(asyncio.CancelledError):
                await task
        await stream.close()


@pytest.mark.asyncio
@pytest.mark.parametrize(
    ("timeout", "expected_scope"),
    [
        (unified_llm.TimeoutConfig(total=0.0), "stream"),
        (unified_llm.TimeoutConfig(per_step=0.0), "stream step"),
        (unified_llm.TimeoutConfig(stream_read=0.0), "stream_read"),
    ],
)
async def test_stream_enforces_total_per_step_and_stream_read_timeouts(
    timeout: unified_llm.TimeoutConfig,
    expected_scope: str,
    capsys: pytest.CaptureFixture[str],
) -> None:
    adapter = _SequencedStreamAdapter(
        "fake",
        [
            [
                _text_delta("Hello"),
            ]
        ],
    )
    client = unified_llm.Client(providers={"fake": adapter}, default_provider="fake")
    stream = unified_llm.stream(
        model="gpt-5.2",
        prompt="hello",
        client=client,
        timeout=timeout,
    )

    with pytest.raises(unified_llm.RequestTimeoutError) as excinfo:
        await stream.__anext__()

    captured = capsys.readouterr()

    assert captured.out == ""
    assert captured.err == ""
    assert excinfo.value.scope == expected_scope
    if adapter.opened_streams:
        assert adapter.opened_streams[0].closed is True
    await stream.close()


@pytest.mark.asyncio
async def test_stream_can_be_closed_and_logs_close_failures(
    caplog: pytest.LogCaptureFixture,
    capsys: pytest.CaptureFixture[str],
) -> None:
    adapter = _SequencedStreamAdapter(
        "fake",
        [
            _SequencedStream(
                [_text_delta("partial")],
                close_error=RuntimeError("boom"),
            )
        ],
    )
    client = unified_llm.Client(providers={"fake": adapter}, default_provider="fake")
    stream = unified_llm.stream(
        model="gpt-5.2",
        prompt="hello",
        client=client,
    )

    first = await stream.__anext__()
    assert first.delta == "partial"

    with caplog.at_level(logging.ERROR, logger="unified_llm.streaming"):
        await stream.close()

    captured = capsys.readouterr()

    assert captured.out == ""
    assert captured.err == ""
    assert adapter.opened_streams[0].closed is True
    assert any(
        record.name == "unified_llm.streaming"
        and "Unexpected error closing stream iterator" in record.message
        for record in caplog.records
    )


@pytest.mark.asyncio
async def test_stream_raises_abort_error_and_closes_provider_stream(
    capsys: pytest.CaptureFixture[str],
) -> None:
    controller = unified_llm.AbortController()
    adapter = _SequencedStreamAdapter(
        "fake",
        [
            [
                unified_llm.StreamEvent(
                    type=unified_llm.StreamEventType.TEXT_DELTA,
                    delta="partial",
                ),
                unified_llm.StreamEvent(
                    type=unified_llm.StreamEventType.TEXT_DELTA,
                    delta="more",
                ),
            ]
        ],
    )
    client = unified_llm.Client(providers={"fake": adapter}, default_provider="fake")
    stream = unified_llm.stream(
        model="gpt-5.2",
        prompt="hello",
        client=client,
        abort_signal=controller.signal,
    )

    first = await stream.__anext__()
    assert first.delta == "partial"
    controller.abort("stop now")

    with pytest.raises(AbortError):
        await stream.__anext__()

    await stream.close()

    captured = capsys.readouterr()

    assert captured.out == ""
    assert captured.err == ""
    assert adapter.opened_streams[0].closed is True


@pytest.mark.asyncio
@pytest.mark.parametrize(
    ("repair_result", "expected_log"),
    [
        (None, "Repair hook for tool weather returned no usable repair"),
        ({"city": 7}, "Invalid repaired arguments for tool weather"),
        (RuntimeError("repair boom"), "Unexpected error repairing tool weather"),
    ],
)
async def test_stream_logs_repair_failures_without_stdout(
    repair_result: object,
    expected_log: str,
    caplog: pytest.LogCaptureFixture,
    capsys: pytest.CaptureFixture[str],
) -> None:
    controller = unified_llm.AbortController()

    async def weather(city: str) -> dict[str, str]:
        return {"tool": "weather", "city": city}

    async def repair_tool_call(
        *,
        tool_call: unified_llm.ToolCall,
        tool: unified_llm.Tool,
        error: object,
        messages: object,
        abort_signal: object,
        tool_call_id: str,
    ) -> object:
        assert tool_call.name == "weather"
        assert tool.name == "weather"
        assert tool_call_id == "call_weather"
        assert [message.role for message in messages] == [unified_llm.Role.USER]
        assert abort_signal is controller.signal
        if isinstance(repair_result, BaseException):
            raise repair_result
        return repair_result

    weather_tool = unified_llm.Tool.active(
        name="weather",
        description="Lookup weather",
        parameters={
            "type": "object",
            "properties": {"city": {"type": "string"}},
            "required": ["city"],
        },
        execute_handler=weather,
    )
    adapter = _SequencedStreamAdapter(
        "fake",
        [
            [
                _text_delta("Need tools"),
                _tool_call_event(
                    unified_llm.StreamEventType.TOOL_CALL_START,
                    "call_weather",
                    "weather",
                    '{"city": 7}',
                ),
                _tool_call_event(
                    unified_llm.StreamEventType.TOOL_CALL_DELTA,
                    "call_weather",
                    "weather",
                    '{"city": 7}',
                ),
                _tool_call_event(
                    unified_llm.StreamEventType.TOOL_CALL_END,
                    "call_weather",
                    "weather",
                    '{"city": 7}',
                ),
                unified_llm.StreamEvent(
                    type=unified_llm.StreamEventType.FINISH,
                    finish_reason=unified_llm.FinishReason(
                        reason=unified_llm.FinishReason.TOOL_CALLS,
                    ),
                    usage=unified_llm.Usage(),
                    response=unified_llm.Response(
                        model="gpt-5.2",
                        provider="fake",
                    ),
                ),
            ],
            [
                _text_delta("finished"),
            ],
        ],
    )
    client = unified_llm.Client(providers={"fake": adapter}, default_provider="fake")

    with caplog.at_level(logging.DEBUG, logger="unified_llm.tools"):
        stream = unified_llm.stream(
            model="gpt-5.2",
            prompt="what should I do?",
            tools=[weather_tool],
            client=client,
            abort_signal=controller.signal,
            repair_tool_call=repair_tool_call,
        )
        events = [event async for event in stream]
        response = await stream.response()
        await stream.close()

    captured = capsys.readouterr()

    assert captured.out == ""
    assert captured.err == ""
    assert [event.type for event in events] == [
        unified_llm.StreamEventType.TEXT_DELTA,
        unified_llm.StreamEventType.TOOL_CALL_START,
        unified_llm.StreamEventType.TOOL_CALL_DELTA,
        unified_llm.StreamEventType.TOOL_CALL_END,
        unified_llm.StreamEventType.FINISH,
        "step_finish",
        unified_llm.StreamEventType.TEXT_DELTA,
        unified_llm.StreamEventType.FINISH,
    ]
    assert events[4].response.text == "Need tools"
    assert events[5].response.text == "Need tools"
    assert response.text == "finished"
    assert stream.partial_response.text == "finished"
    assert len(adapter.stream_requests) == 2
    assert adapter.stream_requests[1].messages[2].content[0].tool_result is not None
    assert adapter.stream_requests[1].messages[2].content[0].tool_result.is_error is True
    assert "Invalid arguments for tool 'weather'" in str(
        adapter.stream_requests[1].messages[2].content[0].tool_result.content
    )
    assert any(
        record.name == "unified_llm.tools" and expected_log in record.message
        for record in caplog.records
    )
