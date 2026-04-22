from __future__ import annotations

import asyncio
import base64
import json
import logging
from collections.abc import AsyncIterator

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


class _BlockingStream(httpx.AsyncByteStream):
    def __init__(self, first_chunk: bytes, second_chunk: bytes) -> None:
        self._first_chunk = first_chunk
        self._second_chunk = second_chunk
        self._release_second_chunk = asyncio.Event()
        self.second_chunk_requested = asyncio.Event()
        self.closed = False

    async def __aiter__(self) -> AsyncIterator[bytes]:
        yield self._first_chunk
        self.second_chunk_requested.set()
        await self._release_second_chunk.wait()
        if self.closed:
            return
        yield self._second_chunk

    async def aclose(self) -> None:
        self.closed = True
        self._release_second_chunk.set()


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


@pytest.mark.asyncio
async def test_openai_adapter_uses_explicit_configuration_and_the_native_responses_endpoint() -> (
    None
):
    response_body = {
        "id": "resp_123",
        "model": "gpt-5.2",
        "status": "completed",
        "output": [
            {
                "type": "function_call",
                "id": "call_123",
                "name": "lookup_weather",
                "arguments": {"city": "Paris"},
            },
            {
                "type": "output_text",
                "text": "Hello",
            },
            {
                "type": "function_call_output",
                "call_id": "call_123",
                "output": {"temperature": 72, "unit": "F"},
            },
        ],
        "usage": {
            "input_tokens": 12,
            "output_tokens": 34,
            "output_tokens_details": {"reasoning_tokens": 5},
            "input_tokens_details": {"cached_tokens": 4},
        },
    }
    captured_requests, transport = _make_complete_transport(
        response_body,
        headers={
            "x-ratelimit-remaining-requests": "7",
            "x-ratelimit-remaining-tokens": "99",
        },
    )
    adapter = unified_llm.OpenAIAdapter(
        api_key="explicit-key",
        base_url="https://explicit.example/api",
        organization="org-123",
        project="project-456",
        timeout=12.5,
        default_headers={
            "Authorization": "wrong",
            "X-Custom": "value",
        },
        transport=transport,
    )
    request = unified_llm.Request(
        model="gpt-5.2",
        messages=[unified_llm.Message.user("hello")],
        reasoning_effort="high",
    )

    response = await adapter.complete(request)

    assert adapter.api_key == "explicit-key"
    assert adapter.base_url == "https://explicit.example/api/v1"
    assert adapter.organization == "org-123"
    assert adapter.project == "project-456"
    assert adapter.timeout == 12.5
    assert adapter.default_headers == {
        "Authorization": "wrong",
        "X-Custom": "value",
    }
    assert len(captured_requests) == 1

    sent_request = captured_requests[0]
    body = _request_json(sent_request)

    assert sent_request.method == "POST"
    assert sent_request.url.path == "/api/v1/responses"
    assert sent_request.headers["authorization"] == "Bearer explicit-key"
    assert sent_request.headers["openai-organization"] == "org-123"
    assert sent_request.headers["openai-project"] == "project-456"
    assert sent_request.headers["x-custom"] == "value"
    assert "messages" not in body
    assert body["model"] == "gpt-5.2"
    assert "stream" not in body
    assert body["reasoning"] == {"effort": "high"}
    assert body["input"] == [
        {
            "type": "message",
            "role": "user",
            "content": [{"type": "input_text", "text": "hello"}],
        }
    ]

    assert response.provider == "openai"
    assert response.id == "resp_123"
    assert response.model == "gpt-5.2"
    assert response.text == "Hello"
    assert response.finish_reason.reason == "tool_calls"
    assert response.finish_reason.raw == "completed"
    assert response.tool_calls == [
        unified_llm.ToolCall(
            id="call_123",
            name="lookup_weather",
            arguments={"city": "Paris"},
            raw_arguments='{"city":"Paris"}',
            type="function",
        )
    ]
    assert response.message.content[0].kind == unified_llm.ContentKind.TOOL_CALL
    assert response.message.content[1].kind == unified_llm.ContentKind.TEXT
    assert response.message.content[2].kind == unified_llm.ContentKind.TOOL_RESULT
    assert response.message.content[2].tool_result is not None
    assert response.message.content[2].tool_result.content == {
        "temperature": 72,
        "unit": "F",
    }
    assert response.usage.input_tokens == 12
    assert response.usage.output_tokens == 34
    assert response.usage.reasoning_tokens == 5
    assert response.usage.cache_read_tokens == 4
    assert response.rate_limit is not None
    assert response.rate_limit.requests_remaining == 7
    assert response.rate_limit.tokens_remaining == 99
    assert response.raw == response_body


@pytest.mark.asyncio
async def test_openai_adapter_normalizes_completed_function_call_responses_to_tool_calls(
) -> None:
    response_body = {
        "id": "resp_tool_calls",
        "model": "gpt-5.2",
        "status": "completed",
        "output": [
            {
                "type": "function_call",
                "id": "call_123",
                "name": "lookup_weather",
                "arguments": {"city": "Paris"},
            },
            {
                "type": "output_text",
                "text": "use the tool",
            },
        ],
    }
    captured_requests, transport = _make_complete_transport(response_body)
    adapter = unified_llm.OpenAIAdapter(api_key="tool-key", transport=transport)
    request = unified_llm.Request(
        model="gpt-5.2",
        messages=[unified_llm.Message.user("ping")],
    )

    response = await adapter.complete(request)

    assert len(captured_requests) == 1
    assert response.tool_calls == [
        unified_llm.ToolCall(
            id="call_123",
            name="lookup_weather",
            arguments={"city": "Paris"},
            raw_arguments='{"city":"Paris"}',
            type="function",
        )
    ]
    assert response.finish_reason.reason == "tool_calls"
    assert response.finish_reason.raw == "completed"
    assert response.raw == response_body


