from __future__ import annotations

import base64
import copy
import json
import os
import struct
import zlib
from pathlib import Path
from typing import Any

import httpx
import pytest

import unified_llm

PROVIDERS = ("openai", "anthropic", "gemini")
REQUEST_TEXT = "hello"
RESPONSE_TEXT = "Hello world"
IMAGE_URL = "https://example.test/image.png"
TOOL_RESULT_CONTENT = {"temperature": 72, "unit": "F"}


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
    content_type: str = "text/event-stream",
) -> tuple[list[httpx.Request], httpx.MockTransport]:
    captured_requests: list[httpx.Request] = []

    async def handler(request: httpx.Request) -> httpx.Response:
        captured_requests.append(request)
        return httpx.Response(
            status_code,
            headers={"content-type": content_type, **dict(headers or {})},
            content=payload.encode("utf-8"),
        )

    return captured_requests, httpx.MockTransport(handler)


def _make_adapter(provider: str, transport: httpx.MockTransport) -> Any:
    if provider == "openai":
        return unified_llm.OpenAIAdapter(api_key="openai-key", transport=transport)
    if provider == "anthropic":
        return unified_llm.AnthropicAdapter(api_key="anthropic-key", transport=transport)
    if provider == "gemini":
        return unified_llm.GeminiAdapter(api_key="gemini-key", transport=transport)
    raise AssertionError(f"Unsupported provider {provider!r}")


def _provider_model(provider: str) -> str:
    if provider == "openai":
        return "gpt-5.2"
    if provider == "anthropic":
        return "claude-sonnet-4-5"
    if provider == "gemini":
        return "gemini-3.1-pro-preview"
    raise AssertionError(f"Unsupported provider {provider!r}")


def _mixed_provider_options(provider: str) -> dict[str, object]:
    if provider == "openai":
        return {
            "openai": {"reasoning": {"effort": "high"}},
            "anthropic": {
                "auto_cache": False,
                "beta_headers": ["ignored-beta"],
            },
            "gemini": {"temperature": 0.7},
        }
    if provider == "anthropic":
        return {
            "anthropic": {
                "auto_cache": False,
                "beta_headers": ["custom-beta"],
            },
            "openai": {"reasoning": {"effort": "high"}},
            "gemini": {"temperature": 0.7},
        }
    if provider == "gemini":
        return {
            "gemini": {"temperature": 0.2, "candidateCount": 2},
            "openai": {"reasoning": {"effort": "high"}},
            "anthropic": {
                "auto_cache": False,
                "beta_headers": ["ignored-beta"],
            },
        }
    raise AssertionError(f"Unsupported provider {provider!r}")


def _cache_suppression_options() -> dict[str, object]:
    return {"anthropic": {"auto_cache": False}}


def _text_completion_body(provider: str) -> dict[str, object]:
    if provider == "openai":
        return {
            "id": "resp_text_openai",
            "model": "gpt-5.2",
            "status": "completed",
            "output_text": RESPONSE_TEXT,
            "usage": {
                "input_tokens": 12,
                "output_tokens": 34,
                "output_tokens_details": {"reasoning_tokens": 5},
                "input_tokens_details": {"cached_tokens": 4},
            },
        }
    if provider == "anthropic":
        return {
            "id": "msg_text_anthropic",
            "type": "message",
            "role": "assistant",
            "model": "claude-sonnet-4-5",
            "content": [
                {
                    "type": "text",
                    "text": RESPONSE_TEXT,
                }
            ],
            "stop_reason": "end_turn",
            "usage": {
                "input_tokens": 11,
                "output_tokens": 13,
                "cache_read_input_tokens": 5,
                "cache_creation_input_tokens": 2,
            },
        }
    if provider == "gemini":
        return {
            "responseId": "resp_text_gemini",
            "modelVersion": "gemini-3.1-pro-preview",
            "candidates": [
                {
                    "content": {
                        "role": "model",
                        "parts": [
                            {
                                "text": RESPONSE_TEXT,
                            }
                        ],
                    },
                    "finishReason": "STOP",
                }
            ],
            "usageMetadata": {
                "promptTokenCount": 12,
                "candidatesTokenCount": 34,
                "totalTokenCount": 46,
                "thoughtsTokenCount": 5,
                "cachedContentTokenCount": 4,
            },
        }
    raise AssertionError(f"Unsupported provider {provider!r}")


