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


def _sse_event(event: str, payload: dict[str, object]) -> str:
    return f"event: {event}\ndata: {json.dumps(payload, separators=(',', ':'), sort_keys=True)}\n\n"


def _anthropic_schema_block(schema: dict[str, object]) -> str:
    schema_json = json.dumps(schema, separators=(",", ":"), sort_keys=True)
    return f"JSON Schema:\n```json\n{schema_json}\n```"


def _anthropic_structured_output_instruction(schema: dict[str, object]) -> str:
    return (
        "Return only valid JSON that matches the provided schema.\n\n"
        f"{_anthropic_schema_block(schema)}"
    )


def test_anthropic_adapter_is_exposed_through_the_public_adapter_namespace() -> None:
    from unified_llm.adapters import AnthropicAdapter as AdapterAnthropicAdapter

    assert AdapterAnthropicAdapter is unified_llm.AnthropicAdapter


@pytest.mark.asyncio
async def test_anthropic_adapter_uses_the_native_messages_endpoint_and_headers() -> None:
    response_body = {
        "id": "msg_123",
        "type": "message",
        "role": "assistant",
        "model": "claude-sonnet-4-5",
        "content": [
            {
                "type": "text",
                "text": "Hello",
            }
        ],
        "stop_reason": "end_turn",
        "stop_sequence": None,
        "usage": {
            "input_tokens": 11,
            "output_tokens": 13,
            "cache_read_input_tokens": 5,
            "cache_creation_input_tokens": 2,
        },
    }
    captured_requests, transport = _make_complete_transport(response_body)
    adapter = unified_llm.AnthropicAdapter(
        api_key="explicit-key",
        base_url="https://explicit.example/api",
        timeout=12.5,
        default_headers={"X-Custom": "value"},
        transport=transport,
    )
    request = unified_llm.Request(
        model="claude-sonnet-4-5",
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
            unified_llm.Message.user("cached user context"),
            unified_llm.Message.assistant("cached assistant context"),
            unified_llm.Message.user("hello"),
        ],
        tools=[
            unified_llm.Tool.passive(
                "lookup",
                "Lookup a fact",
                {
                    "type": "object",
                    "properties": {
                        "query": {
                            "type": "string",
                        }
                    },
                    "required": ["query"],
                    "additionalProperties": False,
                },
            ),
        ],
    )

    response = await adapter.complete(request)

    assert adapter.api_key == "explicit-key"
    assert adapter.base_url == "https://explicit.example/api/v1"
    assert adapter.timeout == 12.5
    assert adapter.default_headers == {"X-Custom": "value"}
    assert adapter.config == {
        "api_key": "explicit-key",
        "base_url": "https://explicit.example/api/v1",
    }
    assert len(captured_requests) == 1

    sent_request = captured_requests[0]
    body = _request_json(sent_request)

    assert sent_request.method == "POST"
    assert sent_request.url.path == "/api/v1/messages"
    assert sent_request.headers["x-api-key"] == "explicit-key"
    assert sent_request.headers["anthropic-version"] == "2023-06-01"
    assert sent_request.headers["anthropic-beta"] == "prompt-caching-2024-07-31"
    assert sent_request.headers["x-custom"] == "value"
    assert "authorization" not in sent_request.headers
    assert body["model"] == "claude-sonnet-4-5"
    assert body["max_tokens"] == 4096
    assert body["system"] == [
        {
            "type": "text",
            "text": "system instructions\n\ndeveloper instructions",
            "cache_control": {"type": "ephemeral"},
        }
    ]
    assert body["tools"] == [
        {
            "name": "lookup",
            "description": "Lookup a fact",
            "input_schema": {
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                    }
                },
                "required": ["query"],
                "additionalProperties": False,
            },
            "cache_control": {"type": "ephemeral"},
        }
    ]
    assert body["messages"] == [
        {
            "role": "user",
            "content": [
                {
                    "type": "text",
                    "text": "cached user context",
                }
            ],
        },
        {
            "role": "assistant",
            "content": [
                {
                    "type": "text",
                    "text": "cached assistant context",
                    "cache_control": {"type": "ephemeral"},
                }
            ],
        },
        {
            "role": "user",
            "content": [
                {
                    "type": "text",
                    "text": "hello",
                }
            ],
        },
    ]
    assert "cache_control" not in body
    assert "input" not in body

    assert response.provider == "anthropic"
    assert response.id == "msg_123"
    assert response.model == "claude-sonnet-4-5"
    assert response.text == "Hello"
    assert response.finish_reason.reason == "stop"
    assert response.finish_reason.raw == "end_turn"
    assert response.usage.input_tokens == 11
    assert response.usage.output_tokens == 13
    assert response.usage.total_tokens == 24
    assert response.usage.cache_read_tokens == 5
    assert response.usage.cache_write_tokens == 2
    assert response.raw == response_body