@pytest.mark.asyncio
async def test_openai_adapter_uses_environment_fallbacks_for_missing_config(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setenv("OPENAI_API_KEY", "env-key")
    monkeypatch.setenv("OPENAI_BASE_URL", "https://env.example/custom")
    monkeypatch.setenv("OPENAI_ORG_ID", "env-org")
    monkeypatch.setenv("OPENAI_PROJECT_ID", "env-project")

    response_body = {
        "id": "resp_456",
        "model": "gpt-5.2",
        "status": "completed",
        "output_text": "ok",
    }
    captured_requests, transport = _make_complete_transport(response_body)
    adapter = unified_llm.OpenAIAdapter(transport=transport)

    response = await adapter.complete(
        unified_llm.Request(
            model="gpt-5.2",
            messages=[unified_llm.Message.user("ping")],
        )
    )

    assert len(captured_requests) == 1
    sent_request = captured_requests[0]
    body = _request_json(sent_request)

    assert sent_request.url.path == "/custom/v1/responses"
    assert sent_request.headers["authorization"] == "Bearer env-key"
    assert sent_request.headers["openai-organization"] == "env-org"
    assert sent_request.headers["openai-project"] == "env-project"
    assert body["input"][0]["content"][0]["text"] == "ping"
    assert response.text == "ok"


@pytest.mark.asyncio
@pytest.mark.parametrize(
    ("status", "expected_reason", "expected_raw"),
    [
        ("tool_calls", "tool_calls", "tool_calls"),
        ("incomplete", "length", "incomplete"),
        ("content_filter", "content_filter", "content_filter"),
        ("vendor.custom", "other", "vendor.custom"),
    ],
)
async def test_openai_adapter_normalizes_finish_reason_values(
    status: str,
    expected_reason: str,
    expected_raw: str,
) -> None:
    response_body = {
        "id": "resp_reason",
        "model": "gpt-5.2",
        "status": status,
        "output_text": "ok",
    }
    captured_requests, transport = _make_complete_transport(response_body)
    adapter = unified_llm.OpenAIAdapter(api_key="reason-key", transport=transport)
    request = unified_llm.Request(
        model="gpt-5.2",
        messages=[unified_llm.Message.user("ping")],
    )

    response = await adapter.complete(request)

    assert len(captured_requests) == 1
    assert response.finish_reason.reason == expected_reason
    assert response.finish_reason.raw == expected_raw
    assert response.raw == response_body
    assert response.text == "ok"


@pytest.mark.asyncio
@pytest.mark.parametrize(
    ("incomplete_reason", "expected_reason"),
    [
        ("content_filter", "content_filter"),
        ("max_output_tokens", "length"),
    ],
)
async def test_openai_adapter_uses_incomplete_details_reason_for_incomplete_responses(
    incomplete_reason: str,
    expected_reason: str,
) -> None:
    response_body = {
        "id": "resp_incomplete",
        "model": "gpt-5.2",
        "status": "incomplete",
        "incomplete_details": {
            "reason": incomplete_reason,
        },
        "output_text": "partial",
    }
    captured_requests, transport = _make_complete_transport(response_body)
    adapter = unified_llm.OpenAIAdapter(api_key="reason-key", transport=transport)
    request = unified_llm.Request(
        model="gpt-5.2",
        messages=[unified_llm.Message.user("ping")],
    )

    response = await adapter.complete(request)

    assert len(captured_requests) == 1
    assert response.finish_reason.reason == expected_reason
    assert response.finish_reason.raw == "incomplete"
    assert response.raw == response_body
    assert response.text == "partial"


@pytest.mark.asyncio
async def test_openai_adapter_preserves_retry_after_and_error_metadata_from_http_errors(
) -> None:
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
    adapter = unified_llm.OpenAIAdapter(api_key="error-key", transport=transport)
    request = unified_llm.Request(
        model="gpt-5.2",
        messages=[unified_llm.Message.user("ping")],
    )

    with pytest.raises(unified_llm.RateLimitError) as excinfo:
        await adapter.complete(request)

    error = excinfo.value
    assert len(captured_requests) == 1
    assert error.message == "slow down"
    assert error.provider == "openai"
    assert error.status_code == 429
    assert error.error_code == "rate_limit"
    assert error.retry_after == 7.0
    assert error.raw == response_body


@pytest.mark.asyncio
@pytest.mark.parametrize(
    ("exception_type", "message", "expected_type", "expected_retryable"),
    [
        (
            httpx.ConnectError,
            "boom",
            unified_llm.NetworkError,
            True,
        ),
        (
            httpx.ReadTimeout,
            "timed out",
            unified_llm.RequestTimeoutError,
            False,
        ),
    ],
)
async def test_openai_adapter_converts_complete_transport_errors_to_sdk_errors(
    exception_type: type[httpx.HTTPError],
    message: str,
    expected_type: type[Exception],
    expected_retryable: bool,
) -> None:
    captured_requests: list[httpx.Request] = []

    async def handler(request: httpx.Request) -> httpx.Response:
        captured_requests.append(request)
        raise exception_type(message, request=request)

    adapter = unified_llm.OpenAIAdapter(
        api_key="transport-key",
        transport=httpx.MockTransport(handler),
    )
    request = unified_llm.Request(
        model="gpt-5.2",
        messages=[unified_llm.Message.user("ping")],
    )

    with pytest.raises(expected_type) as excinfo:
        await adapter.complete(request)

    error = excinfo.value
    assert len(captured_requests) == 1
    assert error.message == message
    assert getattr(error, "provider", None) == "openai"
    assert getattr(error, "cause", None) is not None
    assert type(error.cause) is exception_type
    assert error.retryable is expected_retryable


@pytest.mark.asyncio
async def test_openai_adapter_converts_stream_transport_errors_to_sdk_errors() -> None:
    captured_requests: list[httpx.Request] = []

    async def handler(request: httpx.Request) -> httpx.Response:
        captured_requests.append(request)
        raise httpx.ConnectError("boom", request=request)

    adapter = unified_llm.OpenAIAdapter(
        api_key="stream-key",
        transport=httpx.MockTransport(handler),
    )
    request = unified_llm.Request(
        model="gpt-5.2",
        messages=[unified_llm.Message.user("ping")],
    )

    with pytest.raises(unified_llm.NetworkError) as excinfo:
        async for _ in adapter.stream(request):
            pass

    error = excinfo.value
    assert len(captured_requests) == 1
    assert error.message == "boom"
    assert error.provider == "openai"
    assert error.cause is not None
    assert type(error.cause) is httpx.ConnectError


@pytest.mark.asyncio
async def test_openai_adapter_logs_and_converts_malformed_response_payloads(
    caplog: pytest.LogCaptureFixture,
    capsys: pytest.CaptureFixture[str],
) -> None:
    response_body = {
        "id": "resp_invalid",
        "model": "gpt-5.2",
        "status": "completed",
        "output_text": "ok",
        "usage": [],
    }
    captured_requests, transport = _make_complete_transport(response_body)
    adapter = unified_llm.OpenAIAdapter(api_key="invalid-key", transport=transport)
    request = unified_llm.Request(
        model="gpt-5.2",
        messages=[unified_llm.Message.user("ping")],
    )

    with caplog.at_level(logging.ERROR, logger="unified_llm.provider_utils.openai"):
        with pytest.raises(unified_llm.ProviderError) as excinfo:
            await adapter.complete(request)

    error = excinfo.value
    captured = capsys.readouterr()

    assert len(captured_requests) == 1
    assert error.message == "failed to normalize OpenAI response"
    assert error.provider == "openai"
    assert error.retryable is False
    assert isinstance(error.cause, TypeError)
    assert error.raw == response_body
    assert captured.out == ""
    assert captured.err == ""
    assert any(record.name == "unified_llm.provider_utils.openai" for record in caplog.records)


@pytest.mark.asyncio
async def test_openai_adapter_translates_request_response_format_into_native_payload() -> None:
    response_body = {
        "id": "resp_schema",
        "model": "gpt-5.2",
        "status": "completed",
        "output_text": "{}",
    }
    captured_requests, transport = _make_complete_transport(response_body)
    adapter = unified_llm.OpenAIAdapter(api_key="schema-key", transport=transport)
    schema = {
        "type": "object",
        "properties": {
            "answer": {"type": "string"},
        },
        "required": ["answer"],
        "additionalProperties": False,
    }
    request = unified_llm.Request(
        model="gpt-5.2",
        messages=[unified_llm.Message.user("return json")],
        response_format=unified_llm.ResponseFormat(
            type="json_schema",
            json_schema=schema,
            strict=True,
        ),
    )

    await adapter.complete(request)

    assert len(captured_requests) == 1
    body = _request_json(captured_requests[0])

    assert body["response_format"] == {
        "type": "json_schema",
        "json_schema": schema,
        "strict": True,
    }
    assert request.response_format == unified_llm.ResponseFormat(
        type="json_schema",
        json_schema=schema,
        strict=True,
    )


@pytest.mark.asyncio
async def test_openai_adapter_prefers_request_response_format_over_provider_options(
) -> None:
    response_body = {
        "id": "resp_schema",
        "model": "gpt-5.2",
        "status": "completed",
        "output_text": "{}",
    }
    captured_requests, transport = _make_complete_transport(response_body)
    adapter = unified_llm.OpenAIAdapter(api_key="schema-key", transport=transport)
    request_schema = {
        "type": "object",
        "properties": {
            "answer": {"type": "string"},
        },
        "required": ["answer"],
        "additionalProperties": False,
    }
    provider_schema = {
        "type": "object",
        "properties": {
            "answer": {"type": "integer"},
        },
        "required": ["answer"],
        "additionalProperties": True,
    }
    request = unified_llm.Request(
        model="gpt-5.2",
        messages=[unified_llm.Message.user("return json")],
        response_format=unified_llm.ResponseFormat(
            type="json_schema",
            json_schema=request_schema,
            strict=False,
        ),
        provider_options={
            "openai": {
                "response_format": {
                    "type": "json_schema",
                    "json_schema": provider_schema,
                    "strict": True,
                }
            }
        },
    )

    await adapter.complete(request)

    assert len(captured_requests) == 1
    body = _request_json(captured_requests[0])

    assert body["response_format"] == {
        "type": "json_schema",
        "json_schema": request_schema,
        "strict": False,
    }


@pytest.mark.asyncio
async def test_openai_adapter_translates_complex_request_body_without_mutating_request(
) -> None:
    response_body = {
        "id": "resp_complex",
        "model": "gpt-5.2",
        "status": "completed",
        "output_text": "done",
    }
    captured_requests, transport = _make_complete_transport(response_body)
    adapter = unified_llm.OpenAIAdapter(api_key="body-key", transport=transport)
    image_bytes = b"image-bytes"
    schema = {
        "type": "object",
        "properties": {
            "city": {"type": "string"},
        },
        "required": ["city"],
        "additionalProperties": False,
    }
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
    provider_options = {
        "openai": {
            "parallel_tool_calls": False,
            "tools": [
                {
                    "type": "web_search",
                }
            ],
            "response_format": {
                "type": "json_schema",
                "json_schema": schema,
                "strict": True,
            },
            "structured_output": {
                "provider": "openai",
                "strategy": "json_schema",
                "schema": schema,
                "strict": True,
            },
        },
        "anthropic": {"anthropic_only": True},
        "gemini": {"gemini_only": True},
        "openai_compatible": {"compat_only": True},
    }
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
        reasoning_effort="high",
        tools=[tool],
        tool_choice=unified_llm.ToolChoice.named("lookup_weather"),
        provider_options=provider_options,
    )

    await adapter.complete(request)

    assert len(captured_requests) == 1
    body = _request_json(captured_requests[0])
    expected_image_data_uri = (
        "data:image/png;base64," + base64.b64encode(image_bytes).decode("ascii")
    )

    assert body["instructions"] == "system instructions\n\ndeveloper instructions"
    assert body["reasoning"] == {"effort": "high"}
    assert body["tools"] == [
        {
            "type": "function",
            "name": "lookup_weather",
            "strict": True,
            "description": "Fetch weather for a city",
            "parameters": tool_parameters,
        },
        {
            "type": "web_search",
        },
    ]
    assert body["tool_choice"] == {"type": "function", "name": "lookup_weather"}
    assert body["response_format"] == {
        "type": "json_schema",
        "json_schema": schema,
        "strict": True,
    }
    assert body["parallel_tool_calls"] is False
    assert "structured_output" not in body
    assert "anthropic_only" not in body
    assert "gemini_only" not in body
    assert "compat_only" not in body
    assert body["input"] == [
        {
            "type": "message",
            "role": "user",
            "content": [
                {"type": "input_text", "text": "show me the weather"},
                {"type": "input_image", "image_url": "https://example.test/image.png"},
                {"type": "input_image", "image_url": expected_image_data_uri},
            ],
        },
        {
            "type": "message",
            "role": "assistant",
            "content": [{"type": "output_text", "text": "I need a tool call first"}],
        },
        {
            "type": "function_call",
            "id": "call_123",
            "name": "lookup_weather",
            "arguments": "{\"city\":\"Paris\"}",
        },
        {
            "type": "function_call_output",
            "call_id": "call_123",
            "output": "{\"temperature\":72,\"unit\":\"F\"}",
        },
    ]

    assert request.provider_options == provider_options
    assert request.tools == [tool]
    assert request.tool_choice == unified_llm.ToolChoice.named("lookup_weather")


