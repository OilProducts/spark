from __future__ import annotations

import base64
import json
import logging

import httpx
import pytest

import unified_llm


def _request_json(request: httpx.Request) -> dict[str, object]:
    return json.loads(request.content.decode("utf-8"))


def _make_complete_transport(
    response_body: dict[str, object],
    *,
    headers: dict[str, str] | None = None,
    status_code: int = 200,
) -> tuple[list[httpx.Request], httpx.MockTransport]:
    captured_requests: list[httpx.Request] = []

    async def handler(request: httpx.Request) -> httpx.Response:
        captured_requests.append(request)
        return httpx.Response(
            status_code,
            headers={"content-type": "application/json", **dict(headers or {})},
            content=json.dumps(response_body).encode("utf-8"),
        )

    return captured_requests, httpx.MockTransport(handler)


def _make_stream_transport(
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


def _sse_chunk(payload: dict[str, object]) -> str:
    return f"data: {json.dumps(payload, separators=(',', ':'), sort_keys=True)}\n\n"


def _unsupported_instruction_part(
    content_kind: unified_llm.ContentKind,
) -> unified_llm.ContentPart:
    if content_kind == unified_llm.ContentKind.IMAGE:
        return unified_llm.ContentPart(
            kind=content_kind,
            image=unified_llm.ImageData(url="https://example.test/image.png"),
        )
    if content_kind == unified_llm.ContentKind.AUDIO:
        return unified_llm.ContentPart(
            kind=content_kind,
            audio=unified_llm.AudioData(url="https://example.test/audio.mp3"),
        )
    if content_kind == unified_llm.ContentKind.DOCUMENT:
        return unified_llm.ContentPart(
            kind=content_kind,
            document=unified_llm.DocumentData(url="https://example.test/doc.txt"),
        )
    if content_kind == unified_llm.ContentKind.TOOL_CALL:
        return unified_llm.ContentPart(
            kind=content_kind,
            tool_call=unified_llm.ToolCallData(
                id="call_123",
                name="lookup_weather",
                arguments={"city": "Paris"},
            ),
        )
    if content_kind == unified_llm.ContentKind.TOOL_RESULT:
        return unified_llm.ContentPart(
            kind=content_kind,
            tool_result=unified_llm.ToolResultData(
                tool_call_id="call_123",
                content="tool output",
                is_error=False,
            ),
        )
    raise AssertionError(f"Unsupported instruction content kind: {content_kind!r}")


def test_openai_compatible_adapter_is_distinct_from_openai_adapter() -> None:
    assert unified_llm.OpenAICompatibleAdapter is not unified_llm.OpenAIAdapter
    assert unified_llm.OpenAICompatibleAdapter.name == "openai_compatible"


@pytest.mark.asyncio
async def test_openai_compatible_adapter_chat_completions_payloads() -> None:
    response_body = {
        "id": "chatcmpl_123",
        "model": "gpt-5.2",
        "choices": [
            {
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": "Hello",
                    "tool_calls": [
                        {
                            "id": "call_123",
                            "type": "function",
                            "function": {
                                "name": "lookup_weather",
                                "arguments": {"city": "Paris"},
                            },
                        }
                    ],
                },
                "finish_reason": "tool_calls",
            }
        ],
        "usage": {
            "prompt_tokens": 12,
            "completion_tokens": 34,
            "total_tokens": 46,
            "output_tokens_details": {"reasoning_tokens": 5},
        },
    }
    captured_requests, transport = _make_complete_transport(
        response_body,
        headers={
            "x-ratelimit-remaining-requests": "7",
            "x-ratelimit-remaining-tokens": "99",
        },
    )
    adapter = unified_llm.OpenAICompatibleAdapter(
        api_key="explicit-key",
        base_url="https://explicit.example/api",
        timeout=12.5,
        default_headers={
            "Authorization": "wrong",
            "X-Custom": "value",
        },
        transport=transport,
    )
    image_bytes = b"image-bytes"
    tool_parameters = {
        "type": "object",
        "properties": {
            "city": {"type": "string"},
        },
        "required": ["city"],
        "additionalProperties": False,
    }
    tool = unified_llm.Tool.passive(
        name="lookup_weather",
        description="Fetch weather for a city",
        parameters=tool_parameters,
    )
    request = unified_llm.Request(
        model="gpt-5.2",
        messages=[
            unified_llm.Message.system("system instructions"),
            unified_llm.Message(
                role=unified_llm.Role.DEVELOPER,
                content=[
                    unified_llm.ContentPart(
                        kind=unified_llm.ContentKind.TEXT,
                        text="developer instructions",
                    )
                ],
            ),
            unified_llm.Message.user(
                [
                    unified_llm.ContentPart(
                        kind=unified_llm.ContentKind.TEXT,
                        text="show me the weather",
                    ),
                    unified_llm.ContentPart(
                        kind=unified_llm.ContentKind.IMAGE,
                        image=unified_llm.ImageData(url="https://example.test/image.png"),
                    ),
                    unified_llm.ContentPart(
                        kind=unified_llm.ContentKind.IMAGE,
                        image=unified_llm.ImageData(data=image_bytes, media_type="image/png"),
                    ),
                ]
            ),
            unified_llm.Message.assistant(
                [
                    unified_llm.ContentPart(
                        kind=unified_llm.ContentKind.TEXT,
                        text="I need a tool call first",
                    ),
                    unified_llm.ContentPart(
                        kind=unified_llm.ContentKind.TOOL_CALL,
                        tool_call=unified_llm.ToolCall(
                            id="call_123",
                            name="lookup_weather",
                            arguments={"city": "Paris"},
                        ),
                    ),
                ]
            ),
            unified_llm.Message.tool_result(
                "call_123",
                {"temperature": 72, "unit": "F"},
                is_error=False,
            ),
        ],
        tools=[tool],
        tool_choice=unified_llm.ToolChoice.named("lookup_weather"),
        provider_options={
            "openai_compatible": {
                "parallel_tool_calls": False,
                "headers": {"X-Request-ID": "req-123"},
            },
            "openai": {"reasoning": {"effort": "high"}},
            "anthropic": {"beta_headers": ["prompt-caching-2024-07-31"]},
        },
    )

    response = await adapter.complete(request)

    assert adapter.api_key == "explicit-key"
    assert adapter.base_url == "https://explicit.example/api/v1"
    assert adapter.timeout == 12.5
    assert adapter.default_headers == {
        "Authorization": "wrong",
        "X-Custom": "value",
    }
    assert len(captured_requests) == 1

    sent_request = captured_requests[0]
    body = _request_json(sent_request)
    expected_image_data_uri = "data:image/png;base64," + base64.b64encode(image_bytes).decode(
        "ascii"
    )

    assert sent_request.method == "POST"
    assert sent_request.url.path == "/api/v1/chat/completions"
    assert sent_request.headers["authorization"] == "Bearer explicit-key"
    assert sent_request.headers["x-custom"] == "value"
    assert sent_request.headers["x-request-id"] == "req-123"
    assert body["model"] == "gpt-5.2"
    assert body["parallel_tool_calls"] is False
    assert "reasoning" not in body
    assert "beta_headers" not in body
    assert body["messages"] == [
        {"role": "system", "content": "system instructions"},
        {"role": "developer", "content": "developer instructions"},
        {
            "role": "user",
            "content": [
                {"type": "text", "text": "show me the weather"},
                {
                    "type": "image_url",
                    "image_url": {"url": "https://example.test/image.png"},
                },
                {
                    "type": "image_url",
                    "image_url": {"url": expected_image_data_uri},
                },
            ],
        },
        {
            "role": "assistant",
            "content": "I need a tool call first",
            "tool_calls": [
                {
                    "id": "call_123",
                    "type": "function",
                    "function": {
                        "name": "lookup_weather",
                        "arguments": '{"city":"Paris"}',
                    },
                }
            ],
        },
        {
            "role": "tool",
            "tool_call_id": "call_123",
            "content": '{"temperature":72,"unit":"F"}',
        },
    ]
    assert body["tools"] == [
        {
            "type": "function",
            "function": {
                "name": "lookup_weather",
                "description": "Fetch weather for a city",
                "parameters": tool_parameters,
            },
        }
    ]
    assert body["tool_choice"] == {
        "type": "function",
        "function": {"name": "lookup_weather"},
    }

    assert response.provider == "openai_compatible"
    assert response.id == "chatcmpl_123"
    assert response.model == "gpt-5.2"
    assert response.text == "Hello"
    assert response.finish_reason.reason == "tool_calls"
    assert response.finish_reason.raw == "tool_calls"
    assert response.tool_calls == [
        unified_llm.ToolCall(
            id="call_123",
            name="lookup_weather",
            arguments={"city": "Paris"},
            raw_arguments='{"city":"Paris"}',
            type="function",
        )
    ]
    assert [part.kind for part in response.message.content] == [
        unified_llm.ContentKind.TEXT,
        unified_llm.ContentKind.TOOL_CALL,
    ]
    assert response.usage.input_tokens == 12
    assert response.usage.output_tokens == 34
    assert response.usage.total_tokens == 46
    assert response.usage.reasoning_tokens is None
    assert response.usage.raw == response_body["usage"]
    assert response.rate_limit is not None
    assert response.rate_limit.requests_remaining == 7
    assert response.rate_limit.tokens_remaining == 99
    assert response.raw == response_body
    assert request.provider_options == {
        "openai_compatible": {
            "parallel_tool_calls": False,
            "headers": {"X-Request-ID": "req-123"},
        },
        "openai": {"reasoning": {"effort": "high"}},
        "anthropic": {"beta_headers": ["prompt-caching-2024-07-31"]},
    }
    assert request.tools == [tool]
    assert request.tool_choice == unified_llm.ToolChoice.named("lookup_weather")