@pytest.mark.asyncio
async def test_anthropic_adapter_translates_mixed_content_and_tool_results(
) -> None:
    response_body = {
        "id": "msg_request_translation",
        "type": "message",
        "role": "assistant",
        "model": "claude-sonnet-4-5",
        "content": [
            {
                "type": "text",
                "text": "ok",
            }
        ],
        "stop_reason": "end_turn",
        "usage": {
            "input_tokens": 1,
            "output_tokens": 1,
        },
    }
    captured_requests, transport = _make_complete_transport(response_body)
    adapter = unified_llm.AnthropicAdapter(
        api_key="translate-key",
        transport=transport,
    )
    request = unified_llm.Request(
        model="claude-sonnet-4-5",
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
            unified_llm.Message.user("first user"),
            unified_llm.Message(
                role=unified_llm.Role.USER,
                content=[
                    unified_llm.ContentPart(
                        kind=unified_llm.ContentKind.TEXT,
                        text="second user",
                    ),
                    unified_llm.ContentPart(
                        kind=unified_llm.ContentKind.IMAGE,
                        image=unified_llm.ImageData(url="https://example.com/cat.png"),
                    ),
                    unified_llm.ContentPart(
                        kind=unified_llm.ContentKind.IMAGE,
                        image=unified_llm.ImageData(data=b"cat", media_type="image/png"),
                    ),
                ],
            ),
            unified_llm.Message(
                role=unified_llm.Role.ASSISTANT,
                content=[
                    unified_llm.ContentPart(
                        kind=unified_llm.ContentKind.TEXT,
                        text="assistant turn",
                    ),
                    unified_llm.ContentPart(
                        kind=unified_llm.ContentKind.THINKING,
                        thinking=unified_llm.ThinkingData(
                            text="reasoning",
                            signature="sig-1",
                        ),
                    ),
                    unified_llm.ContentPart(
                        kind=unified_llm.ContentKind.REDACTED_THINKING,
                        thinking=unified_llm.ThinkingData(
                            text="opaque",
                            redacted=True,
                        ),
                    ),
                    unified_llm.ContentPart(
                        kind=unified_llm.ContentKind.TOOL_CALL,
                        tool_call=unified_llm.ToolCallData(
                            id="call_1",
                            name="lookup",
                            arguments={"query": "Paris"},
                        ),
                    ),
                ],
            ),
            unified_llm.Message.assistant("follow-up"),
            unified_llm.Message.tool_result(
                "call_1",
                "tool result",
                image_data=b"image-bytes",
                image_media_type="image/png",
            ),
        ],
        provider_options={
            "anthropic": {
                "auto_cache": False,
            }
        },
    )

    response = await adapter.complete(request)

    assert len(captured_requests) == 1
    sent_request = captured_requests[0]
    body = _request_json(sent_request)

    assert sent_request.method == "POST"
    assert sent_request.url.path == "/v1/messages"
    assert sent_request.headers["x-api-key"] == "translate-key"
    assert sent_request.headers["anthropic-version"] == "2023-06-01"
    assert "anthropic-beta" not in sent_request.headers
    assert body["max_tokens"] == 4096
    assert body["system"] == "system instructions\n\ndeveloper instructions"
    assert body["messages"] == [
        {
            "role": "user",
            "content": [
                {
                    "type": "text",
                    "text": "first user",
                },
                {
                    "type": "text",
                    "text": "second user",
                },
                {
                    "type": "image",
                    "source": {
                        "type": "url",
                        "url": "https://example.com/cat.png",
                    },
                },
                {
                    "type": "image",
                    "source": {
                        "type": "base64",
                        "media_type": "image/png",
                        "data": base64.b64encode(b"cat").decode("ascii"),
                    },
                },
            ],
        },
        {
            "role": "assistant",
            "content": [
                {
                    "type": "text",
                    "text": "assistant turn",
                },
                {
                    "type": "thinking",
                    "thinking": "reasoning",
                    "signature": "sig-1",
                },
                {
                    "type": "redacted_thinking",
                    "data": "opaque",
                },
                {
                    "type": "tool_use",
                    "id": "call_1",
                    "name": "lookup",
                    "input": {
                        "query": "Paris",
                    },
                },
                {
                    "type": "text",
                    "text": "follow-up",
                },
            ],
        },
        {
            "role": "user",
            "content": [
                {
                    "type": "tool_result",
                    "tool_use_id": "call_1",
                    "content": [
                        {
                            "type": "text",
                            "text": "tool result",
                        },
                        {
                            "type": "image",
                            "source": {
                                "type": "base64",
                                "media_type": "image/png",
                                "data": base64.b64encode(b"image-bytes").decode("ascii"),
                            },
                        },
                    ],
                }
            ],
        },
    ]
    assert "cache_control" not in body
    assert response.provider == "anthropic"
    assert response.text == "ok"


@pytest.mark.asyncio
async def test_anthropic_adapter_serializes_passive_tools_with_default_metadata() -> None:
    response_body = {
        "id": "msg_passive_tool",
        "type": "message",
        "role": "assistant",
        "model": "claude-sonnet-4-5",
        "content": [
            {
                "type": "text",
                "text": "ok",
            }
        ],
        "stop_reason": "end_turn",
        "usage": {
            "input_tokens": 1,
            "output_tokens": 1,
        },
    }
    captured_requests, transport = _make_complete_transport(response_body)
    adapter = unified_llm.AnthropicAdapter(
        api_key="tool-key",
        transport=transport,
    )
    request = unified_llm.Request(
        model="claude-sonnet-4-5",
        messages=[unified_llm.Message.user("hello")],
        tools=[unified_llm.Tool.passive("ping")],
    )

    response = await adapter.complete(request)

    assert len(captured_requests) == 1

    body = _request_json(captured_requests[0])
    assert body["tools"] == [
        {
            "name": "ping",
            "description": "",
            "input_schema": {
                "type": "object",
                "properties": {},
            },
            "cache_control": {"type": "ephemeral"},
        }
    ]
    assert body["messages"] == [
        {
            "role": "user",
            "content": [
                {
                    "type": "text",
                    "text": "hello",
                }
            ],
        }
    ]
    assert captured_requests[0].headers["anthropic-beta"] == "prompt-caching-2024-07-31"
    assert response.text == "ok"