@pytest.mark.asyncio
@pytest.mark.parametrize(
    ("arguments", "expected_arguments"),
    [
        ({"city": "Paris"}, '{"city":"Paris"}'),
        ('{"city": "Paris"}', '{"city": "Paris"}'),
    ],
)
async def test_openai_adapter_translates_tool_call_data_into_function_call_input_item(
    arguments: object,
    expected_arguments: str,
) -> None:
    response_body = {
        "id": "resp_tool_call_data",
        "model": "gpt-5.2",
        "status": "completed",
        "output_text": "done",
    }
    captured_requests, transport = _make_complete_transport(response_body)
    adapter = unified_llm.OpenAIAdapter(api_key="body-key", transport=transport)
    tool_call = unified_llm.ToolCallData(
        id="call_123",
        name="lookup_weather",
        arguments=arguments,
    )
    request = unified_llm.Request(
        model="gpt-5.2",
        messages=[
            unified_llm.Message.assistant(
                [
                    unified_llm.ContentPart(
                        kind=unified_llm.ContentKind.TEXT,
                        text="I need a tool call first",
                    ),
                    unified_llm.ContentPart(
                        kind=unified_llm.ContentKind.TOOL_CALL,
                        tool_call=tool_call,
                    ),
                ]
            )
        ],
    )

    await adapter.complete(request)

    assert len(captured_requests) == 1
    body = _request_json(captured_requests[0])

    assert body["input"] == [
        {
            "type": "message",
            "role": "assistant",
            "content": [{"type": "output_text", "text": "I need a tool call first"}],
        },
        {
            "type": "function_call",
            "id": "call_123",
            "name": "lookup_weather",
            "arguments": expected_arguments,
        },
    ]
    assert tool_call.arguments == arguments