@pytest.mark.asyncio
async def test_openai_compatible_adapter_http_error_metadata() -> None:
    response_body = {
        "error": {
            "message": "slow down",
            "code": "rate_limit",
        }
    }
    captured_requests, transport = _make_complete_transport(
        response_body,
        headers={"Retry-After": "7"},
        status_code=429,
    )
    adapter = unified_llm.OpenAICompatibleAdapter(api_key="error-key", transport=transport)
    request = unified_llm.Request(
        model="gpt-5.2",
        messages=[unified_llm.Message.user("ping")],
    )

    with pytest.raises(unified_llm.RateLimitError) as excinfo:
        await adapter.complete(request)

    error = excinfo.value
    assert len(captured_requests) == 1
    assert error.message == "slow down"
    assert error.provider == "openai_compatible"
    assert error.status_code == 429
    assert error.error_code == "rate_limit"
    assert error.retry_after == 7.0
    assert error.raw == response_body


@pytest.mark.asyncio
async def test_openai_compatible_adapter_converts_transport_errors_to_sdk_errors() -> None:
    captured_requests: list[httpx.Request] = []

    async def handler(request: httpx.Request) -> httpx.Response:
        captured_requests.append(request)
        raise httpx.ConnectError("boom", request=request)

    adapter = unified_llm.OpenAICompatibleAdapter(
        api_key="transport-key",
        transport=httpx.MockTransport(handler),
    )
    request = unified_llm.Request(
        model="gpt-5.2",
        messages=[unified_llm.Message.user("ping")],
    )

    with pytest.raises(unified_llm.NetworkError) as excinfo:
        await adapter.complete(request)

    error = excinfo.value
    assert len(captured_requests) == 1
    assert error.message == "boom"
    assert getattr(error, "provider", None) == "openai_compatible"
    assert getattr(error, "cause", None) is not None
    assert type(error.cause) is httpx.ConnectError