def _stream_payload(provider: str) -> str:
    if provider == "openai":
        return "".join(
            [
                _sse_event(
                    "response.created",
                    {
                        "type": "response.created",
                        "response": {"id": "resp_stream_openai", "model": "gpt-5.2"},
                    },
                ),
                _sse_event(
                    "response.output_text.delta",
                    {
                        "type": "response.output_text.delta",
                        "item_id": "text_1",
                        "delta": "Hel",
                    },
                ),
                _sse_event(
                    "response.output_text.delta",
                    {
                        "type": "response.output_text.delta",
                        "item_id": "text_1",
                        "delta": "lo",
                    },
                ),
                _sse_event(
                    "response.output_item.done",
                    {
                        "type": "response.output_item.done",
                        "item": {
                            "type": "output_text",
                            "item_id": "text_1",
                            "text": "Hello",
                        },
                    },
                ),
                _sse_event(
                    "response.completed",
                    {
                        "type": "response.completed",
                        "response": {
                            "id": "resp_stream_openai",
                            "model": "gpt-5.2",
                            "status": "completed",
                            "usage": {
                                "input_tokens": 12,
                                "output_tokens": 34,
                                "output_tokens_details": {"reasoning_tokens": 5},
                                "input_tokens_details": {"cached_tokens": 4},
                            },
                        },
                    },
                ),
            ]
        )
    if provider == "anthropic":
        return "".join(
            [
                _sse_event(
                    "message_start",
                    {
                        "type": "message_start",
                        "message": {
                            "id": "msg_stream_anthropic",
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
                            "text": "Hello",
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
                    "message_delta",
                    {
                        "type": "message_delta",
                        "delta": {
                            "stop_reason": "end_turn",
                            "stop_sequence": None,
                        },
                        "usage": {
                            "output_tokens": 13,
                            "cache_read_input_tokens": 5,
                            "cache_creation_input_tokens": 2,
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
    if provider == "gemini":
        first_chunk = {
            "responseId": "resp_stream_gemini",
            "modelVersion": "gemini-3.1-pro-preview",
            "candidates": [
                {
                    "content": {
                        "role": "model",
                        "parts": [
                            {
                                "text": "Hello",
                            }
                        ],
                    },
                }
            ],
        }
        second_chunk = {
            "responseId": "resp_stream_gemini",
            "modelVersion": "gemini-3.1-pro-preview",
            "candidates": [
                {
                    "content": {
                        "role": "model",
                        "parts": [
                            {
                                "text": "Hello world",
                            }
                        ],
                    },
                    "finishReason": "STOP",
                }
            ],
            "usageMetadata": {
                "promptTokenCount": 12,
                "candidatesTokenCount": 34,
                "totalTokenCount": 46,
                "thoughtsTokenCount": 5,
                "cachedContentTokenCount": 4,
            },
        }
        return (
            f"data: {json.dumps(first_chunk, separators=(',', ':'), sort_keys=True)}\n\n"
            f"data: {json.dumps(second_chunk, separators=(',', ':'), sort_keys=True)}\n\n"
        )
    raise AssertionError(f"Unsupported provider {provider!r}")


def _sse_event(event: str, payload: dict[str, object]) -> str:
    return (
        f"event: {event}\n"
        f"data: {json.dumps(payload, separators=(',', ':'), sort_keys=True)}\n\n"
    )


def _text_completion_request(provider: str) -> unified_llm.Request:
    return unified_llm.Request(
        model=_provider_model(provider),
        messages=[unified_llm.Message.user(REQUEST_TEXT)],
        provider_options=_mixed_provider_options(provider),
    )


def _image_request(provider: str, image_path: Path) -> unified_llm.Request:
    image_bytes = b"image-bytes"
    return unified_llm.Request(
        model=_provider_model(provider),
        messages=[
            unified_llm.Message.user(
                [
                    unified_llm.ContentPart(
                        kind=unified_llm.ContentKind.TEXT,
                        text="show me the image",
                    ),
                    unified_llm.ContentPart(
                        kind=unified_llm.ContentKind.IMAGE,
                        image=unified_llm.ImageData(url=IMAGE_URL),
                    ),
                    unified_llm.ContentPart(
                        kind=unified_llm.ContentKind.IMAGE,
                        image=unified_llm.ImageData(
                            data=image_bytes,
                            media_type="image/png",
                        ),
                    ),
                    unified_llm.ContentPart(
                        kind=unified_llm.ContentKind.IMAGE,
                        image=unified_llm.ImageData(url=str(image_path)),
                    ),
                ]
            )
        ],
        provider_options=_cache_suppression_options(),
    )


def _tool_round_trip_request(
    provider: str,
    response_message: unified_llm.Message,
    tool_call_id: str,
) -> unified_llm.Request:
    return unified_llm.Request(
        model=_provider_model(provider),
        messages=[
            unified_llm.Message.user(REQUEST_TEXT),
            response_message,
            unified_llm.Message.tool_result(
                tool_call_id,
                TOOL_RESULT_CONTENT,
            ),
        ],
        provider_options=_cache_suppression_options(),
    )


def _unsupported_media_part(
    content_kind: unified_llm.ContentKind,
) -> unified_llm.ContentPart:
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
    raise AssertionError(f"Unsupported media kind {content_kind!r}")


def _assert_text_completion_parity(
    provider: str,
    *,
    sent_request: httpx.Request,
    body: dict[str, object],
    response: unified_llm.Response,
    request: unified_llm.Request,
    original_provider_options: dict[str, Any],
) -> None:
    if provider == "openai":
        assert sent_request.method == "POST"
        assert sent_request.url.path == "/v1/responses"
        assert sent_request.headers["authorization"] == "Bearer openai-key"
        assert body["model"] == "gpt-5.2"
        assert body["input"] == [
            {
                "type": "message",
                "role": "user",
                "content": [
                    {
                        "type": "input_text",
                        "text": REQUEST_TEXT,
                    }
                ],
            }
        ]
        assert body["reasoning"] == {"effort": "high"}
        assert "beta_headers" not in body
        assert "generationConfig" not in body
        assert "messages" not in body
        assert request.provider_options == original_provider_options
        assert response.provider == "openai"
        assert response.id == "resp_text_openai"
        assert response.model == "gpt-5.2"
        assert response.text == RESPONSE_TEXT
        assert response.finish_reason.reason == "stop"
        assert response.finish_reason.raw == "completed"
        assert response.usage.input_tokens == 12
        assert response.usage.output_tokens == 34
        assert response.usage.total_tokens == 46
        assert response.usage.reasoning_tokens == 5
        assert response.usage.cache_read_tokens == 4
        assert response.raw == _text_completion_body(provider)
        return

    if provider == "anthropic":
        assert sent_request.method == "POST"
        assert sent_request.url.path == "/v1/messages"
        assert sent_request.headers["x-api-key"] == "anthropic-key"
        assert sent_request.headers["anthropic-beta"] == "custom-beta"
        assert body["model"] == "claude-sonnet-4-5"
        assert body["max_tokens"] == 4096
        assert body["messages"] == [
            {
                "role": "user",
                "content": [
                    {
                        "type": "text",
                        "text": REQUEST_TEXT,
                    }
                ],
            }
        ]
        assert "cache_control" not in body
        assert "reasoning" not in body
        assert "generationConfig" not in body
        assert request.provider_options == original_provider_options
        assert response.provider == "anthropic"
        assert response.id == "msg_text_anthropic"
        assert response.model == "claude-sonnet-4-5"
        assert response.text == RESPONSE_TEXT
        assert response.finish_reason.reason == "stop"
        assert response.finish_reason.raw == "end_turn"
        assert response.usage.input_tokens == 11
        assert response.usage.output_tokens == 13
        assert response.usage.total_tokens == 24
        assert response.usage.cache_read_tokens == 5
        assert response.usage.cache_write_tokens == 2
        assert response.raw == _text_completion_body(provider)
        return

    if provider == "gemini":
        assert sent_request.method == "POST"
        assert sent_request.url.path == "/v1beta/models/gemini-3.1-pro-preview:generateContent"
        assert sent_request.url.params["key"] == "gemini-key"
        assert "authorization" not in sent_request.headers
        assert body["contents"] == [
            {
                "role": "user",
                "parts": [
                    {
                        "text": REQUEST_TEXT,
                    }
                ],
            }
        ]
        assert body["generationConfig"] == {
            "candidateCount": 2,
            "temperature": 0.2,
        }
        assert "reasoning" not in body
        assert "beta_headers" not in body
        assert request.provider_options == original_provider_options
        assert response.provider == "gemini"
        assert response.id == "resp_text_gemini"
        assert response.model == "gemini-3.1-pro-preview"
        assert response.text == RESPONSE_TEXT
        assert response.finish_reason.reason == "stop"
        assert response.finish_reason.raw == "STOP"
        assert response.usage.input_tokens == 12
        assert response.usage.output_tokens == 34
        assert response.usage.total_tokens == 46
        assert response.usage.reasoning_tokens == 5
        assert response.usage.cache_read_tokens == 4
        assert response.raw == _text_completion_body(provider)
        return

    raise AssertionError(f"Unsupported provider {provider!r}")


def _assert_stream_parity(
    provider: str,
    *,
    sent_request: httpx.Request,
    body: dict[str, object],
    events: list[unified_llm.StreamEvent],
    response: unified_llm.Response,
) -> None:
    assert response.provider == provider

    if provider == "openai":
        assert [event.type for event in events] == [
            unified_llm.StreamEventType.STREAM_START,
            unified_llm.StreamEventType.TEXT_START,
            unified_llm.StreamEventType.TEXT_DELTA,
            unified_llm.StreamEventType.TEXT_DELTA,
            unified_llm.StreamEventType.TEXT_END,
            unified_llm.StreamEventType.FINISH,
        ]
        assert events[1].delta is None
        assert _text_delta_values(events) == ["Hel", "lo"]
        assert response.text == "Hello"
        assert response.finish_reason.reason == "stop"
        assert response.finish_reason.raw == "completed"
        assert response.usage.input_tokens == 12
        assert response.usage.output_tokens == 34
        assert response.usage.total_tokens == 46
        assert response.usage.reasoning_tokens == 5
        assert response.usage.cache_read_tokens == 4
        assert sent_request.method == "POST"
        assert sent_request.url.path == "/v1/responses"
        assert sent_request.headers["authorization"] == "Bearer openai-key"
        assert body["stream"] is True
        assert body["input"] == [
            {
                "type": "message",
                "role": "user",
                "content": [
                    {
                        "type": "input_text",
                        "text": REQUEST_TEXT,
                    }
                ],
            }
        ]
        return

    if provider == "anthropic":
        assert [event.type for event in events] == [
            unified_llm.StreamEventType.STREAM_START,
            unified_llm.StreamEventType.TEXT_START,
            unified_llm.StreamEventType.TEXT_DELTA,
            unified_llm.StreamEventType.TEXT_END,
            unified_llm.StreamEventType.FINISH,
        ]
        assert events[1].delta == "Hello"
        assert _text_delta_values(events) == [" world"]
        assert response.text == RESPONSE_TEXT
        assert response.finish_reason.reason == "stop"
        assert response.finish_reason.raw == "end_turn"
        assert response.usage.input_tokens == 11
        assert response.usage.output_tokens == 13
        assert response.usage.total_tokens == 24
        assert response.usage.cache_read_tokens == 5
        assert response.usage.cache_write_tokens == 2
        assert sent_request.method == "POST"
        assert sent_request.url.path == "/v1/messages"
        assert sent_request.headers["x-api-key"] == "anthropic-key"
        assert body["stream"] is True
        assert body["messages"] == [
            {
                "role": "user",
                "content": [
                    {
                        "type": "text",
                        "text": REQUEST_TEXT,
                    }
                ],
            }
        ]
        return

    if provider == "gemini":
        assert [event.type for event in events] == [
            unified_llm.StreamEventType.STREAM_START,
            unified_llm.StreamEventType.TEXT_START,
            unified_llm.StreamEventType.TEXT_DELTA,
            unified_llm.StreamEventType.TEXT_DELTA,
            unified_llm.StreamEventType.TEXT_END,
            unified_llm.StreamEventType.FINISH,
        ]
        assert events[1].delta is None
        assert _text_delta_values(events) == ["Hello", " world"]
        assert response.text == RESPONSE_TEXT
        assert response.finish_reason.reason == "stop"
        assert response.finish_reason.raw == "STOP"
        assert response.usage.input_tokens == 12
        assert response.usage.output_tokens == 34
        assert response.usage.total_tokens == 46
        assert response.usage.reasoning_tokens == 5
        assert response.usage.cache_read_tokens == 4
        assert sent_request.method == "POST"
        expected_stream_path = (
            "/v1beta/models/gemini-3.1-pro-preview"
            ":streamGenerateContent"
        )
        assert sent_request.url.path == expected_stream_path
        assert sent_request.url.params["key"] == "gemini-key"
        assert sent_request.url.params["alt"] == "sse"
        assert body == {
            "contents": [
                {
                    "role": "user",
                    "parts": [
                        {
                            "text": REQUEST_TEXT,
                        }
                    ],
                }
            ]
        }
        return

    raise AssertionError(f"Unsupported provider {provider!r}")


def _assert_image_parity(
    provider: str,
    *,
    sent_request: httpx.Request,
    body: dict[str, object],
    image_path: Path,
) -> None:
    encoded_bytes = base64.b64encode(b"image-bytes").decode("ascii")
    encoded_data_uri = f"data:image/png;base64,{encoded_bytes}"

    if provider == "openai":
        assert sent_request.method == "POST"
        assert sent_request.url.path == "/v1/responses"
        assert sent_request.headers["authorization"] == "Bearer openai-key"
        assert body["input"] == [
            {
                "type": "message",
                "role": "user",
                "content": [
                    {
                        "type": "input_text",
                        "text": "show me the image",
                    },
                    {
                        "type": "input_image",
                        "image_url": IMAGE_URL,
                    },
                    {
                        "type": "input_image",
                        "image_url": encoded_data_uri,
                    },
                    {
                        "type": "input_image",
                        "image_url": encoded_data_uri,
                    },
                ],
            }
        ]
        return

    if provider == "anthropic":
        assert sent_request.method == "POST"
        assert sent_request.url.path == "/v1/messages"
        assert sent_request.headers["x-api-key"] == "anthropic-key"
        assert sent_request.headers["anthropic-version"] == "2023-06-01"
        assert "anthropic-beta" not in sent_request.headers
        assert body["messages"] == [
            {
                "role": "user",
                "content": [
                    {
                        "type": "text",
                        "text": "show me the image",
                    },
                    {
                        "type": "image",
                        "source": {
                            "type": "url",
                            "url": IMAGE_URL,
                        },
                    },
                    {
                        "type": "image",
                        "source": {
                            "type": "base64",
                            "media_type": "image/png",
                            "data": encoded_bytes,
                        },
                    },
                    {
                        "type": "image",
                        "source": {
                            "type": "base64",
                            "media_type": "image/png",
                            "data": encoded_bytes,
                        },
                    },
                ],
            }
        ]
        assert "cache_control" not in body
        return

    if provider == "gemini":
        assert sent_request.method == "POST"
        assert sent_request.url.path == "/v1beta/models/gemini-3.1-pro-preview:generateContent"
        assert sent_request.url.params["key"] == "gemini-key"
        assert "authorization" not in sent_request.headers
        assert body == {
            "contents": [
                {
                    "role": "user",
                    "parts": [
                        {
                            "text": "show me the image",
                        },
                        {
                            "fileData": {
                                "fileUri": IMAGE_URL,
                                "mimeType": "image/png",
                            }
                        },
                        {
                            "inlineData": {
                                "data": encoded_bytes,
                                "mimeType": "image/png",
                            }
                        },
                        {
                            "inlineData": {
                                "data": encoded_bytes,
                                "mimeType": "image/png",
                            }
                        },
                    ],
                }
            ]
        }
        return

    raise AssertionError(f"Unsupported provider {provider!r}")


def _assert_tool_round_trip_parity(
    provider: str,
    *,
    first_response: unified_llm.Response,
    second_response: unified_llm.Response,
    first_request: httpx.Request,
    second_request: httpx.Request,
    first_body: dict[str, object],
    second_body: dict[str, object],
) -> None:
    assert first_response.text == "Use the tool"
    assert first_response.finish_reason.reason == "tool_calls"
    assert first_response.tool_calls[0].name == "lookup_weather"
    assert first_response.tool_calls[0].arguments == {"city": "Paris"}
    assert first_response.tool_calls[0].raw_arguments == '{"city":"Paris"}'
    assert second_response.text == "done"
    assert second_response.finish_reason.reason == "stop"

    if provider == "openai":
        assert first_request.method == "POST"
        assert first_request.url.path == "/v1/responses"
        assert second_request.method == "POST"
        assert second_request.url.path == "/v1/responses"
        assert first_body["input"] == [
            {
                "type": "message",
                "role": "user",
                "content": [
                    {
                        "type": "input_text",
                        "text": REQUEST_TEXT,
                    }
                ],
            },
        ]
        assert second_body["input"] == [
            {
                "type": "message",
                "role": "user",
                "content": [
                    {
                        "type": "input_text",
                        "text": REQUEST_TEXT,
                    }
                ],
            },
            {
                "type": "message",
                "role": "assistant",
                "content": [
                    {
                        "type": "output_text",
                        "text": "Use the tool",
                    }
                ],
            },
            {
                "type": "function_call",
                "id": "call_123",
                "name": "lookup_weather",
                "arguments": '{"city":"Paris"}',
            },
            {
                "type": "function_call_output",
                "call_id": "call_123",
                "output": json.dumps(TOOL_RESULT_CONTENT, separators=(",", ":"), sort_keys=True),
            },
        ]
        return

    if provider == "anthropic":
        assert first_request.method == "POST"
        assert first_request.url.path == "/v1/messages"
        assert first_request.headers["x-api-key"] == "anthropic-key"
        assert second_request.method == "POST"
        assert second_request.url.path == "/v1/messages"
        assert "anthropic-beta" not in first_request.headers
        assert "anthropic-beta" not in second_request.headers
        assert first_body["messages"] == [
            {
                "role": "user",
                "content": [
                    {
                        "type": "text",
                        "text": REQUEST_TEXT,
                    }
                ],
            },
        ]
        assert second_body["messages"] == [
            {
                "role": "user",
                "content": [
                    {
                        "type": "text",
                        "text": REQUEST_TEXT,
                    }
                ],
            },
            {
                "role": "assistant",
                "content": [
                    {
                        "type": "text",
                        "text": "Use the tool",
                    },
                    {
                        "type": "tool_use",
                        "id": "call_123",
                        "name": "lookup_weather",
                        "input": {
                            "city": "Paris",
                        },
                    },
                ],
            },
            {
                "role": "user",
                "content": [
                    {
                        "type": "tool_result",
                        "tool_use_id": "call_123",
                        "content": json.dumps(
                            TOOL_RESULT_CONTENT,
                            separators=(",", ":"),
                            sort_keys=True,
                        ),
                    }
                ],
            },
        ]
        return

    if provider == "gemini":
        provider_tool_call_id = "provider-call-weather-123"
        assert first_request.method == "POST"
        assert first_request.url.path == "/v1beta/models/gemini-3.1-pro-preview:generateContent"
        assert first_request.url.params["key"] == "gemini-key"
        assert second_request.method == "POST"
        assert second_request.url.path == "/v1beta/models/gemini-3.1-pro-preview:generateContent"
        assert second_request.url.params["key"] == "gemini-key"
        assert first_body["contents"] == [
            {
                "role": "user",
                "parts": [
                    {
                        "text": REQUEST_TEXT,
                    }
                ],
            },
        ]
        assert second_body["contents"] == [
            {
                "role": "user",
                "parts": [
                    {
                        "text": REQUEST_TEXT,
                    }
                ],
            },
            {
                "role": "model",
                "parts": [
                    {
                        "text": "Use the tool",
                    },
                    {
                        "functionCall": {
                            "name": "lookup_weather",
                            "args": {
                                "city": "Paris",
                            },
                        }
                    },
                ],
            },
            {
                "role": "user",
                "parts": [
                    {
                        "functionResponse": {
                            "id": provider_tool_call_id,
                            "name": "lookup_weather",
                            "response": TOOL_RESULT_CONTENT,
                        }
                    }
                ],
            },
        ]
        assert first_response.tool_calls[0].id == provider_tool_call_id
        assert first_response.tool_calls[0].name == "lookup_weather"
        return

    raise AssertionError(f"Unsupported provider {provider!r}")


def _text_delta_values(events: list[unified_llm.StreamEvent]) -> list[str]:
    return [
        event.delta or ""
        for event in events
        if event.type == unified_llm.StreamEventType.TEXT_DELTA
    ]


def _expected_stream_deltas(provider: str) -> list[str]:
    if provider == "openai":
        return ["Hel", "lo"]
    return ["Hello", " world"]


def _expected_stream_text(provider: str) -> str:
    if provider == "openai":
        return "Hello"
    return RESPONSE_TEXT


@pytest.mark.asyncio
@pytest.mark.parametrize("provider", PROVIDERS, ids=PROVIDERS)
async def test_native_text_generation_parity_and_usage_normalization(
    provider: str,
) -> None:
    response_body = _text_completion_body(provider)
    captured_requests, transport = _make_complete_transport(response_body)
    adapter = _make_adapter(provider, transport)
    request = _text_completion_request(provider)
    original_provider_options = copy.deepcopy(request.provider_options)

    response = await adapter.complete(request)

    assert len(captured_requests) == 1
    sent_request = captured_requests[0]
    body = _request_json(sent_request)

    _assert_text_completion_parity(
        provider,
        sent_request=sent_request,
        body=body,
        response=response,
        request=request,
        original_provider_options=original_provider_options,
    )


@pytest.mark.asyncio
@pytest.mark.parametrize("provider", PROVIDERS, ids=PROVIDERS)
async def test_native_streaming_text_parity_and_usage_normalization(
    provider: str,
) -> None:
    payload = _stream_payload(provider)
    captured_requests, transport = _make_stream_transport(payload)
    adapter = _make_adapter(provider, transport)
    request = unified_llm.Request(
        model=_provider_model(provider),
        messages=[unified_llm.Message.user(REQUEST_TEXT)],
        provider_options=_cache_suppression_options(),
    )

    events = [event async for event in adapter.stream(request)]
    accumulator = unified_llm.StreamAccumulator.from_events(events)
    response = accumulator.response

    assert len(captured_requests) == 1
    sent_request = captured_requests[0]
    body = _request_json(sent_request)

    _assert_stream_parity(
        provider,
        sent_request=sent_request,
        body=body,
        events=events,
        response=response,
    )


@pytest.mark.asyncio
@pytest.mark.parametrize("provider", PROVIDERS, ids=PROVIDERS)
async def test_native_multimodal_image_translation(
    provider: str,
    tmp_path: Path,
) -> None:
    image_path = tmp_path / "diagram.png"
    image_path.write_bytes(b"image-bytes")
    response_body = _text_completion_body(provider)
    captured_requests, transport = _make_complete_transport(response_body)
    adapter = _make_adapter(provider, transport)
    request = _image_request(provider, image_path)

    await adapter.complete(request)

    assert len(captured_requests) == 1
    sent_request = captured_requests[0]
    body = _request_json(sent_request)

    _assert_image_parity(
        provider,
        sent_request=sent_request,
        body=body,
        image_path=image_path,
    )


@pytest.mark.asyncio
@pytest.mark.parametrize(
    ("provider", "content_kind"),
    [
        (provider, content_kind)
        for provider in PROVIDERS
        for content_kind in (
            unified_llm.ContentKind.AUDIO,
            unified_llm.ContentKind.DOCUMENT,
        )
    ],
    ids=[
        f"{provider}-{content_kind.value}"
        for provider in PROVIDERS
        for content_kind in (
            unified_llm.ContentKind.AUDIO,
            unified_llm.ContentKind.DOCUMENT,
        )
    ],
)
async def test_native_audio_and_document_requests_are_rejected_gracefully(
    provider: str,
    content_kind: unified_llm.ContentKind,
) -> None:
    response_body = _text_completion_body(provider)
    captured_requests, transport = _make_complete_transport(response_body)
    adapter = _make_adapter(provider, transport)
    request = unified_llm.Request(
        model=_provider_model(provider),
        messages=[
            unified_llm.Message.user(
                [
                    _unsupported_media_part(content_kind),
                ]
            )
        ],
    )

    with pytest.raises(unified_llm.InvalidRequestError, match=content_kind.value):
        await adapter.complete(request)

    assert captured_requests == []


@pytest.mark.asyncio
@pytest.mark.parametrize("provider", PROVIDERS, ids=PROVIDERS)
async def test_native_tool_round_trip_continuations(
    provider: str,
) -> None:
    if provider == "openai":
        first_response_body = {
            "id": "resp_tool_openai",
            "model": "gpt-5.2",
            "status": "completed",
            "output": [
                {
                    "type": "output_text",
                    "text": "Use the tool",
                },
                {
                    "type": "function_call",
                    "id": "call_123",
                    "name": "lookup_weather",
                    "arguments": {
                        "city": "Paris",
                    },
                },
            ],
        }
        second_response_body = {
            "id": "resp_tool_openai_done",
            "model": "gpt-5.2",
            "status": "completed",
            "output_text": "done",
        }
    elif provider == "anthropic":
        first_response_body = {
            "id": "msg_tool_anthropic",
            "type": "message",
            "role": "assistant",
            "model": "claude-sonnet-4-5",
            "content": [
                {
                    "type": "text",
                    "text": "Use the tool",
                },
                {
                    "type": "tool_use",
                    "id": "call_123",
                    "name": "lookup_weather",
                    "input": {
                        "city": "Paris",
                    },
                },
            ],
            "stop_reason": "tool_use",
        }
        second_response_body = {
            "id": "msg_tool_anthropic_done",
            "type": "message",
            "role": "assistant",
            "model": "claude-sonnet-4-5",
            "content": [
                {
                    "type": "text",
                    "text": "done",
                }
            ],
            "stop_reason": "end_turn",
        }
    elif provider == "gemini":
        first_response_body = {
            "responseId": "resp_tool_gemini",
            "modelVersion": "gemini-3.1-pro-preview",
            "candidates": [
                {
                    "content": {
                        "role": "model",
                            "parts": [
                                {
                                    "text": "Use the tool",
                                },
                                {
                                    "functionCall": {
                                        "id": "provider-call-weather-123",
                                        "name": "lookup_weather",
                                        "args": {
                                            "city": "Paris",
                                        },
                                    }
                            },
                        ],
                    },
                    "finishReason": "STOP",
                }
            ],
        }
        second_response_body = {
            "responseId": "resp_tool_gemini_done",
            "modelVersion": "gemini-3.1-pro-preview",
            "candidates": [
                {
                    "content": {
                        "role": "model",
                        "parts": [
                            {
                                "text": "done",
                            }
                        ],
                    },
                    "finishReason": "STOP",
                }
            ],
        }
    else:
        raise AssertionError(f"Unsupported provider {provider!r}")

    captured_requests: list[httpx.Request] = []
    queued_responses = [first_response_body, second_response_body]

    async def handler(request: httpx.Request) -> httpx.Response:
        captured_requests.append(request)
        if not queued_responses:
            raise AssertionError("received more requests than expected")
        response_body = queued_responses.pop(0)
        return httpx.Response(
            200,
            headers={"content-type": "application/json"},
            content=json.dumps(response_body).encode("utf-8"),
        )

    adapter = _make_adapter(provider, httpx.MockTransport(handler))
    request1 = unified_llm.Request(
        model=_provider_model(provider),
        messages=[unified_llm.Message.user(REQUEST_TEXT)],
        provider_options=_cache_suppression_options(),
    )

    first_response = await adapter.complete(request1)
    tool_call_id = first_response.tool_calls[0].id
    request2 = _tool_round_trip_request(provider, first_response.message, tool_call_id)
    second_response = await adapter.complete(request2)

    assert len(captured_requests) == 2
    first_request = captured_requests[0]
    second_request = captured_requests[1]
    first_body = _request_json(first_request)
    second_body = _request_json(second_request)

    _assert_tool_round_trip_parity(
        provider,
        first_response=first_response,
        second_response=second_response,
        first_request=first_request,
        second_request=second_request,
        first_body=first_body,
        second_body=second_body,
    )


# Live smoke tests:
# Run with `uv run pytest -q --run-live` or `uv run pytest -q -m live`.
# Required keys:
# - OPENAI_API_KEY for OpenAI native and chat-completions smoke tests.
# - ANTHROPIC_API_KEY for Anthropic smoke tests.
# - GEMINI_API_KEY or GOOGLE_API_KEY for Gemini smoke tests.
# Optional model overrides:
# - UNIFIED_LLM_LIVE_OPENAI_MODEL
# - UNIFIED_LLM_LIVE_ANTHROPIC_MODEL
# - UNIFIED_LLM_LIVE_GEMINI_MODEL
# - UNIFIED_LLM_LIVE_CHAT_MODEL

LIVE_PROVIDER_MODEL_DEFAULTS = {
    "openai": "gpt-5.2",
    "anthropic": "claude-sonnet-4-5",
    "gemini": "gemini-3.1-pro-preview",
    "openai_compatible": "gpt-4o-mini",
}

LIVE_PROVIDER_MODEL_ENV_VARS = {
    "openai": "UNIFIED_LLM_LIVE_OPENAI_MODEL",
    "anthropic": "UNIFIED_LLM_LIVE_ANTHROPIC_MODEL",
    "gemini": "UNIFIED_LLM_LIVE_GEMINI_MODEL",
    "openai_compatible": "UNIFIED_LLM_LIVE_CHAT_MODEL",
}

LIVE_PROVIDER_API_KEY_ENV_VARS = {
    "openai": ("OPENAI_API_KEY",),
    "anthropic": ("ANTHROPIC_API_KEY",),
    "gemini": ("GEMINI_API_KEY", "GOOGLE_API_KEY"),
    "openai_compatible": ("OPENAI_API_KEY",),
}


def _live_env_value(name: str) -> str | None:
    value = os.environ.get(name)
    if value is None:
        return None
    value = value.strip()
    return value or None


def _live_api_key(provider: str) -> str:
    env_names = LIVE_PROVIDER_API_KEY_ENV_VARS[provider]
    for env_name in env_names:
        value = _live_env_value(env_name)
        if value is not None:
            return value

    if len(env_names) == 1:
        required = env_names[0]
    else:
        required = " or ".join(env_names)
    pytest.skip(f"{provider} live smoke tests require {required}")


def _live_model(provider: str) -> str:
    env_name = LIVE_PROVIDER_MODEL_ENV_VARS[provider]
    value = _live_env_value(env_name)
    if value is not None:
        return value
    return LIVE_PROVIDER_MODEL_DEFAULTS[provider]


def _live_adapter(provider: str) -> Any:
    api_key = _live_api_key(provider)
    if provider == "openai":
        return unified_llm.OpenAIAdapter(api_key=api_key)
    if provider == "anthropic":
        return unified_llm.AnthropicAdapter(api_key=api_key)
    if provider == "gemini":
        return unified_llm.GeminiAdapter(api_key=api_key)
    if provider == "openai_compatible":
        return unified_llm.OpenAICompatibleAdapter(
            api_key=api_key,
            base_url=_live_env_value("OPENAI_BASE_URL"),
        )
    raise AssertionError(f"Unsupported live provider {provider!r}")


def _live_provider_options(provider: str) -> dict[str, object]:
    if provider == "openai":
        return {
            "openai": {
                "parallel_tool_calls": False,
            },
            "anthropic": {
                "auto_cache": False,
            },
            "gemini": {
                "temperature": 0.25,
            },
        }
    if provider == "anthropic":
        return {
            "anthropic": {
                "auto_cache": False,
                "thinking": {
                    "type": "enabled",
                    "budget_tokens": 128,
                },
            },
            "openai": {
                "parallel_tool_calls": False,
            },
            "gemini": {
                "temperature": 0.25,
            },
        }
    if provider == "gemini":
        return {
            "gemini": {
                "temperature": 0.25,
                "thinkingConfig": {
                    "includeThoughts": True,
                },
            },
            "openai": {
                "parallel_tool_calls": False,
            },
            "anthropic": {
                "auto_cache": False,
            },
        }
    raise AssertionError(f"Unsupported live provider {provider!r}")


def _live_image_bytes() -> bytes:
    def _chunk(kind: bytes, data: bytes) -> bytes:
        checksum = zlib.crc32(kind + data) & 0xFFFFFFFF
        return (
            struct.pack(">I", len(data))
            + kind
            + data
            + struct.pack(">I", checksum)
        )

    pixel = b"\xff\x00\x00\xff"
    header = struct.pack(">IIBBBBB", 1, 1, 8, 6, 0, 0, 0)
    body = zlib.compress(b"\x00" + pixel)
    return b"".join(
        [
            b"\x89PNG\r\n\x1a\n",
            _chunk(b"IHDR", header),
            _chunk(b"IDAT", body),
            _chunk(b"IEND", b""),
        ]
    )


def _live_image_path(tmp_path: Path) -> Path:
    image_path = tmp_path / "live-smoke.png"
    image_path.write_bytes(_live_image_bytes())
    return image_path


def _live_generation_request(provider: str) -> unified_llm.Request:
    prompt = "What is 17 * 23? Return only the result."
    if provider == "openai":
        return unified_llm.Request(
            model=_live_model(provider),
            messages=[unified_llm.Message.user(prompt)],
            reasoning_effort="high",
            temperature=0,
            provider_options=_live_provider_options(provider),
        )
    if provider == "anthropic":
        return unified_llm.Request(
            model=_live_model(provider),
            messages=[unified_llm.Message.user(prompt)],
            temperature=0,
            provider_options=_live_provider_options(provider),
        )
    if provider == "gemini":
        return unified_llm.Request(
            model=_live_model(provider),
            messages=[unified_llm.Message.user(prompt)],
            temperature=0,
            provider_options=_live_provider_options(provider),
        )
    raise AssertionError(f"Unsupported live provider {provider!r}")


def _live_structured_output_schema() -> dict[str, object]:
    return {
        "type": "object",
        "properties": {
            "answer": {"type": "string"},
            "provider": {"type": "string"},
        },
        "required": ["answer", "provider"],
        "additionalProperties": False,
    }


def _live_structured_output_request(provider: str) -> unified_llm.Request:
    schema = _live_structured_output_schema()
    prompt = "Return JSON with answer and provider fields for the question 17 * 23."
    if provider == "openai":
        return unified_llm.Request(
            model=_live_model(provider),
            messages=[unified_llm.Message.user(prompt)],
            response_format=unified_llm.ResponseFormat(
                type="json_schema",
                json_schema=schema,
                strict=True,
            ),
            temperature=0,
            provider_options=_live_provider_options(provider),
        )
    if provider == "anthropic":
        return unified_llm.Request(
            model=_live_model(provider),
            messages=[unified_llm.Message.user(prompt)],
            temperature=0,
            provider_options={
                "anthropic": {
                    "auto_cache": False,
                    "structured_output": {
                        "provider": "anthropic",
                        "strategy": "schema-instruction",
                        "schema": schema,
                    },
                },
                "openai": {
                    "parallel_tool_calls": False,
                },
                "gemini": {
                    "temperature": 0.25,
                },
            },
        )
    if provider == "gemini":
        return unified_llm.Request(
            model=_live_model(provider),
            messages=[unified_llm.Message.user(prompt)],
            temperature=0,
            provider_options={
                "gemini": {
                    "temperature": 0.25,
                    "responseMimeType": "application/json",
                    "responseSchema": schema,
                    "structured_output": {
                        "provider": "gemini",
                        "strategy": "responseSchema",
                        "schema": schema,
                        "responseMimeType": "application/json",
                    },
                },
                "openai": {
                    "parallel_tool_calls": False,
                },
                "anthropic": {
                    "auto_cache": False,
                },
            },
        )
    raise AssertionError(f"Unsupported live provider {provider!r}")


def _live_image_request(provider: str, image_path: Path) -> unified_llm.Request:
    prompt = "What color is the square in the attached image? Reply with one word."
    return unified_llm.Request(
        model=_live_model(provider),
        messages=[
            unified_llm.Message.user(
                [
                    unified_llm.ContentPart(
                        kind=unified_llm.ContentKind.TEXT,
                        text=prompt,
                    ),
                    unified_llm.ContentPart(
                        kind=unified_llm.ContentKind.IMAGE,
                        image=unified_llm.ImageData(url=str(image_path)),
                    ),
                ]
            )
        ],
        temperature=0,
        provider_options=_live_provider_options(provider),
    )


def _live_chat_request() -> unified_llm.Request:
    return unified_llm.Request(
        model=_live_model("openai_compatible"),
        messages=[unified_llm.Message.user("Say hello in one short sentence.")],
        temperature=0,
        provider_options={
            "openai_compatible": {
                "parallel_tool_calls": False,
                "headers": {
                    "X-Live-Smoke": "1",
                },
            },
            "openai": {
                "reasoning": {
                    "effort": "high",
                },
            },
            "anthropic": {
                "auto_cache": False,
            },
        },
    )


def _live_weather_tool() -> unified_llm.Tool:
    weather_schema = {
        "type": "object",
        "properties": {
            "city": {"type": "string"},
        },
        "required": ["city"],
        "additionalProperties": False,
    }
    return unified_llm.Tool.passive(
        "lookup_weather",
        "Return the weather for a city.",
        weather_schema,
    )


def _live_tool_call_tools() -> list[unified_llm.Tool]:
    time_schema = {
        "type": "object",
        "properties": {
            "city": {"type": "string"},
        },
        "required": ["city"],
        "additionalProperties": False,
    }
    return [
        _live_weather_tool(),
        unified_llm.Tool.passive(
            "lookup_time",
            "Return the local time for a city.",
            time_schema,
        ),
    ]


def _live_parallel_tool_request() -> unified_llm.Request:
    return unified_llm.Request(
        model=_live_model("openai"),
        messages=[
            unified_llm.Message.user(
                "Use both tools to answer: what is the weather in Paris and the time in Tokyo?"
            )
        ],
        tools=_live_tool_call_tools(),
        tool_choice=unified_llm.ToolChoice.required(),
        temperature=0,
        provider_options={
            "openai": {
                "parallel_tool_calls": True,
            },
            "anthropic": {
                "auto_cache": False,
            },
            "gemini": {
                "temperature": 0.25,
            },
        },
    )


def _live_tool_request(provider: str) -> unified_llm.Request:
    return unified_llm.Request(
        model=_live_model(provider),
        messages=[
            unified_llm.Message.user(
                "Use the lookup_weather tool to answer: what is the weather in Paris?"
            )
        ],
        tools=[_live_weather_tool()],
        tool_choice=unified_llm.ToolChoice.required(),
        temperature=0,
        provider_options=_live_provider_options(provider),
    )


def _live_tool_result_request(
    provider: str,
    first_response: unified_llm.Response,
) -> unified_llm.Request:
    tool_call = first_response.tool_calls[0]
    return unified_llm.Request(
        model=_live_model(provider),
        messages=[
            unified_llm.Message.user(
                "Use the lookup_weather tool to answer: what is the weather in Paris?"
            ),
            first_response.message,
            unified_llm.Message.tool_result(
                tool_call.id,
                TOOL_RESULT_CONTENT,
                name=tool_call.name,
            ),
        ],
        tools=[_live_weather_tool()],
        temperature=0,
        provider_options=_live_provider_options(provider),
    )


@pytest.mark.live
@pytest.mark.asyncio
@pytest.mark.parametrize("provider", PROVIDERS, ids=PROVIDERS)
async def test_live_native_generation_and_streaming(
    provider: str,
) -> None:
    adapter = _live_adapter(provider)
    request = _live_generation_request(provider)

    response = await adapter.complete(request)
    stream_events = [event async for event in adapter.stream(request)]
    stream_response = unified_llm.StreamAccumulator.from_events(stream_events).response

    assert response.provider == provider
    assert stream_response.provider == provider
    assert response.text.strip()
    assert stream_response.text.strip()
    assert response.usage.total_tokens >= 0
    assert stream_response.usage.total_tokens >= 0
    assert any(event.type == unified_llm.StreamEventType.TEXT_DELTA for event in stream_events)
    assert stream_events[0].type == unified_llm.StreamEventType.STREAM_START
    assert stream_events[-1].type == unified_llm.StreamEventType.FINISH
    assert response.reasoning is not None or response.usage.reasoning_tokens is not None
    assert (
        stream_response.reasoning is not None
        or stream_response.usage.reasoning_tokens is not None
    )


@pytest.mark.live
@pytest.mark.asyncio
@pytest.mark.parametrize("provider", PROVIDERS, ids=PROVIDERS)
async def test_live_native_image_input(
    provider: str,
    tmp_path: Path,
) -> None:
    adapter = _live_adapter(provider)
    image_path = _live_image_path(tmp_path)
    request = _live_image_request(provider, image_path)

    response = await adapter.complete(request)

    assert response.provider == provider
    assert response.text.strip()
    assert response.usage.total_tokens >= 0


@pytest.mark.live
@pytest.mark.asyncio
@pytest.mark.parametrize("provider", PROVIDERS, ids=PROVIDERS)
async def test_live_native_structured_output(
    provider: str,
) -> None:
    adapter = _live_adapter(provider)
    request = _live_structured_output_request(provider)

    response = await adapter.complete(request)
    structured_response = json.loads(response.text)

    assert response.provider == provider
    assert structured_response["answer"].strip()
    assert structured_response["provider"].strip()


@pytest.mark.live
@pytest.mark.asyncio
@pytest.mark.parametrize("provider", PROVIDERS, ids=PROVIDERS)
async def test_live_native_tool_calling(provider: str) -> None:
    adapter = _live_adapter(provider)
    request = _live_tool_request(provider)

    first_response = await adapter.complete(request)
    tool_calls = first_response.tool_calls
    assert len(tool_calls) >= 1
    assert tool_calls[0].name == "lookup_weather"
    assert tool_calls[0].id

    second_response = await adapter.complete(
        _live_tool_result_request(provider, first_response)
    )

    assert second_response.provider == provider
    assert second_response.text.strip()


@pytest.mark.live
@pytest.mark.asyncio
async def test_live_openai_parallel_tool_calling() -> None:
    adapter = _live_adapter("openai")
    request = _live_parallel_tool_request()

    first_response = await adapter.complete(request)
    tool_calls = first_response.tool_calls
    assert len(tool_calls) >= 2
    assert {tool_call.name for tool_call in tool_calls} >= {
        "lookup_weather",
        "lookup_time",
    }

    second_request = unified_llm.Request(
        model=_live_model("openai"),
        messages=[
            unified_llm.Message.user(
                "Use both tools to answer: what is the weather in Paris and the time in Tokyo?"
            ),
            first_response.message,
            unified_llm.Message.tool_result(
                tool_calls[0].id,
                {"temperature": 72, "unit": "F"},
            ),
            unified_llm.Message.tool_result(
                tool_calls[1].id,
                {"time": "12:00"},
                is_error=True,
            ),
        ],
        tools=_live_tool_call_tools(),
        temperature=0,
        provider_options={
            "openai": {
                "parallel_tool_calls": True,
            },
            "anthropic": {
                "auto_cache": False,
            },
            "gemini": {
                "temperature": 0.25,
            },
        },
    )
    second_response = await adapter.complete(second_request)

    assert second_response.provider == "openai"
    assert second_response.text.strip()


@pytest.mark.live
@pytest.mark.asyncio
async def test_live_anthropic_caching_usage() -> None:
    adapter = _live_adapter("anthropic")
    prefix_messages = [
        unified_llm.Message.system(
            "You are a concise assistant that remembers shared context."
        ),
        unified_llm.Message.user("Remember the code phrase cobalt lantern."),
        unified_llm.Message.assistant("I will remember the code phrase."),
    ]
    first_request = unified_llm.Request(
        model=_live_model("anthropic"),
        messages=prefix_messages
        + [unified_llm.Message.user("What code phrase should you remember?")],
        temperature=0,
        provider_options={
            "anthropic": {
                "auto_cache": True,
            },
            "openai": {
                "parallel_tool_calls": False,
            },
            "gemini": {
                "temperature": 0.25,
            },
        },
    )
    second_request = unified_llm.Request(
        model=_live_model("anthropic"),
        messages=prefix_messages
        + [
            unified_llm.Message.user(
                "What code phrase should you remember? Answer with the phrase."
            )
        ],
        temperature=0,
        provider_options={
            "anthropic": {
                "auto_cache": True,
            },
            "openai": {
                "parallel_tool_calls": False,
            },
            "gemini": {
                "temperature": 0.25,
            },
        },
    )

    first_response = await adapter.complete(first_request)
    second_response = await adapter.complete(second_request)

    assert first_response.provider == "anthropic"
    assert second_response.provider == "anthropic"
    assert first_response.text.strip()
    assert second_response.text.strip()
    assert first_response.usage.cache_write_tokens is not None
    assert second_response.usage.cache_read_tokens is not None


@pytest.mark.live
@pytest.mark.asyncio
@pytest.mark.parametrize(
    "provider",
    ("openai", "anthropic", "gemini", "openai_compatible"),
    ids=("openai", "anthropic", "gemini", "openai-compatible"),
)
async def test_live_provider_error_conversion(
    provider: str,
    capsys: pytest.CaptureFixture[str],
) -> None:
    adapter = _live_adapter(provider)
    if provider == "openai_compatible":
        model = "definitely-not-a-real-chat-model"
    else:
        model = f"definitely-not-a-real-model-{provider}"
    request = unified_llm.Request(
        model=model,
        messages=[unified_llm.Message.user("hello")],
        temperature=0,
    )

    with pytest.raises(unified_llm.ProviderError) as exc_info:
        await adapter.complete(request)

    captured = capsys.readouterr()
    error = exc_info.value

    assert captured.out == ""
    assert captured.err == ""
    assert error.provider == provider
    assert error.message
    assert error.status_code is not None


@pytest.mark.live
@pytest.mark.asyncio
async def test_live_openai_chat_completions_via_compatible_adapter() -> None:
    adapter = _live_adapter("openai_compatible")
    request = _live_chat_request()

    response = await adapter.complete(request)
    stream_events = [event async for event in adapter.stream(request)]
    stream_response = unified_llm.StreamAccumulator.from_events(stream_events).response

    assert response.provider == "openai_compatible"
    assert stream_response.provider == "openai_compatible"
    assert response.text.strip()
    assert stream_response.text.strip()
    assert any(event.type == unified_llm.StreamEventType.TEXT_DELTA for event in stream_events)
    assert stream_events[0].type == unified_llm.StreamEventType.STREAM_START
    assert stream_events[-1].type == unified_llm.StreamEventType.FINISH