@pytest.mark.asyncio
async def test_openai_adapter_rejects_malformed_tool_call_payloads() -> None:
    response_body = {
        "id": "resp_malformed_tool_call",
        "model": "gpt-5.2",
        "status": "completed",
        "output_text": "done",
    }
    captured_requests, transport = _make_complete_transport(response_body)
    adapter = unified_llm.OpenAIAdapter(api_key="body-key", transport=transport)
    tool_call = unified_llm.ToolCallData(
        id="call_123",
        name="lookup_weather",
        arguments={"city": "Paris"},
    )
    tool_call.arguments = None  # type: ignore[assignment]
    request = unified_llm.Request(
        model="gpt-5.2",
        messages=[
            unified_llm.Message.assistant(
                [
                    unified_llm.ContentPart(
                        kind=unified_llm.ContentKind.TOOL_CALL,
                        tool_call=tool_call,
                    )
                ]
            )
        ],
    )

    with pytest.raises(unified_llm.InvalidRequestError, match="arguments"):
        await adapter.complete(request)

    assert captured_requests == []


@pytest.mark.asyncio
@pytest.mark.parametrize("role", [unified_llm.Role.SYSTEM, unified_llm.Role.DEVELOPER])
@pytest.mark.parametrize(
    "content_kind",
    [
        unified_llm.ContentKind.IMAGE,
        unified_llm.ContentKind.AUDIO,
        unified_llm.ContentKind.DOCUMENT,
        unified_llm.ContentKind.TOOL_CALL,
        unified_llm.ContentKind.TOOL_RESULT,
    ],
)
async def test_openai_adapter_rejects_unsupported_instruction_content_before_transport(
    role: unified_llm.Role,
    content_kind: unified_llm.ContentKind,
) -> None:
    response_body = {
        "id": "resp_invalid_instruction_content",
        "model": "gpt-5.2",
        "status": "completed",
        "output_text": "should not be sent",
    }
    captured_requests, transport = _make_complete_transport(response_body)
    adapter = unified_llm.OpenAIAdapter(api_key="instruction-key", transport=transport)
    request = unified_llm.Request(
        model="gpt-5.2",
        messages=[
            unified_llm.Message(
                role=role,
                content=[
                    unified_llm.ContentPart(
                        kind=unified_llm.ContentKind.TEXT,
                        text=f"{role.value} instructions",
                    ),
                    _unsupported_instruction_part(content_kind),
                ],
            )
        ],
    )

    with pytest.raises(unified_llm.InvalidRequestError, match=content_kind.value):
        await adapter.complete(request)

    assert captured_requests == []