@pytest.mark.asyncio
async def test_openai_compatible_adapter_skips_builtin_tools(
    caplog: pytest.LogCaptureFixture,
    capsys: pytest.CaptureFixture[str],
) -> None:
    response_body = {
        "id": "chatcmpl_builtin",
        "model": "gpt-5.2",
        "choices": [
            {
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": "ok",
                },
                "finish_reason": "stop",
            }
        ],
    }
    captured_requests, transport = _make_complete_transport(
        response_body,
        headers={"x-ratelimit-remaining-requests": "1"},
    )
    adapter = unified_llm.OpenAICompatibleAdapter(
        api_key="builtin-key",
        default_headers={"X-Custom": "value"},
        transport=transport,
    )
    request = unified_llm.Request(
        model="gpt-5.2",
        messages=[unified_llm.Message.user("ping")],
        provider_options={
            "openai_compatible": {
                "tools": [{"type": "web_search"}],
                "headers": {"X-Trace": "trace-1"},
            }
        },
    )

    with caplog.at_level(logging.WARNING, logger="unified_llm.provider_utils.openai_compatible"):
        response = await adapter.complete(request)

    captured = capsys.readouterr()

    assert len(captured_requests) == 1
    body = _request_json(captured_requests[0])
    assert "tools" not in body
    assert body["messages"] == [{"role": "user", "content": "ping"}]
    assert captured_requests[0].headers["authorization"] == "Bearer builtin-key"
    assert captured_requests[0].headers["x-custom"] == "value"
    assert captured_requests[0].headers["x-trace"] == "trace-1"
    assert response.text == "ok"
    assert captured.out == ""
    assert captured.err == ""
    assert any(
        record.name == "unified_llm.provider_utils.openai_compatible"
        and "ignores unsupported Responses tool type web_search" in record.message
        for record in caplog.records
    )