@pytest.mark.asyncio
async def test_anthropic_adapter_uses_environment_fallbacks_for_api_key_and_base_url(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    response_body = {
        "id": "msg_env",
        "type": "message",
        "role": "assistant",
        "model": "claude-sonnet-4-5",
        "content": [
            {
                "type": "text",
                "text": "Fallbacks work",
            }
        ],
        "stop_reason": "end_turn",
        "usage": {
            "input_tokens": 5,
            "output_tokens": 7,
        },
    }
    captured_requests, transport = _make_complete_transport(response_body)
    monkeypatch.setenv("ANTHROPIC_API_KEY", "env-key")
    monkeypatch.setenv("ANTHROPIC_BASE_URL", "https://env.example/api")
    adapter = unified_llm.AnthropicAdapter(transport=transport)
    request = unified_llm.Request(
        model="claude-sonnet-4-5",
        messages=[unified_llm.Message.user("hello")],
    )

    await adapter.complete(request)

    assert adapter.config == {
        "api_key": "env-key",
        "base_url": "https://env.example/api/v1",
    }
    assert len(captured_requests) == 1

    sent_request = captured_requests[0]
    body = _request_json(sent_request)

    assert sent_request.url.path == "/api/v1/messages"
    assert sent_request.headers["x-api-key"] == "env-key"
    assert sent_request.headers["anthropic-version"] == "2023-06-01"
    assert "anthropic-beta" not in sent_request.headers
    assert "cache_control" not in body
    assert body["messages"] == [
        {
            "role": "user",
            "content": [
                {
                    "type": "text",
                    "text": "hello",
                }
            ],
        }
    ]


@pytest.mark.asyncio
async def test_anthropic_adapter_streams_from_the_native_messages_endpoint() -> None:
    payload = "".join(
        [
            _sse_event(
                "message_start",
                {
                    "type": "message_start",
                    "message": {
                        "id": "msg_stream",
                        "type": "message",
                        "role": "assistant",
                        "model": "claude-sonnet-4-5",
                        "content": [],
                        "stop_reason": None,
                        "stop_sequence": None,
                        "usage": {
                            "input_tokens": 11,
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
                        "type": "text",
                        "text": "",
                    },
                },
            ),
            _sse_event(
                "content_block_delta",
                {
                    "type": "content_block_delta",
                    "index": 0,
                    "delta": {
                        "type": "text_delta",
                        "text": "Hello",
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
                "message_delta",
                {
                    "type": "message_delta",
                    "delta": {
                        "stop_reason": "end_turn",
                        "stop_sequence": None,
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
    captured_requests, transport = _make_stream_transport(
        payload,
        headers={
            "anthropic-ratelimit-requests-remaining": "7",
        },
    )
    adapter = unified_llm.AnthropicAdapter(
        api_key="stream-key",
        base_url="https://stream.example",
        transport=transport,
    )
    request = unified_llm.Request(
        model="claude-sonnet-4-5",
        messages=[unified_llm.Message.user("hello")],
    )

    events = [event async for event in adapter.stream(request)]

    assert len(captured_requests) == 1

    sent_request = captured_requests[0]
    body = _request_json(sent_request)

    assert sent_request.method == "POST"
    assert sent_request.url.path == "/v1/messages"
    assert sent_request.headers["x-api-key"] == "stream-key"
    assert sent_request.headers["anthropic-version"] == "2023-06-01"
    assert "anthropic-beta" not in sent_request.headers
    assert body["stream"] is True
    assert "cache_control" not in body
    assert body["messages"] == [
        {
            "role": "user",
            "content": [
                {
                    "type": "text",
                    "text": "hello",
                }
            ],
        }
    ]

    assert [event.type for event in events] == [
        unified_llm.StreamEventType.STREAM_START,
        unified_llm.StreamEventType.TEXT_START,
        unified_llm.StreamEventType.TEXT_DELTA,
        unified_llm.StreamEventType.TEXT_END,
        unified_llm.StreamEventType.FINISH,
    ]
    finish_event = events[-1]
    assert finish_event.finish_reason is not None
    assert finish_event.finish_reason.reason == "stop"
    assert finish_event.response is not None
    assert finish_event.response.text == "Hello"
    assert finish_event.response.usage.input_tokens == 11
    assert finish_event.response.usage.output_tokens == 5
    assert finish_event.response.usage.total_tokens == 16
    assert finish_event.response.usage.cache_read_tokens == 2
    assert finish_event.response.usage.cache_write_tokens == 1
    assert finish_event.response.rate_limit == unified_llm.RateLimitInfo(
        requests_remaining=7,
        requests_limit=None,
        tokens_remaining=None,
        tokens_limit=None,
        reset_at=None,
    )
    assert isinstance(finish_event.response.raw, list)
    assert len(finish_event.response.raw) == 6
    assert finish_event.response.raw[-1] == {"type": "message_stop"}


@pytest.mark.asyncio
async def test_anthropic_adapter_preserves_whitespace_text_deltas_and_tool_json_fragments(
) -> None:
    payload = "".join(
        [
            _sse_event(
                "message_start",
                {
                    "type": "message_start",
                    "message": {
                        "id": "msg_whitespace",
                        "type": "message",
                        "role": "assistant",
                        "model": "claude-sonnet-4-5",
                        "content": [],
                        "stop_reason": None,
                        "stop_sequence": None,
                        "usage": {
                            "input_tokens": 2,
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
                        "type": "text",
                        "text": "Hello ",
                    },
                },
            ),
            _sse_event(
                "content_block_delta",
                {
                    "type": "content_block_delta",
                    "index": 0,
                    "delta": {
                        "type": "text_delta",
                        "text": " world",
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
                        "type": "tool_use",
                        "id": "call_456",
                        "name": "lookup",
                        "input": {},
                    },
                },
            ),
            _sse_event(
                "content_block_delta",
                {
                    "type": "content_block_delta",
                    "index": 1,
                    "delta": {
                        "type": "input_json_delta",
                        "partial_json": '{"location":"Par ',
                    },
                },
            ),
            _sse_event(
                "content_block_delta",
                {
                    "type": "content_block_delta",
                    "index": 1,
                    "delta": {
                        "type": "input_json_delta",
                        "partial_json": 'is"}',
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
                        "output_tokens": 4,
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
    captured_requests, transport = _make_stream_transport(payload)
    adapter = unified_llm.AnthropicAdapter(
        api_key="stream-key",
        base_url="https://stream.example",
        transport=transport,
    )
    request = unified_llm.Request(
        model="claude-sonnet-4-5",
        messages=[unified_llm.Message.user("hello")],
    )

    events = [event async for event in adapter.stream(request)]

    assert len(captured_requests) == 1
    assert [event.type for event in events] == [
        unified_llm.StreamEventType.STREAM_START,
        unified_llm.StreamEventType.TEXT_START,
        unified_llm.StreamEventType.TEXT_DELTA,
        unified_llm.StreamEventType.TEXT_END,
        unified_llm.StreamEventType.TOOL_CALL_START,
        unified_llm.StreamEventType.TOOL_CALL_DELTA,
        unified_llm.StreamEventType.TOOL_CALL_DELTA,
        unified_llm.StreamEventType.TOOL_CALL_END,
        unified_llm.StreamEventType.FINISH,
    ]
    assert events[1].delta == "Hello "
    assert events[2].delta == " world"
    assert events[5].tool_call is not None
    assert events[5].tool_call.arguments == '{"location":"Par '
    assert events[5].tool_call.raw_arguments == '{"location":"Par '
    assert events[6].tool_call is not None
    assert events[6].tool_call.arguments == 'is"}'
    assert events[6].tool_call.raw_arguments == 'is"}'

    finish_event = events[-1]
    assert finish_event.finish_reason is not None
    assert finish_event.finish_reason.reason == "stop"
    assert finish_event.response is not None
    assert finish_event.response.text == "Hello  world"
    assert finish_event.response.tool_calls[0].id == "call_456"
    assert finish_event.response.tool_calls[0].name == "lookup"
    assert finish_event.response.tool_calls[0].arguments == {"location": "Par is"}
    assert finish_event.response.tool_calls[0].raw_arguments == '{"location":"Par is"}'


@pytest.mark.asyncio
async def test_anthropic_adapter_streams_thinking_redaction_and_tool_calls_into_the_final_response(
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
                "content_block_start",
                {
                    "type": "content_block_start",
                    "index": 2,
                    "content_block": {
                        "type": "tool_use",
                        "id": "call_123",
                        "name": "lookup",
                        "input": {},
                    },
                },
            ),
            _sse_event(
                "content_block_delta",
                {
                    "type": "content_block_delta",
                    "index": 2,
                    "delta": {
                        "type": "input_json_delta",
                        "partial_json": '{"location":"Par',
                    },
                },
            ),
            _sse_event(
                "content_block_delta",
                {
                    "type": "content_block_delta",
                    "index": 2,
                    "delta": {
                        "type": "input_json_delta",
                        "partial_json": 'is"}',
                    },
                },
            ),
            _sse_event(
                "content_block_stop",
                {
                    "type": "content_block_stop",
                    "index": 2,
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
    captured_requests, transport = _make_stream_transport(
        payload,
        headers={
            "anthropic-ratelimit-requests-remaining": "4",
        },
    )
    adapter = unified_llm.AnthropicAdapter(
        api_key="stream-key",
        base_url="https://stream.example",
        transport=transport,
    )
    request = unified_llm.Request(
        model="claude-sonnet-4-5",
        messages=[unified_llm.Message.user("hello")],
    )

    events = [event async for event in adapter.stream(request)]

    assert len(captured_requests) == 1
    assert [event.type for event in events] == [
        unified_llm.StreamEventType.STREAM_START,
        unified_llm.StreamEventType.REASONING_START,
        unified_llm.StreamEventType.REASONING_DELTA,
        unified_llm.StreamEventType.REASONING_END,
        unified_llm.StreamEventType.REASONING_START,
        unified_llm.StreamEventType.REASONING_END,
        unified_llm.StreamEventType.TOOL_CALL_START,
        unified_llm.StreamEventType.TOOL_CALL_DELTA,
        unified_llm.StreamEventType.TOOL_CALL_DELTA,
        unified_llm.StreamEventType.TOOL_CALL_END,
        unified_llm.StreamEventType.FINISH,
    ]

    finish_event = events[-1]
    assert finish_event.response is not None
    assert finish_event.response.finish_reason.reason == "stop"
    assert finish_event.response.text == ""
    assert finish_event.response.reasoning == "reasoning thouopaque-123"
    assert finish_event.response.usage.input_tokens == 3
    assert finish_event.response.usage.output_tokens == 5
    assert finish_event.response.usage.total_tokens == 8
    assert finish_event.response.usage.cache_read_tokens == 2
    assert finish_event.response.usage.cache_write_tokens == 1
    assert finish_event.response.usage.reasoning_tokens is not None
    assert finish_event.response.usage.reasoning_tokens > 0
    assert finish_event.response.rate_limit == unified_llm.RateLimitInfo(
        requests_remaining=4,
        requests_limit=None,
        tokens_remaining=None,
        tokens_limit=None,
        reset_at=None,
    )
    assert isinstance(finish_event.response.raw, list)
    assert finish_event.response.raw[-1] == {"type": "message_stop"}

    content_kinds = [part.kind for part in finish_event.response.message.content]
    assert content_kinds == [
        unified_llm.ContentKind.THINKING,
        unified_llm.ContentKind.REDACTED_THINKING,
        unified_llm.ContentKind.TOOL_CALL,
    ]
    assert finish_event.response.message.content[0].thinking is not None
    assert finish_event.response.message.content[0].thinking.signature == "sig-123"
    assert finish_event.response.message.content[0].thinking.redacted is False
    assert finish_event.response.message.content[1].thinking is not None
    assert finish_event.response.message.content[1].thinking.redacted is True
    assert finish_event.response.message.content[1].thinking.text == "opaque-123"
    assert events[6].tool_call is not None
    assert events[6].tool_call.id == "call_123"
    assert events[6].tool_call.name == "lookup"
    assert events[6].tool_call.arguments == ""
    assert events[6].tool_call.raw_arguments == ""
    assert events[7].tool_call is not None
    assert events[7].tool_call.raw_arguments == '{"location":"Par'
    assert events[8].tool_call is not None
    assert events[8].tool_call.raw_arguments == 'is"}'
    assert events[9].tool_call is not None
    assert events[9].tool_call.id == "call_123"
    assert events[9].tool_call.name == "lookup"
    assert events[9].tool_call.arguments == {"location": "Paris"}
    assert events[9].tool_call.raw_arguments == '{"location":"Paris"}'
    assert finish_event.response.tool_calls[0].id == "call_123"
    assert finish_event.response.tool_calls[0].name == "lookup"
    assert finish_event.response.tool_calls[0].arguments == {"location": "Paris"}
    assert finish_event.response.tool_calls[0].raw_arguments == '{"location":"Paris"}'


@pytest.mark.asyncio
async def test_anthropic_adapter_logs_and_converts_malformed_stream_payloads(
    caplog: pytest.LogCaptureFixture,
    capsys: pytest.CaptureFixture[str],
) -> None:
    payload = "".join(
        [
            _sse_event(
                "message_start",
                {
                    "type": "message_start",
                    "message": {
                        "id": "msg_stream_error",
                        "type": "message",
                        "role": "assistant",
                        "model": "claude-sonnet-4-5",
                        "content": [],
                        "stop_reason": None,
                        "stop_sequence": None,
                        "usage": {
                            "input_tokens": 1,
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
                        "type": "text",
                        "text": "",
                    },
                },
            ),
            _sse_event(
                "content_block_delta",
                {
                    "type": "content_block_delta",
                    "index": 0,
                    "delta": {
                        "type": "text_delta",
                        "text": "Hel",
                    },
                },
            ),
            "event: content_block_delta\n"
            'data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta",'
            '"text":"lo"}\n\n',
        ]
    )
    captured_requests, transport = _make_stream_transport(payload)
    adapter = unified_llm.AnthropicAdapter(
        api_key="stream-key",
        transport=transport,
    )
    request = unified_llm.Request(
        model="claude-sonnet-4-5",
        messages=[unified_llm.Message.user("hello")],
    )

    with caplog.at_level(logging.ERROR, logger="unified_llm.provider_utils.anthropic"):
        events = [event async for event in adapter.stream(request)]

    captured = capsys.readouterr()
    accumulator = unified_llm.StreamAccumulator.from_events(events)

    assert len(captured_requests) == 1
    assert [event.type for event in events] == [
        unified_llm.StreamEventType.STREAM_START,
        unified_llm.StreamEventType.TEXT_START,
        unified_llm.StreamEventType.TEXT_DELTA,
        unified_llm.StreamEventType.ERROR,
    ]
    assert events[-1].error is not None
    assert events[-1].error.provider == "anthropic"
    assert events[-1].error.message == "failed to normalize Anthropic stream event"
    assert accumulator.response.text == "Hel"
    assert accumulator.response.finish_reason.reason == "error"
    assert accumulator.finish_event is not None
    assert accumulator.finish_event.type == unified_llm.StreamEventType.ERROR
    assert captured.out == ""
    assert captured.err == ""
    assert any(
        "Anthropic stream event content_block_delta payload is not a JSON object"
        in record.message
        for record in caplog.records
    )


@pytest.mark.asyncio
async def test_anthropic_adapter_emits_provider_events_for_unhandled_stream_events(
) -> None:
    payload = "".join(
        [
            _sse_event(
                "message_start",
                {
                    "type": "message_start",
                    "message": {
                        "id": "msg_provider_event",
                        "type": "message",
                        "role": "assistant",
                        "model": "claude-sonnet-4-5",
                        "content": [],
                        "stop_reason": None,
                        "stop_sequence": None,
                        "usage": {
                            "input_tokens": 1,
                            "output_tokens": 0,
                        },
                    },
                },
            ),
            _sse_event(
                "ping",
                {
                    "type": "ping",
                    "detail": "keepalive",
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
    captured_requests, transport = _make_stream_transport(payload)
    adapter = unified_llm.AnthropicAdapter(
        api_key="stream-key",
        transport=transport,
    )
    request = unified_llm.Request(
        model="claude-sonnet-4-5",
        messages=[unified_llm.Message.user("hello")],
    )

    events = [event async for event in adapter.stream(request)]

    assert len(captured_requests) == 1
    assert [event.type for event in events] == [
        unified_llm.StreamEventType.STREAM_START,
        unified_llm.StreamEventType.PROVIDER_EVENT,
        unified_llm.StreamEventType.FINISH,
    ]
    assert events[1].raw == {
        "type": "ping",
        "detail": "keepalive",
    }
    finish_event = events[-1]
    assert finish_event.response is not None
    assert finish_event.response.raw == [
        {
            "type": "message_start",
            "message": {
                "id": "msg_provider_event",
                "type": "message",
                "role": "assistant",
                "model": "claude-sonnet-4-5",
                "content": [],
                "stop_reason": None,
                "stop_sequence": None,
                "usage": {
                    "input_tokens": 1,
                    "output_tokens": 0,
                },
            },
        },
        {
            "type": "ping",
            "detail": "keepalive",
        },
        {
            "type": "message_stop",
        },
    ]


@pytest.mark.asyncio
async def test_anthropic_adapter_uses_only_anthropic_provider_options_for_cache_and_thinking(
) -> None:
    response_body = {
        "id": "msg_options",
        "type": "message",
        "role": "assistant",
        "model": "claude-sonnet-4-5",
        "content": [
            {
                "type": "thinking",
                "thinking": "reasoning",
                "signature": "sig-123",
            },
            {
                "type": "redacted_thinking",
                "data": "  opaque-123  ",
            },
            {
                "type": "text",
                "text": "Done",
            },
        ],
        "stop_reason": "end_turn",
        "usage": {
            "input_tokens": 2,
            "output_tokens": 3,
        },
    }
    captured_requests, transport = _make_complete_transport(
        response_body,
        headers={
            "anthropic-ratelimit-requests-remaining": "7",
            "anthropic-ratelimit-requests-limit": "10",
            "anthropic-ratelimit-tokens-remaining": "99",
            "anthropic-ratelimit-tokens-limit": "200",
            "anthropic-ratelimit-requests-reset": "2026-04-21T00:00:00Z",
        },
    )
    adapter = unified_llm.AnthropicAdapter(
        api_key="options-key",
        transport=transport,
    )
    request = unified_llm.Request(
        model="claude-sonnet-4-5",
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
            unified_llm.Message.user("cached user context"),
            unified_llm.Message.assistant("cached assistant context"),
            unified_llm.Message.user("hello"),
        ],
        tools=[
            unified_llm.Tool.passive(
                "lookup",
                "Lookup a fact",
                {
                    "type": "object",
                    "properties": {
                        "query": {
                            "type": "string",
                        }
                    },
                    "required": ["query"],
                    "additionalProperties": False,
                },
            ),
            unified_llm.Tool.passive(
                "summarize",
                "Summarize content",
                {
                    "type": "object",
                    "properties": {
                        "text": {
                            "type": "string",
                        }
                    },
                    "required": ["text"],
                    "additionalProperties": False,
                },
            ),
        ],
        provider_options={
            "openai": {
                "beta_headers": ["ignored-openai"],
                "cache_control": {"type": "ephemeral"},
                "thinking": {"type": "enabled"},
            },
            "gemini": {
                "beta_headers": ["ignored-gemini"],
                "cache_control": {"type": "ephemeral"},
            },
            "anthropic": {
                "beta_headers": [
                    "alpha-beta",
                    "prompt-caching-2024-07-31",
                    "alpha-beta",
                ],
                "thinking": {
                    "type": "enabled",
                    "budget_tokens": 1234,
                },
                "output_config": {
                    "effort": "high",
                },
                "cache_control": {
                    "type": "ephemeral",
                    "ttl": "1h",
                },
                "auto_cache": False,
            },
        },
    )

    response = await adapter.complete(request)

    assert len(captured_requests) == 1

    sent_request = captured_requests[0]
    body = _request_json(sent_request)

    assert sent_request.headers["anthropic-beta"] == (
        "alpha-beta,prompt-caching-2024-07-31"
    )
    assert body["thinking"] == {
        "type": "enabled",
        "budget_tokens": 1234,
    }
    assert body["output_config"] == {
        "effort": "high",
    }
    assert body["system"] == [
        {
            "type": "text",
            "text": "system instructions\n\ndeveloper instructions",
            "cache_control": {
                "type": "ephemeral",
                "ttl": "1h",
            },
        }
    ]
    assert body["tools"] == [
        {
            "name": "lookup",
            "description": "Lookup a fact",
            "input_schema": {
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                    }
                },
                "required": ["query"],
                "additionalProperties": False,
            },
        },
        {
            "name": "summarize",
            "description": "Summarize content",
            "input_schema": {
                "type": "object",
                "properties": {
                    "text": {
                        "type": "string",
                    }
                },
                "required": ["text"],
                "additionalProperties": False,
            },
            "cache_control": {
                "type": "ephemeral",
                "ttl": "1h",
            },
        },
    ]
    assert "beta_headers" not in body
    assert body["messages"] == [
        {
            "role": "user",
            "content": [
                {
                    "type": "text",
                    "text": "cached user context",
                }
            ],
        },
        {
            "role": "assistant",
            "content": [
                {
                    "type": "text",
                    "text": "cached assistant context",
                    "cache_control": {
                        "type": "ephemeral",
                        "ttl": "1h",
                    },
                }
            ],
        },
        {
            "role": "user",
            "content": [
                {
                    "type": "text",
                    "text": "hello",
                }
            ],
        },
    ]
    assert "cache_control" not in body
    assert response.provider == "anthropic"
    assert response.text == "Done"
    assert [part.kind for part in response.message.content] == [
        unified_llm.ContentKind.THINKING,
        unified_llm.ContentKind.REDACTED_THINKING,
        unified_llm.ContentKind.TEXT,
    ]
    assert response.message.content[0].thinking is not None
    assert response.message.content[0].thinking.signature == "sig-123"
    assert response.message.content[1].thinking is not None
    assert response.message.content[1].thinking.redacted is True
    assert response.message.content[1].thinking.text == "  opaque-123  "
    assert response.usage.reasoning_tokens is not None
    assert response.usage.reasoning_tokens > 0
    assert response.rate_limit == unified_llm.RateLimitInfo(
        requests_remaining=7,
        requests_limit=10,
        tokens_remaining=99,
        tokens_limit=200,
        reset_at="2026-04-21T00:00:00Z",
    )
    assert response.raw == response_body


@pytest.mark.asyncio
async def test_anthropic_adapter_normalizes_non_streaming_tool_calls_thinking_and_redacted_thinking(
) -> None:
    response_body = {
        "id": "msg_response_tool_calls",
        "type": "message",
        "role": "assistant",
        "model": "claude-sonnet-4-5",
        "content": [
            {
                "type": "thinking",
                "thinking": "reasoning ",
                "signature": "sig-123",
            },
            {
                "type": "redacted_thinking",
                "data": "opaque-123",
            },
            {
                "type": "tool_use",
                "id": "call_123",
                "name": "lookup",
                "input": {
                    "location": "Paris",
                },
            },
            {
                "type": "text",
                "text": "Done",
            },
        ],
        "stop_reason": "tool_use",
        "usage": {
            "input_tokens": 4,
            "output_tokens": 6,
            "cache_read_input_tokens": 2,
            "cache_creation_input_tokens": 3,
        },
    }
    captured_requests, transport = _make_complete_transport(
        response_body,
        headers={
            "anthropic-ratelimit-requests-remaining": "9",
            "anthropic-ratelimit-requests-limit": "10",
            "anthropic-ratelimit-tokens-remaining": "99",
            "anthropic-ratelimit-tokens-limit": "100",
            "anthropic-ratelimit-requests-reset": "2026-04-21T00:00:00Z",
        },
    )
    adapter = unified_llm.AnthropicAdapter(
        api_key="response-key",
        transport=transport,
    )
    request = unified_llm.Request(
        model="claude-sonnet-4-5",
        messages=[unified_llm.Message.user("hello")],
        provider_options={
            "anthropic": {
                "auto_cache": False,
            }
        },
    )

    response = await adapter.complete(request)

    assert len(captured_requests) == 1
    assert response.provider == "anthropic"
    assert response.id == "msg_response_tool_calls"
    assert response.model == "claude-sonnet-4-5"
    assert response.text == "Done"
    assert response.reasoning == "reasoning opaque-123"
    assert response.finish_reason.reason == "tool_calls"
    assert response.finish_reason.raw == "tool_use"
    assert response.usage.input_tokens == 4
    assert response.usage.output_tokens == 6
    assert response.usage.total_tokens == 10
    assert response.usage.cache_read_tokens == 2
    assert response.usage.cache_write_tokens == 3
    assert response.usage.reasoning_tokens is not None
    assert response.usage.reasoning_tokens > 0
    assert response.rate_limit == unified_llm.RateLimitInfo(
        requests_remaining=9,
        requests_limit=10,
        tokens_remaining=99,
        tokens_limit=100,
        reset_at="2026-04-21T00:00:00Z",
    )
    assert response.raw == response_body
    assert [part.kind for part in response.message.content] == [
        unified_llm.ContentKind.THINKING,
        unified_llm.ContentKind.REDACTED_THINKING,
        unified_llm.ContentKind.TOOL_CALL,
        unified_llm.ContentKind.TEXT,
    ]
    assert response.message.content[0].thinking is not None
    assert response.message.content[0].thinking.signature == "sig-123"
    assert response.message.content[1].thinking is not None
    assert response.message.content[1].thinking.redacted is True
    assert response.message.content[1].thinking.text == "opaque-123"
    assert response.tool_calls[0].id == "call_123"
    assert response.tool_calls[0].name == "lookup"
    assert response.tool_calls[0].arguments == {"location": "Paris"}
    assert response.tool_calls[0].raw_arguments == '{"location":"Paris"}'


@pytest.mark.asyncio
async def test_anthropic_adapter_translates_http_error_bodies_and_retry_after(
    capsys: pytest.CaptureFixture[str],
) -> None:
    response_body = {
        "type": "error",
        "error": {
            "type": "rate_limit_error",
            "message": "slow down",
        },
        "request_id": "req_123",
    }
    captured_requests, transport = _make_complete_transport(
        response_body,
        headers={"Retry-After": "7"},
        status_code=429,
    )
    adapter = unified_llm.AnthropicAdapter(
        api_key="error-key",
        transport=transport,
    )
    request = unified_llm.Request(
        model="claude-sonnet-4-5",
        messages=[unified_llm.Message.user("hello")],
    )

    with pytest.raises(unified_llm.RateLimitError) as exc_info:
        await adapter.complete(request)

    captured = capsys.readouterr()
    assert captured.out == ""
    assert captured.err == ""
    assert len(captured_requests) == 1

    error = exc_info.value
    assert error.message == "slow down"
    assert error.provider == "anthropic"
    assert error.status_code == 429
    assert error.error_code == "rate_limit_error"
    assert error.retry_after == 7.0
    assert error.raw == response_body
    assert error.cause is None


@pytest.mark.asyncio
async def test_anthropic_adapter_preserves_httpx_timeout_causes(
    capsys: pytest.CaptureFixture[str],
) -> None:
    async def handler(request: httpx.Request) -> httpx.Response:
        raise httpx.ReadTimeout("request timed out", request=request)

    transport = httpx.MockTransport(handler)
    adapter = unified_llm.AnthropicAdapter(
        api_key="timeout-key",
        transport=transport,
    )
    request = unified_llm.Request(
        model="claude-sonnet-4-5",
        messages=[unified_llm.Message.user("hello")],
    )

    with pytest.raises(unified_llm.RequestTimeoutError) as exc_info:
        await adapter.complete(request)

    captured = capsys.readouterr()
    assert captured.out == ""
    assert captured.err == ""

    error = exc_info.value
    assert error.message == "request timed out"
    assert error.provider == "anthropic"
    assert error.cause is not None
    assert isinstance(error.cause, httpx.ReadTimeout)


def test_anthropic_adapter_supports_the_standard_tool_choice_modes() -> None:
    adapter = unified_llm.AnthropicAdapter(api_key="tool-choice-key", client=object())

    assert adapter.supports_tool_choice("auto") is True
    assert adapter.supports_tool_choice("none") is True
    assert adapter.supports_tool_choice("required") is True
    assert adapter.supports_tool_choice("named") is True
    assert adapter.supports_tool_choice("unsupported") is False


@pytest.mark.asyncio
@pytest.mark.parametrize(
    ("tool_choice", "expected_tool_choice", "expect_tools"),
    [
        (
            unified_llm.ToolChoice.auto(),
            {"type": "auto"},
            True,
        ),
        (
            unified_llm.ToolChoice.required(),
            {"type": "any"},
            True,
        ),
        (
            unified_llm.ToolChoice.named("lookup"),
            {"type": "tool", "name": "lookup"},
            True,
        ),
        (
            unified_llm.ToolChoice.none(),
            None,
            False,
        ),
    ],
)
async def test_anthropic_adapter_translates_tool_choice_modes(
    tool_choice: unified_llm.ToolChoice,
    expected_tool_choice: dict[str, object] | None,
    expect_tools: bool,
) -> None:
    response_body = {
        "id": "msg_tool_choice",
        "type": "message",
        "role": "assistant",
        "model": "claude-sonnet-4-5",
        "content": [
            {
                "type": "text",
                "text": "ok",
            }
        ],
        "stop_reason": "end_turn",
        "usage": {
            "input_tokens": 1,
            "output_tokens": 1,
        },
    }
    captured_requests, transport = _make_complete_transport(response_body)
    adapter = unified_llm.AnthropicAdapter(
        api_key="tool-choice-key",
        transport=transport,
    )
    request = unified_llm.Request(
        model="claude-sonnet-4-5",
        messages=[unified_llm.Message.user("hello")],
        tools=[
            unified_llm.Tool.passive(
                "lookup",
                "Lookup a fact",
                {
                    "type": "object",
                    "properties": {
                        "query": {
                            "type": "string",
                        }
                    },
                    "required": ["query"],
                    "additionalProperties": False,
                },
            ),
        ],
        tool_choice=tool_choice,
    )

    await adapter.complete(request)

    assert len(captured_requests) == 1
    body = _request_json(captured_requests[0])

    if expect_tools:
        assert body["tools"] == [
            {
                "name": "lookup",
                "description": "Lookup a fact",
                "input_schema": {
                    "type": "object",
                    "properties": {
                        "query": {
                            "type": "string",
                        }
                    },
                    "required": ["query"],
                    "additionalProperties": False,
                },
                "cache_control": {"type": "ephemeral"},
            }
        ]
    else:
        assert "tools" not in body

    if expected_tool_choice is None:
        assert "tool_choice" not in body
    else:
        assert body["tool_choice"] == expected_tool_choice

    assert body["messages"] == [
        {
            "role": "user",
            "content": [
                {
                    "type": "text",
                    "text": "hello",
                }
            ],
        }
    ]

    if expect_tools:
        assert captured_requests[0].headers["anthropic-beta"] == "prompt-caching-2024-07-31"
    else:
        assert "anthropic-beta" not in captured_requests[0].headers


@pytest.mark.asyncio
@pytest.mark.parametrize(
    ("tool_choice", "tools", "expected_fragment"),
    [
        (
            unified_llm.ToolChoice.required(),
            [],
            "requires at least one tool",
        ),
        (
            unified_llm.ToolChoice.named("lookup"),
            [
                unified_llm.Tool.passive(
                    "other",
                    "Other tool",
                    {
                        "type": "object",
                        "properties": {
                            "query": {
                                "type": "string",
                            }
                        },
                        "required": ["query"],
                    },
                ),
            ],
            "requires a matching tool",
        ),
    ],
)
async def test_anthropic_adapter_raises_unsupported_tool_choice_for_unrepresentable_requests(
    tool_choice: unified_llm.ToolChoice,
    tools: list[unified_llm.Tool],
    expected_fragment: str,
    caplog: pytest.LogCaptureFixture,
    capsys: pytest.CaptureFixture[str],
) -> None:
    response_body = {
        "id": "msg_tool_choice_invalid",
        "type": "message",
        "role": "assistant",
        "model": "claude-sonnet-4-5",
        "content": [
            {
                "type": "text",
                "text": "ok",
            }
        ],
        "stop_reason": "end_turn",
        "usage": {
            "input_tokens": 1,
            "output_tokens": 1,
        },
    }
    captured_requests, transport = _make_complete_transport(response_body)
    adapter = unified_llm.AnthropicAdapter(
        api_key="tool-choice-key",
        transport=transport,
    )
    request = unified_llm.Request(
        model="claude-sonnet-4-5",
        messages=[unified_llm.Message.user("hello")],
        tools=tools,
        tool_choice=tool_choice,
    )

    with caplog.at_level(logging.WARNING, logger="unified_llm.provider_utils.anthropic"):
        with pytest.raises(unified_llm.UnsupportedToolChoiceError):
            await adapter.complete(request)

    captured = capsys.readouterr()
    assert captured.out == ""
    assert captured.err == ""
    assert captured_requests == []
    assert any(expected_fragment in record.message for record in caplog.records)


@pytest.mark.asyncio
async def test_anthropic_adapter_uses_response_format_as_a_schema_instruction(
    capsys: pytest.CaptureFixture[str],
) -> None:
    schema = {
        "type": "object",
        "properties": {
            "name": {"type": "string"},
            "age": {"type": "integer"},
        },
        "required": ["name", "age"],
        "additionalProperties": False,
    }
    response_body = {
        "id": "msg_response_format",
        "type": "message",
        "role": "assistant",
        "model": "claude-sonnet-4-5",
        "content": [
            {
                "type": "text",
                "text": '{"name":"Alice","age":30}',
            }
        ],
        "stop_reason": "end_turn",
        "usage": {
            "input_tokens": 1,
            "output_tokens": 1,
        },
    }
    captured_requests, transport = _make_complete_transport(response_body)
    adapter = unified_llm.AnthropicAdapter(
        api_key="schema-key",
        transport=transport,
    )
    request = unified_llm.Request(
        model="claude-sonnet-4-5",
        messages=[unified_llm.Message.user("extract a person")],
        response_format=unified_llm.ResponseFormat(
            type="json_schema",
            json_schema=schema,
            strict=True,
        ),
        provider_options={
            "anthropic": {
                "auto_cache": False,
            }
        },
    )

    await adapter.complete(request)

    captured = capsys.readouterr()
    assert captured.out == ""
    assert captured.err == ""
    assert len(captured_requests) == 1

    body = _request_json(captured_requests[0])

    assert "response_format" not in body
    assert "tools" not in body
    assert body["system"] == _anthropic_structured_output_instruction(schema)


@pytest.mark.asyncio
async def test_anthropic_adapter_uses_provider_options_structured_output_schema(
    capsys: pytest.CaptureFixture[str],
) -> None:
    schema = {
        "type": "object",
        "properties": {
            "name": {"type": "string"},
            "age": {"type": "integer"},
        },
        "required": ["name", "age"],
        "additionalProperties": False,
    }
    response_body = {
        "id": "msg_structured_output",
        "type": "message",
        "role": "assistant",
        "model": "claude-sonnet-4-5",
        "content": [
            {
                "type": "text",
                "text": '{"name":"Alice","age":30}',
            }
        ],
        "stop_reason": "end_turn",
        "usage": {
            "input_tokens": 1,
            "output_tokens": 1,
        },
    }
    captured_requests, transport = _make_complete_transport(response_body)
    adapter = unified_llm.AnthropicAdapter(
        api_key="schema-key",
        transport=transport,
    )
    request = unified_llm.Request(
        model="claude-sonnet-4-5",
        messages=[unified_llm.Message.user("extract a person")],
        provider_options={
            "anthropic": {
                "auto_cache": False,
                "structured_output": {
                    "provider": "anthropic",
                    "strategy": "schema-instruction",
                    "schema": schema,
                },
            }
        },
    )

    await adapter.complete(request)

    captured = capsys.readouterr()
    assert captured.out == ""
    assert captured.err == ""
    assert len(captured_requests) == 1

    body = _request_json(captured_requests[0])

    assert "response_format" not in body
    assert "tools" not in body
    assert body["system"] == _anthropic_structured_output_instruction(schema)


@pytest.mark.asyncio
@pytest.mark.parametrize(
    ("strategy", "expected_fragment"),
    [
        ("forced-tool", "forced-tool"),
        ("experimental-mode", "experimental-mode"),
    ],
)
async def test_anthropic_adapter_rejects_unsupported_structured_output_strategies(
    strategy: str,
    expected_fragment: str,
    caplog: pytest.LogCaptureFixture,
    capsys: pytest.CaptureFixture[str],
) -> None:
    schema = {
        "type": "object",
        "properties": {
            "name": {"type": "string"},
            "age": {"type": "integer"},
        },
        "required": ["name", "age"],
        "additionalProperties": False,
    }
    response_body = {
        "id": "msg_structured_output_strategy",
        "type": "message",
        "role": "assistant",
        "model": "claude-sonnet-4-5",
        "content": [
            {
                "type": "text",
                "text": '{"name":"Alice","age":30}',
            }
        ],
        "stop_reason": "end_turn",
        "usage": {
            "input_tokens": 1,
            "output_tokens": 1,
        },
    }
    captured_requests, transport = _make_complete_transport(response_body)
    adapter = unified_llm.AnthropicAdapter(
        api_key="schema-key",
        transport=transport,
    )
    request = unified_llm.Request(
        model="claude-sonnet-4-5",
        messages=[unified_llm.Message.user("extract a person")],
        response_format=unified_llm.ResponseFormat(
            type="json_schema",
            json_schema=schema,
            strict=True,
        ),
        provider_options={
            "anthropic": {
                "auto_cache": False,
                "structured_output": {
                    "provider": "anthropic",
                    "strategy": strategy,
                    "schema": schema,
                },
            }
        },
    )

    with caplog.at_level(logging.WARNING, logger="unified_llm.provider_utils.anthropic"):
        with pytest.raises(unified_llm.InvalidRequestError, match="structured_output strategy"):
            await adapter.complete(request)

    captured = capsys.readouterr()
    assert captured.out == ""
    assert captured.err == ""
    assert captured_requests == []
    assert any(expected_fragment in record.message for record in caplog.records)


@pytest.mark.asyncio
async def test_anthropic_adapter_uses_provider_options_system_instruction_for_structured_output(
    capsys: pytest.CaptureFixture[str],
) -> None:
    schema = {
        "type": "object",
        "properties": {
            "name": {"type": "string"},
            "age": {"type": "integer"},
        },
        "required": ["name", "age"],
        "additionalProperties": False,
    }
    response_body = {
        "id": "msg_provider_instruction",
        "type": "message",
        "role": "assistant",
        "model": "claude-sonnet-4-5",
        "content": [
            {
                "type": "text",
                "text": '{"name":"Alice","age":30}',
            }
        ],
        "stop_reason": "end_turn",
        "usage": {
            "input_tokens": 1,
            "output_tokens": 1,
        },
    }
    captured_requests, transport = _make_complete_transport(response_body)
    adapter = unified_llm.AnthropicAdapter(
        api_key="schema-key",
        transport=transport,
    )
    request = unified_llm.Request(
        model="claude-sonnet-4-5",
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
            unified_llm.Message.user("extract a person"),
        ],
        response_format=unified_llm.ResponseFormat(
            type="json_schema",
            json_schema=schema,
            strict=True,
        ),
        provider_options={
            "anthropic": {
                "auto_cache": False,
                "system_instruction": "Return only JSON that matches the provided schema.",
            }
        },
    )

    await adapter.complete(request)

    captured = capsys.readouterr()
    assert captured.out == ""
    assert captured.err == ""
    assert len(captured_requests) == 1

    body = _request_json(captured_requests[0])

    assert "response_format" not in body
    assert "tools" not in body
    assert body["system"] == (
        "system instructions\n\ndeveloper instructions\n\n"
        "Return only JSON that matches the provided schema.\n\n"
        f"{_anthropic_schema_block(schema)}"
    )


@pytest.mark.asyncio
async def test_anthropic_adapter_rejects_unsupported_response_format_types(
    caplog: pytest.LogCaptureFixture,
    capsys: pytest.CaptureFixture[str],
) -> None:
    response_body = {
        "id": "msg_invalid_response_format",
        "type": "message",
        "role": "assistant",
        "model": "claude-sonnet-4-5",
        "content": [
            {
                "type": "text",
                "text": "ignored",
            }
        ],
        "stop_reason": "end_turn",
        "usage": {
            "input_tokens": 1,
            "output_tokens": 1,
        },
    }
    captured_requests, transport = _make_complete_transport(response_body)
    adapter = unified_llm.AnthropicAdapter(
        api_key="schema-key",
        transport=transport,
    )
    request = unified_llm.Request(
        model="claude-sonnet-4-5",
        messages=[unified_llm.Message.user("extract a person")],
        response_format=unified_llm.ResponseFormat(type="json_object"),
    )

    with caplog.at_level(logging.WARNING, logger="unified_llm.provider_utils.anthropic"):
        with pytest.raises(unified_llm.InvalidRequestError, match="json_schema"):
            await adapter.complete(request)

    captured = capsys.readouterr()
    assert captured.out == ""
    assert captured.err == ""
    assert captured_requests == []
    assert any(
        "response_format only supports json_schema" in record.message
        for record in caplog.records
    )


@pytest.mark.asyncio
async def test_anthropic_adapter_can_disable_automatic_cache_injection() -> None:
    response_body = {
        "id": "msg_no_cache",
        "type": "message",
        "role": "assistant",
        "model": "claude-sonnet-4-5",
        "content": [
            {
                "type": "text",
                "text": "No cache",
            }
        ],
        "stop_reason": "end_turn",
        "usage": {
            "input_tokens": 1,
            "output_tokens": 1,
        },
    }
    captured_requests, transport = _make_complete_transport(response_body)
    adapter = unified_llm.AnthropicAdapter(
        api_key="cache-key",
        transport=transport,
    )
    request = unified_llm.Request(
        model="claude-sonnet-4-5",
        messages=[unified_llm.Message.user("hello")],
        provider_options={
            "anthropic": {
                "auto_cache": False,
            }
        },
    )

    await adapter.complete(request)

    assert len(captured_requests) == 1

    sent_request = captured_requests[0]
    body = _request_json(sent_request)

    assert "anthropic-beta" not in sent_request.headers
    assert "cache_control" not in body
    assert body["messages"] == [
        {
            "role": "user",
            "content": [
                {
                    "type": "text",
                    "text": "hello",
                }
            ],
        }
    ]


@pytest.mark.asyncio
@pytest.mark.parametrize(
    ("anthropic_options", "expected_fragment"),
    [
        ({"beta_headers": {"oops": "bad"}}, "beta_headers"),
        ({"cache_control": ["bad"]}, "cache_control"),
        ({"auto_cache": "bad"}, "auto_cache"),
    ],
)
async def test_anthropic_adapter_reports_invalid_anthropic_provider_option_shapes(
    anthropic_options: dict[str, object],
    expected_fragment: str,
    caplog: pytest.LogCaptureFixture,
    capsys: pytest.CaptureFixture[str],
) -> None:
    response_body = {
        "id": "msg_invalid",
        "type": "message",
        "role": "assistant",
        "model": "claude-sonnet-4-5",
        "content": [
            {
                "type": "text",
                "text": "ignored",
            }
        ],
        "stop_reason": "end_turn",
        "usage": {
            "input_tokens": 1,
            "output_tokens": 1,
        },
    }
    captured_requests, transport = _make_complete_transport(response_body)
    adapter = unified_llm.AnthropicAdapter(
        api_key="invalid-key",
        transport=transport,
    )
    request = unified_llm.Request(
        model="claude-sonnet-4-5",
        messages=[unified_llm.Message.user("hello")],
        provider_options={
            "anthropic": anthropic_options,
        },
    )

    with caplog.at_level(logging.WARNING, logger="unified_llm.provider_utils.anthropic"):
        with pytest.raises(unified_llm.InvalidRequestError):
            await adapter.complete(request)

    captured = capsys.readouterr()
    assert captured.out == ""
    assert captured.err == ""
    assert captured_requests == []
    assert any(
        expected_fragment in record.message
        for record in caplog.records
    )


@pytest.mark.asyncio
async def test_anthropic_adapter_close_respects_client_ownership_and_logs_failures(
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
    borrowed_adapter = unified_llm.AnthropicAdapter(
        api_key="close-key",
        client=borrowed,
    )
    await borrowed_adapter.close()
    assert borrowed.closed is False

    owned = _CloseRecorder()
    owned_adapter = unified_llm.AnthropicAdapter(
        api_key="close-key",
        client=owned,
        owns_client=True,
    )
    await owned_adapter.close()
    assert owned.closed is True

    failing = _CloseRecorder(error=RuntimeError("boom"))
    failing_adapter = unified_llm.AnthropicAdapter(
        api_key="close-key",
        client=failing,
        owns_client=True,
    )

    with caplog.at_level(logging.ERROR, logger="unified_llm.adapters.anthropic"):
        await failing_adapter.close()

    captured = capsys.readouterr()
    assert captured.out == ""
    assert captured.err == ""
    assert failing.closed is True
    assert any(
        record.name == "unified_llm.adapters.anthropic"
        and "Unexpected error closing Anthropic HTTP client" in record.message
        for record in caplog.records
    )