@pytest.mark.asyncio
async def test_openai_adapter_translates_tool_role_text_to_function_call_output(
) -> None:
    response_body = {
        "id": "resp_tool_text",
        "model": "gpt-5.2",
        "status": "completed",
        "output_text": "done",
    }
    captured_requests, transport = _make_complete_transport(response_body)
    adapter = unified_llm.OpenAIAdapter(api_key="body-key", transport=transport)
    request = unified_llm.Request(
        model="gpt-5.2",
        messages=[
            unified_llm.Message(
                role=unified_llm.Role.TOOL,
                content=[
                    unified_llm.ContentPart(
                        kind=unified_llm.ContentKind.TEXT,
                        text="tool result text",
                    )
                ],
                tool_call_id="call_123",
            )
        ],
    )

    await adapter.complete(request)

    assert len(captured_requests) == 1
    body = _request_json(captured_requests[0])

    assert body["input"] == [
        {
            "type": "function_call_output",
            "call_id": "call_123",
            "output": "tool result text",
        }
    ]


@pytest.mark.asyncio
async def test_openai_adapter_rejects_unsupported_tool_role_payloads() -> None:
    response_body = {
        "id": "resp_invalid_tool_payload",
        "model": "gpt-5.2",
        "status": "completed",
        "output_text": "done",
    }
    captured_requests, transport = _make_complete_transport(response_body)
    adapter = unified_llm.OpenAIAdapter(api_key="body-key", transport=transport)
    request = unified_llm.Request(
        model="gpt-5.2",
        messages=[
            unified_llm.Message(
                role=unified_llm.Role.TOOL,
                content=[
                    unified_llm.ContentPart(
                        kind=unified_llm.ContentKind.IMAGE,
                        image=unified_llm.ImageData(url="https://example.test/result.png"),
                    )
                ],
                tool_call_id="call_123",
            )
        ],
    )

    with pytest.raises(unified_llm.InvalidRequestError, match="tool messages"):
        await adapter.complete(request)

    assert captured_requests == []


@pytest.mark.asyncio
@pytest.mark.parametrize(
    ("tool_choice", "expected"),
    [
        (unified_llm.ToolChoice.auto(), "auto"),
        (unified_llm.ToolChoice.none(), "none"),
        (unified_llm.ToolChoice.required(), "required"),
        (
            unified_llm.ToolChoice.named("lookup_weather"),
            {"type": "function", "name": "lookup_weather"},
        ),
    ],
)
async def test_openai_adapter_translates_tool_choice_modes(
    tool_choice: unified_llm.ToolChoice,
    expected: object,
) -> None:
    response_body = {
        "id": "resp_tool_choice",
        "model": "gpt-5.2",
        "status": "completed",
        "output_text": "ok",
    }
    captured_requests, transport = _make_complete_transport(response_body)
    adapter = unified_llm.OpenAIAdapter(api_key="tool-key", transport=transport)
    tool = unified_llm.Tool.passive(
        name="lookup_weather",
        parameters={
            "type": "object",
            "properties": {
                "city": {"type": "string"},
            },
            "required": ["city"],
        },
    )
    request = unified_llm.Request(
        model="gpt-5.2",
        messages=[unified_llm.Message.user("hello")],
        tools=[tool],
        tool_choice=tool_choice,
    )

    await adapter.complete(request)

    assert len(captured_requests) == 1
    body = _request_json(captured_requests[0])

    assert body["tools"] == [
        {
            "type": "function",
            "name": "lookup_weather",
            "strict": True,
            "parameters": {
                "type": "object",
                "properties": {
                    "city": {"type": "string"},
                },
                "required": ["city"],
            },
        }
    ]
    assert body["tool_choice"] == expected


@pytest.mark.asyncio
async def test_openai_adapter_rejects_unsupported_response_format_types(
    caplog: pytest.LogCaptureFixture,
    capsys: pytest.CaptureFixture[str],
) -> None:
    response_body = {
        "id": "resp_invalid_format",
        "model": "gpt-5.2",
        "status": "completed",
        "output_text": "ok",
    }
    captured_requests, transport = _make_complete_transport(response_body)
    adapter = unified_llm.OpenAIAdapter(api_key="format-key", transport=transport)
    request = unified_llm.Request(
        model="gpt-5.2",
        messages=[unified_llm.Message.user("hello")],
        response_format=unified_llm.ResponseFormat(type="json_object"),
    )

    with caplog.at_level(logging.WARNING, logger="unified_llm.adapters.openai"):
        with pytest.raises(unified_llm.InvalidRequestError, match="json_schema"):
            await adapter.complete(request)

    captured = capsys.readouterr()

    assert captured_requests == []
    assert captured.out == ""
    assert captured.err == ""
    assert any("json_schema" in record.message for record in caplog.records)


@pytest.mark.asyncio
async def test_openai_adapter_rejects_unsupported_message_content_kinds() -> None:
    response_body = {
        "id": "resp_invalid_content",
        "model": "gpt-5.2",
        "status": "completed",
        "output_text": "ok",
    }
    captured_requests, transport = _make_complete_transport(response_body)
    adapter = unified_llm.OpenAIAdapter(api_key="content-key", transport=transport)
    request = unified_llm.Request(
        model="gpt-5.2",
        messages=[
            unified_llm.Message.user(
                [
                    unified_llm.ContentPart(
                        kind=unified_llm.ContentKind.AUDIO,
                        audio=unified_llm.AudioData(
                            url="https://example.test/audio.mp3",
                        ),
                    )
                ]
            )
        ],
    )

    with pytest.raises(unified_llm.InvalidRequestError, match="audio"):
        await adapter.complete(request)

    assert captured_requests == []


@pytest.mark.asyncio
@pytest.mark.parametrize(
    ("content_kind", "redacted"),
    [
        (unified_llm.ContentKind.THINKING, False),
        (unified_llm.ContentKind.REDACTED_THINKING, True),
    ],
)
async def test_openai_adapter_rejects_reasoning_content_kinds_in_request_messages(
    content_kind: unified_llm.ContentKind,
    redacted: bool,
) -> None:
    response_body = {
        "id": "resp_invalid_reasoning",
        "model": "gpt-5.2",
        "status": "completed",
        "output_text": "ok",
    }
    captured_requests, transport = _make_complete_transport(response_body)
    adapter = unified_llm.OpenAIAdapter(api_key="reasoning-key", transport=transport)
    request = unified_llm.Request(
        model="gpt-5.2",
        messages=[
            unified_llm.Message.assistant(
                [
                    unified_llm.ContentPart(
                        kind=content_kind,
                        thinking=unified_llm.ThinkingData(
                            text="reasoning text",
                            redacted=redacted,
                        ),
                    )
                ]
            )
        ],
    )

    with pytest.raises(unified_llm.InvalidRequestError, match="reasoning content"):
        await adapter.complete(request)

    assert captured_requests == []