@pytest.mark.asyncio
async def test_openai_compatible_adapter_warns_on_responses_only_request_features(
    caplog: pytest.LogCaptureFixture,
    capsys: pytest.CaptureFixture[str],
) -> None:
    response_body = {
        "id": "chatcmpl_features",
        "model": "gpt-5.2",
        "choices": [
            {
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": "ok",
                },
                "finish_reason": "stop",
            }
        ],
    }
    captured_requests, transport = _make_complete_transport(response_body)
    adapter = unified_llm.OpenAICompatibleAdapter(
        api_key="features-key",
        transport=transport,
    )
    request = unified_llm.Request(
        model="gpt-5.2",
        messages=[unified_llm.Message.user("ping")],
        response_format=unified_llm.ResponseFormat(
            type="json_schema",
            json_schema={
                "type": "object",
                "properties": {
                    "answer": {"type": "string"},
                },
                "required": ["answer"],
            },
            strict=True,
        ),
        reasoning_effort="high",
    )

    with caplog.at_level(logging.WARNING, logger="unified_llm.provider_utils.openai_compatible"):
        response = await adapter.complete(request)

    captured = capsys.readouterr()

    assert len(captured_requests) == 1
    body = _request_json(captured_requests[0])
    assert body["messages"] == [{"role": "user", "content": "ping"}]
    assert "response_format" not in body
    assert "reasoning" not in body
    assert response.text == "ok"
    assert captured.out == ""
    assert captured.err == ""
    assert any("ignores request.response_format" in record.message for record in caplog.records)
    assert any("ignores reasoning_effort" in record.message for record in caplog.records)


@pytest.mark.asyncio
async def test_openai_compatible_adapter_preserves_raw_responses_stream_events_with_warning(
    caplog: pytest.LogCaptureFixture,
    capsys: pytest.CaptureFixture[str],
) -> None:
    payload = (
        "event: response.created\n"
        'data: {"type":"response.created","response":{"id":"chatcmpl_stream","model":"gpt-5.2"}}\n'
        "\n"
    )
    captured_requests, transport = _make_stream_transport(payload)
    adapter = unified_llm.OpenAICompatibleAdapter(api_key="stream-key", transport=transport)
    request = unified_llm.Request(
        model="gpt-5.2",
        messages=[unified_llm.Message.user("hello")],
    )

    with caplog.at_level(logging.WARNING, logger="unified_llm.provider_utils.openai_compatible"):
        events = [event async for event in adapter.stream(request)]

    captured = capsys.readouterr()

    assert len(captured_requests) == 1
    assert [event.type for event in events] == [
        unified_llm.StreamEventType.PROVIDER_EVENT,
        unified_llm.StreamEventType.FINISH,
    ]
    assert events[0].raw == {
        "type": "response.created",
        "response": {
            "id": "chatcmpl_stream",
            "model": "gpt-5.2",
        },
    }
    assert events[-1].response is not None
    assert events[-1].finish_reason.reason == "stop"
    assert captured.out == ""
    assert captured.err == ""
    assert any(
        "received unsupported Responses stream event response.created" in record.message
        for record in caplog.records
    )