@pytest.mark.asyncio
async def test_openai_adapter_streams_from_the_responses_endpoint_and_accumulates_events() -> None:
    payload = (
        "event: response.created\n"
        'data: {"type":"response.created","response":{"id":"resp_stream","model":"gpt-5.2"}}\n'
        "\n"
        "event: response.output_text.delta\n"
        'data: {"type":"response.output_text.delta","item_id":"text_1","delta":"Hel"}\n'
        "\n"
        "event: response.output_text.delta\n"
        'data: {"type":"response.output_text.delta","item_id":"text_1","delta":"lo"}\n'
        "\n"
        "event: response.output_item.done\n"
        'data: {"type":"response.output_item.done","item":{"type":"output_text",'
        '"item_id":"text_1","text":"Hello"}}\n'
        "\n"
        "event: response.completed\n"
        'data: {"type":"response.completed","response":{"id":"resp_stream",'
        '"model":"gpt-5.2","status":"completed","usage":{"input_tokens":2,'
        '"output_tokens":3,"output_tokens_details":{"reasoning_tokens":1},'
        '"input_tokens_details":{"cached_tokens":1}}}}\n'
        "\n"
    )
    captured_requests, transport = _make_stream_transport(
        payload,
        headers={
            "x-ratelimit-remaining-requests": "7",
            "x-ratelimit-remaining-tokens": "99",
        },
    )
    adapter = unified_llm.OpenAIAdapter(
        api_key="stream-key",
        base_url="https://stream.example",
        transport=transport,
    )
    request = unified_llm.Request(
        model="gpt-5.2",
        messages=[unified_llm.Message.user("hello")],
        reasoning_effort="medium",
    )

    events = [event async for event in adapter.stream(request)]
    accumulator = unified_llm.StreamAccumulator.from_events(events)
    response = accumulator.response

    assert len(captured_requests) == 1
    sent_request = captured_requests[0]
    body = _request_json(sent_request)

    assert sent_request.method == "POST"
    assert sent_request.url.path == "/v1/responses"
    assert sent_request.headers["authorization"] == "Bearer stream-key"
    assert body["stream"] is True
    assert body["reasoning"] == {"effort": "medium"}
    assert body["input"][0]["content"][0]["text"] == "hello"
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
        event.delta
        for event in events
        if event.type == unified_llm.StreamEventType.TEXT_DELTA
    ] == ["Hel", "lo"]
    assert "".join(
        event.delta or ""
        for event in events
        if event.type == unified_llm.StreamEventType.TEXT_DELTA
    ) == "Hello"
    assert response.id == "resp_stream"
    assert response.text == "Hello"
    assert response.finish_reason.reason == "stop"
    assert response.usage.input_tokens == 2
    assert response.usage.output_tokens == 3
    assert response.usage.reasoning_tokens == 1
    assert response.usage.cache_read_tokens == 1
    assert response.rate_limit is not None
    assert response.rate_limit.requests_remaining == 7
    assert response.rate_limit.tokens_remaining == 99
    assert events[2].raw == {
        "type": "response.output_text.delta",
        "item_id": "text_1",
        "delta": "Hel",
    }
    assert events[4].raw == {
        "type": "response.output_item.done",
        "item": {"type": "output_text", "item_id": "text_1", "text": "Hello"},
    }
    assert events[-1].usage is not None
    assert events[-1].usage.cache_read_tokens == 1
    assert events[-1].raw == {
        "type": "response.completed",
        "response": {
            "id": "resp_stream",
            "model": "gpt-5.2",
            "status": "completed",
            "usage": {
                "input_tokens": 2,
                "output_tokens": 3,
                "output_tokens_details": {"reasoning_tokens": 1},
                "input_tokens_details": {"cached_tokens": 1},
            },
        },
    }


@pytest.mark.asyncio
async def test_openai_adapter_streams_function_call_argument_deltas_and_final_item_completion(
) -> None:
    payload = (
        "event: response.created\n"
        'data: {"type":"response.created","response":{"id":"resp_tools","model":"gpt-5.2"}}\n'
        "\n"
        "event: response.function_call_arguments.delta\n"
        'data: {"type":"response.function_call_arguments.delta","call_id":"call_123",'
        '"name":"lookup_weather","delta":"{\\"city\\":"}\n'
        "\n"
        "event: response.function_call_arguments.delta\n"
        'data: {"type":"response.function_call_arguments.delta","call_id":"call_123",'
        '"name":"lookup_weather","delta":"\\"Paris\\"}"}\n'
        "\n"
        "event: response.output_item.done\n"
        'data: {"type":"response.output_item.done","item":{"type":"function_call",'
        '"id":"call_123","name":"lookup_weather","arguments":{"city":"Paris"}}}\n'
        "\n"
        "event: response.completed\n"
        'data: {"type":"response.completed","response":{"id":"resp_tools",'
        '"model":"gpt-5.2","status":"completed","usage":{"input_tokens":4,'
        '"output_tokens":5,"output_tokens_details":{"reasoning_tokens":2},'
        '"input_tokens_details":{"cached_tokens":1}}}}\n'
        "\n"
    )
    captured_requests, transport = _make_stream_transport(
        payload,
        headers={
            "x-ratelimit-remaining-requests": "3",
            "x-ratelimit-remaining-tokens": "88",
        },
    )
    adapter = unified_llm.OpenAIAdapter(api_key="stream-key", transport=transport)
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
    assert [
        event.tool_call.raw_arguments
        for event in events
        if event.type == unified_llm.StreamEventType.TOOL_CALL_DELTA
    ] == ['{"city":', '"Paris"}']
    assert "".join(
        event.tool_call.raw_arguments or ""
        for event in events
        if event.type == unified_llm.StreamEventType.TOOL_CALL_DELTA
    ) == '{"city":"Paris"}'
    assert events[1].tool_call is not None
    assert events[1].tool_call.id == "call_123"
    assert events[1].tool_call.name == "lookup_weather"
    assert events[2].tool_call is not None
    assert events[2].tool_call.raw_arguments == '{"city":'
    assert events[3].tool_call is not None
    assert events[3].tool_call.raw_arguments == '"Paris"}'
    assert events[4].tool_call is not None
    assert events[4].tool_call.arguments == {"city": "Paris"}
    assert events[4].tool_call.raw_arguments == '{"city":"Paris"}'
    assert events[-1].finish_reason.reason == "tool_calls"
    assert events[-1].usage.input_tokens == 4
    assert events[-1].usage.output_tokens == 5
    assert events[-1].usage.reasoning_tokens == 2
    assert events[-1].response.rate_limit is not None
    assert events[-1].response.rate_limit.requests_remaining == 3
    assert events[-1].response.rate_limit.tokens_remaining == 88
    assert response.id == "resp_tools"
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
        "type": "response.completed",
        "response": {
            "id": "resp_tools",
            "model": "gpt-5.2",
            "status": "completed",
            "usage": {
                "input_tokens": 4,
                "output_tokens": 5,
                "output_tokens_details": {"reasoning_tokens": 2},
                "input_tokens_details": {"cached_tokens": 1},
            },
        },
    }


@pytest.mark.asyncio
async def test_openai_adapter_streams_function_call_deltas_with_final_call_id(
) -> None:
    payload = (
        "event: response.created\n"
        'data: {"type":"response.created","response":{"id":"resp_tools","model":"gpt-5.2"}}\n'
        "\n"
        "event: response.function_call_arguments.delta\n"
        'data: {"type":"response.function_call_arguments.delta","item_id":"item_123",'
        '"name":"lookup_weather","delta":"{\\"city\\":"}\n'
        "\n"
        "event: response.function_call_arguments.delta\n"
        'data: {"type":"response.function_call_arguments.delta","item_id":"item_123",'
        '"name":"lookup_weather","delta":"\\"Paris\\"}"}\n'
        "\n"
        "event: response.output_item.done\n"
        'data: {"type":"response.output_item.done","item":{"type":"function_call",'
        '"id":"item_123","call_id":"call_123","name":"lookup_weather"}}\n'
        "\n"
        "event: response.completed\n"
        'data: {"type":"response.completed","response":{"id":"resp_tools",'
        '"model":"gpt-5.2","status":"completed","usage":{"input_tokens":4,'
        '"output_tokens":5,"output_tokens_details":{"reasoning_tokens":2},'
        '"input_tokens_details":{"cached_tokens":1}}}}\n'
        "\n"
    )
    captured_requests, transport = _make_stream_transport(
        payload,
        headers={
            "x-ratelimit-remaining-requests": "3",
            "x-ratelimit-remaining-tokens": "88",
        },
    )
    adapter = unified_llm.OpenAIAdapter(api_key="stream-key", transport=transport)
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
    assert events[1].tool_call.id == "item_123"
    assert events[2].tool_call is not None
    assert events[2].tool_call.id == "item_123"
    assert events[3].tool_call is not None
    assert events[3].tool_call.id == "item_123"
    assert events[4].tool_call is not None
    assert events[4].tool_call.id == "call_123"
    assert events[4].tool_call.name == "lookup_weather"
    assert events[4].tool_call.arguments == {"city": "Paris"}
    assert events[4].tool_call.raw_arguments == '{"city":"Paris"}'
    assert events[-1].response is not None
    assert events[-1].response.tool_calls == [
        unified_llm.ToolCall(
            id="call_123",
            name="lookup_weather",
            arguments={"city": "Paris"},
            raw_arguments='{"city":"Paris"}',
            type="function",
        )
    ]
    assert response.tool_calls == [
        unified_llm.ToolCall(
            id="call_123",
            name="lookup_weather",
            arguments={"city": "Paris"},
            raw_arguments='{"city":"Paris"}',
            type="function",
        )
    ]
    assert response.finish_reason.reason == "tool_calls"
    assert response.raw == {
        "type": "response.completed",
        "response": {
            "id": "resp_tools",
            "model": "gpt-5.2",
            "status": "completed",
            "usage": {
                "input_tokens": 4,
                "output_tokens": 5,
                "output_tokens_details": {"reasoning_tokens": 2},
                "input_tokens_details": {"cached_tokens": 1},
            },
        },
    }


@pytest.mark.asyncio
async def test_openai_adapter_high_level_text_stream_includes_the_first_chunk() -> None:
    payload = (
        "event: response.created\n"
        'data: {"type":"response.created","response":{"id":"resp_stream","model":"gpt-5.2"}}\n'
        "\n"
        "event: response.output_text.delta\n"
        'data: {"type":"response.output_text.delta","item_id":"text_1","delta":"Hel"}\n'
        "\n"
        "event: response.output_text.delta\n"
        'data: {"type":"response.output_text.delta","item_id":"text_1","delta":"lo"}\n'
        "\n"
        "event: response.output_item.done\n"
        'data: {"type":"response.output_item.done","item":{"type":"output_text",'
        '"item_id":"text_1","text":"Hello"}}\n'
        "\n"
        "event: response.completed\n"
        'data: {"type":"response.completed","response":{"id":"resp_stream",'
        '"model":"gpt-5.2","status":"completed"}}\n'
        "\n"
    )
    captured_requests, transport = _make_stream_transport(payload)
    adapter = unified_llm.OpenAIAdapter(api_key="stream-key", transport=transport)
    client = unified_llm.Client(providers={"openai": adapter}, default_provider="openai")
    stream = unified_llm.stream(model="gpt-5.2", prompt="hello", client=client)

    deltas = [delta async for delta in stream.text_stream]
    response = await stream.response()
    await stream.close()

    assert len(captured_requests) == 1
    assert _request_json(captured_requests[0])["stream"] is True
    assert deltas == ["Hel", "lo"]
    assert response.text == "Hello"