@pytest.mark.asyncio
async def test_openai_compatible_adapter_streams_text_chunks_and_terminates_on_done() -> None:
    payload = (
        _sse_chunk(
            {
                "id": "chatcmpl_stream",
                "model": "gpt-5.2",
                "choices": [
                    {
                        "index": 0,
                        "delta": {"role": "assistant"},
                    }
                ],
            }
        )
        + _sse_chunk(
            {
                "id": "chatcmpl_stream",
                "model": "gpt-5.2",
                "choices": [
                    {
                        "index": 0,
                        "delta": {"content": "Hel"},
                    }
                ],
            }
        )
        + _sse_chunk(
            {
                "id": "chatcmpl_stream",
                "model": "gpt-5.2",
                "choices": [
                    {
                        "index": 0,
                        "delta": {"content": "lo"},
                        "finish_reason": "stop",
                    }
                ],
                "usage": {
                    "prompt_tokens": 2,
                    "completion_tokens": 3,
                    "total_tokens": 5,
                },
            }
        )
        + "data: [DONE]\n\n"
    )
    captured_requests, transport = _make_stream_transport(
        payload,
        headers={
            "x-ratelimit-remaining-requests": "7",
            "x-ratelimit-remaining-tokens": "99",
        },
    )
    adapter = unified_llm.OpenAICompatibleAdapter(
        api_key="stream-key",
        base_url="https://stream.example",
        transport=transport,
    )
    request = unified_llm.Request(
        model="gpt-5.2",
        messages=[unified_llm.Message.user("hello")],
    )

    events = [event async for event in adapter.stream(request)]
    accumulator = unified_llm.StreamAccumulator.from_events(events)
    response = accumulator.response

    assert len(captured_requests) == 1
    sent_request = captured_requests[0]
    body = _request_json(sent_request)

    assert sent_request.method == "POST"
    assert sent_request.url.path == "/v1/chat/completions"
    assert sent_request.headers["authorization"] == "Bearer stream-key"
    assert body["stream"] is True
    assert body["messages"] == [{"role": "user", "content": "hello"}]
    assert [event.type for event in events] == [
        unified_llm.StreamEventType.STREAM_START,
        unified_llm.StreamEventType.TEXT_START,
        unified_llm.StreamEventType.TEXT_DELTA,
        unified_llm.StreamEventType.TEXT_DELTA,
        unified_llm.StreamEventType.TEXT_END,
        unified_llm.StreamEventType.FINISH,
    ]
    assert events[1].delta is None
    assert [
        event.delta for event in events if event.type == unified_llm.StreamEventType.TEXT_DELTA
    ] == [
        "Hel",
        "lo",
    ]
    assert events[4].delta == "Hello"
    assert response.text == "Hello"
    assert response.finish_reason.reason == "stop"
    assert response.usage.input_tokens == 2
    assert response.usage.output_tokens == 3
    assert response.usage.total_tokens == 5
    assert response.usage.reasoning_tokens is None
    assert response.rate_limit is not None
    assert response.rate_limit.requests_remaining == 7
    assert response.rate_limit.tokens_remaining == 99