@pytest.mark.asyncio
async def test_openai_adapter_streams_explicit_error_events_as_terminal_errors() -> None:
    payload = (
        "event: response.created\n"
        'data: {"type":"response.created","response":{"id":"resp_error","model":"gpt-5.2"}}\n'
        "\n"
        "event: response.output_text.delta\n"
        'data: {"type":"response.output_text.delta","item_id":"text_1","delta":"Hel"}\n'
        "\n"
        "event: error\n"
        'data: {"message":"boom","code":"rate_limit"}\n'
        "\n"
    )
    captured_requests, transport = _make_stream_transport(payload)
    adapter = unified_llm.OpenAIAdapter(api_key="stream-key", transport=transport)
    request = unified_llm.Request(
        model="gpt-5.2",
        messages=[unified_llm.Message.user("hello")],
    )

    events = [event async for event in adapter.stream(request)]
    accumulator = unified_llm.StreamAccumulator.from_events(events)
    response = accumulator.response

    assert len(captured_requests) == 1
    assert [event.type for event in events] == [
        unified_llm.StreamEventType.STREAM_START,
        unified_llm.StreamEventType.TEXT_START,
        unified_llm.StreamEventType.TEXT_DELTA,
        unified_llm.StreamEventType.ERROR,
    ]
    assert events[-1].error is not None
    assert events[-1].error.provider == "openai"
    assert events[-1].error.message == "boom"
    assert events[-1].raw == {"message": "boom", "code": "rate_limit"}
    assert response.text == "Hel"
    assert response.finish_reason.reason == "error"
    assert accumulator.finish_event is not None
    assert accumulator.finish_event.type == unified_llm.StreamEventType.ERROR


@pytest.mark.asyncio
async def test_openai_adapter_preserves_raw_provider_events_in_responses_stream(
    capsys: pytest.CaptureFixture[str],
) -> None:
    payload = (
        "event: response.created\n"
        'data: {"type":"response.created","response":{"id":"resp_stream","model":"gpt-5.2"}}\n'
        "\n"
        "event: response.in_progress\n"
        'data: {"type":"response.in_progress","response":{"id":"resp_stream","model":"gpt-5.2"}}\n'
        "\n"
        "event: response.completed\n"
        'data: {"type":"response.completed","response":{"id":"resp_stream",'
        '"model":"gpt-5.2","status":"completed","output_text":"done"}}\n'
        "\n"
    )
    captured_requests, transport = _make_stream_transport(payload)
    adapter = unified_llm.OpenAIAdapter(api_key="stream-key", transport=transport)
    request = unified_llm.Request(
        model="gpt-5.2",
        messages=[unified_llm.Message.user("hello")],
    )

    events = [event async for event in adapter.stream(request)]
    captured = capsys.readouterr()

    assert len(captured_requests) == 1
    assert [event.type for event in events] == [
        unified_llm.StreamEventType.STREAM_START,
        unified_llm.StreamEventType.PROVIDER_EVENT,
        unified_llm.StreamEventType.FINISH,
    ]
    assert events[1].raw == {
        "type": "response.in_progress",
        "response": {
            "id": "resp_stream",
            "model": "gpt-5.2",
        },
    }
    assert events[-1].response is not None
    assert events[-1].response.text == "done"
    assert events[-1].finish_reason.reason == "stop"
    assert captured.out == ""
    assert captured.err == ""


@pytest.mark.asyncio
async def test_openai_adapter_stream_returns_after_the_first_sse_chunk_without_buffering(
) -> None:
    first_chunk = (
        b"event: response.created\n"
        b'data: {"type":"response.created","response":{"id":"resp_stream",'
        b'"model":"gpt-5.2"}}\n'
        b"\n"
    )
    second_chunk = (
        b"event: response.completed\n"
        b'data: {"type":"response.completed","response":{"id":"resp_stream",'
        b'"model":"gpt-5.2","status":"completed"}}\n'
        b"\n"
    )
    captured_requests: list[httpx.Request] = []
    blocking_stream = _BlockingStream(first_chunk, second_chunk)

    async def handler(request: httpx.Request) -> httpx.Response:
        captured_requests.append(request)
        return httpx.Response(
            200,
            headers={"content-type": "text/event-stream"},
            stream=blocking_stream,
        )

    adapter = unified_llm.OpenAIAdapter(
        api_key="stream-key",
        transport=httpx.MockTransport(handler),
    )
    request = unified_llm.Request(
        model="gpt-5.2",
        messages=[unified_llm.Message.user("hello")],
    )

    stream = adapter.stream(request)
    try:
        first = await asyncio.wait_for(stream.__anext__(), timeout=0.5)
    finally:
        await stream.aclose()

    assert len(captured_requests) == 1
    assert first.type == unified_llm.StreamEventType.STREAM_START
    assert first.response is not None
    assert first.response.id == "resp_stream"
    assert blocking_stream.closed is True


@pytest.mark.asyncio
async def test_openai_adapter_close_respects_client_ownership_and_logs_failures(
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
    borrowed_adapter = unified_llm.OpenAIAdapter(
        api_key="close-key",
        client=borrowed,
    )
    await borrowed_adapter.close()
    assert borrowed.closed is False

    owned = _CloseRecorder()
    owned_adapter = unified_llm.OpenAIAdapter(
        api_key="close-key",
        client=owned,
        owns_client=True,
    )
    await owned_adapter.close()
    assert owned.closed is True

    failing = _CloseRecorder(error=RuntimeError("boom"))
    failing_adapter = unified_llm.OpenAIAdapter(
        api_key="close-key",
        client=failing,
        owns_client=True,
    )

    with caplog.at_level(logging.ERROR, logger="unified_llm.adapters.openai"):
        await failing_adapter.close()

    captured = capsys.readouterr()
    assert captured.out == ""
    assert captured.err == ""
    assert failing.closed is True
    assert any(
        record.name == "unified_llm.adapters.openai"
        and "Unexpected error closing OpenAI HTTP client" in record.message
        for record in caplog.records
    )