@pytest.mark.asyncio
async def test_openai_compatible_adapter_streams_tool_call_deltas_and_terminates_on_done() -> None:
    payload = (
        _sse_chunk(
            {
                "id": "chatcmpl_tools",
                "model": "gpt-5.2",
                "choices": [
                    {
                        "index": 0,
                        "delta": {"role": "assistant"},
                    }
                ],
            }
        )
        + _sse_chunk(
            {
                "id": "chatcmpl_tools",
                "model": "gpt-5.2",
                "choices": [
                    {
                        "index": 0,
                        "delta": {
                            "tool_calls": [
                                {
                                    "index": 0,
                                    "id": "call_123",
                                    "type": "function",
                                    "function": {
                                        "name": "lookup_weather",
                                        "arguments": '{"city":',
                                    },
                                }
                            ]
                        },
                    }
                ],
            }
        )
        + _sse_chunk(
            {
                "id": "chatcmpl_tools",
                "model": "gpt-5.2",
                "choices": [
                    {
                        "index": 0,
                        "delta": {
                            "tool_calls": [
                                {
                                    "index": 0,
                                    "function": {
                                        "arguments": '"Paris"}',
                                    },
                                }
                            ]
                        },
                        "finish_reason": "tool_calls",
                    }
                ],
                "usage": {
                    "prompt_tokens": 4,
                    "completion_tokens": 5,
                    "total_tokens": 9,
                },
            }
        )
        + "data: [DONE]\n\n"
    )
    captured_requests, transport = _make_stream_transport(
        payload,
        headers={
            "x-ratelimit-remaining-requests": "3",
            "x-ratelimit-remaining-tokens": "88",
        },
    )
    adapter = unified_llm.OpenAICompatibleAdapter(api_key="stream-key", transport=transport)
    request = unified_llm.Request(
        model="gpt-5.2",
        messages=[unified_llm.Message.user("call the weather tool")],
    )

    events = [event async for event in adapter.stream(request)]
    accumulator = unified_llm.StreamAccumulator.from_events(events)
    response = accumulator.response

    assert len(captured_requests) == 1
    body = _request_json(captured_requests[0])

    assert body["stream"] is True
    assert [event.type for event in events] == [
        unified_llm.StreamEventType.STREAM_START,
        unified_llm.StreamEventType.TOOL_CALL_START,
        unified_llm.StreamEventType.TOOL_CALL_DELTA,
        unified_llm.StreamEventType.TOOL_CALL_DELTA,
        unified_llm.StreamEventType.TOOL_CALL_END,
        unified_llm.StreamEventType.FINISH,
    ]
    assert events[1].tool_call is not None
    assert events[1].tool_call.id == "call_123"
    assert events[1].tool_call.name == "lookup_weather"
    assert events[2].tool_call is not None
    assert events[2].tool_call.raw_arguments == '{"city":'
    assert events[3].tool_call is not None
    assert events[3].tool_call.raw_arguments == '"Paris"}'
    assert events[4].tool_call is not None
    assert events[4].tool_call.id == "call_123"
    assert events[4].tool_call.name == "lookup_weather"
    assert events[4].tool_call.arguments == {"city": "Paris"}
    assert events[4].tool_call.raw_arguments == '{"city":"Paris"}'
    assert events[-1].finish_reason.reason == "tool_calls"
    assert events[-1].usage is not None
    assert events[-1].usage.total_tokens == 9
    assert response.finish_reason.reason == "tool_calls"
    assert response.tool_calls == [
        unified_llm.ToolCall(
            id="call_123",
            name="lookup_weather",
            arguments={"city": "Paris"},
            raw_arguments='{"city":"Paris"}',
            type="function",
        )
    ]
    assert response.raw == {
        "id": "chatcmpl_tools",
        "model": "gpt-5.2",
        "choices": [
            {
                "index": 0,
                "delta": {
                    "tool_calls": [
                        {
                            "index": 0,
                            "function": {
                                "arguments": '"Paris"}',
                            },
                        }
                    ]
                },
                "finish_reason": "tool_calls",
            }
        ],
        "usage": {
            "prompt_tokens": 4,
            "completion_tokens": 5,
            "total_tokens": 9,
        },
    }
    assert response.usage.input_tokens == 4
    assert response.usage.output_tokens == 5
    assert response.usage.total_tokens == 9
    assert response.usage.reasoning_tokens is None
    assert response.rate_limit is not None
    assert response.rate_limit.requests_remaining == 3
    assert response.rate_limit.tokens_remaining == 88


def test_openai_compatible_adapter_supports_the_standard_tool_choice_modes() -> None:
    adapter = unified_llm.OpenAICompatibleAdapter(api_key="choice-key", client=object())

    assert adapter.supports_tool_choice("auto") is True
    assert adapter.supports_tool_choice("none") is True
    assert adapter.supports_tool_choice("required") is True
    assert adapter.supports_tool_choice("named") is True
    assert adapter.supports_tool_choice("unsupported") is False


@pytest.mark.asyncio
async def test_openai_compatible_adapter_close_respects_client_ownership_and_logs_failures(
    caplog: pytest.LogCaptureFixture,
    capsys: pytest.CaptureFixture[str],
) -> None:
    class _CloseRecorder:
        def __init__(self, error: BaseException | None = None) -> None:
            self.closed = False
            self.error = error

        async def aclose(self) -> None:
            self.closed = True
            if self.error is not None:
                raise self.error

    borrowed = _CloseRecorder()
    borrowed_adapter = unified_llm.OpenAICompatibleAdapter(
        api_key="close-key",
        client=borrowed,
    )
    await borrowed_adapter.close()
    assert borrowed.closed is False

    owned = _CloseRecorder()
    owned_adapter = unified_llm.OpenAICompatibleAdapter(
        api_key="close-key",
        client=owned,
        owns_client=True,
    )
    await owned_adapter.close()
    assert owned.closed is True

    failing = _CloseRecorder(error=RuntimeError("boom"))
    failing_adapter = unified_llm.OpenAICompatibleAdapter(
        api_key="close-key",
        client=failing,
        owns_client=True,
    )

    with caplog.at_level(logging.ERROR, logger="unified_llm.adapters.openai_compatible"):
        await failing_adapter.close()

    captured = capsys.readouterr()
    assert captured.out == ""
    assert captured.err == ""
    assert failing.closed is True
    assert any(
        record.name == "unified_llm.adapters.openai_compatible"
        and "Unexpected error closing OpenAI-compatible HTTP client" in record.message
        for record in caplog.records
    )
