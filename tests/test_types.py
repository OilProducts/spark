from __future__ import annotations

import dataclasses

import pytest

import unified_llm


def test_shared_message_and_content_records_are_public_dataclasses_or_str_enums() -> None:
    dataclass_types = (
        unified_llm.Request,
        unified_llm.Response,
        unified_llm.FinishReason,
        unified_llm.Usage,
        unified_llm.ResponseFormat,
        unified_llm.Warning,
        unified_llm.RateLimitInfo,
        unified_llm.StreamEvent,
        unified_llm.Message,
        unified_llm.ContentPart,
        unified_llm.ImageData,
        unified_llm.AudioData,
        unified_llm.DocumentData,
        unified_llm.ToolCallData,
        unified_llm.ToolResultData,
        unified_llm.ThinkingData,
    )

    for record_type in dataclass_types:
        assert dataclasses.is_dataclass(record_type)
        assert hasattr(unified_llm, record_type.__name__)

    assert issubclass(unified_llm.Role, str)
    assert issubclass(unified_llm.ContentKind, str)
    assert issubclass(unified_llm.StreamEventType, str)


@pytest.mark.parametrize(
    ("factory", "role"),
    [
        (unified_llm.Message.system, unified_llm.Role.SYSTEM),
        (unified_llm.Message.user, unified_llm.Role.USER),
        (unified_llm.Message.assistant, unified_llm.Role.ASSISTANT),
    ],
)
def test_message_text_constructors_create_structured_text_messages(factory, role) -> None:
    message = factory("Hello, world")

    assert message.role == role
    assert message.content == [
        unified_llm.ContentPart(kind=unified_llm.ContentKind.TEXT, text="Hello, world")
    ]
    assert message.text == "Hello, world"


def test_message_text_concatenates_text_parts_in_order_and_ignores_custom_kinds() -> None:
    custom_part = unified_llm.ContentPart(kind="vendor.custom.block", text="ignored")
    message = unified_llm.Message(
        role=unified_llm.Role.ASSISTANT,
        content=[
            unified_llm.ContentPart(kind=unified_llm.ContentKind.TEXT, text="Hello"),
            custom_part,
            unified_llm.ContentPart(kind=unified_llm.ContentKind.TEXT, text=", world"),
        ],
    )

    assert custom_part.kind == "vendor.custom.block"
    assert message.text == "Hello, world"


def test_message_text_returns_empty_string_without_text_parts() -> None:
    message = unified_llm.Message(
        role=unified_llm.Role.USER,
        content=[
            unified_llm.ContentPart(
                kind=unified_llm.ContentKind.IMAGE,
                image=unified_llm.ImageData(url="https://example.com/image.png"),
            )
        ],
    )

    assert message.text == ""


def test_message_constructor_normalizes_role_and_content_iterables() -> None:
    message = unified_llm.Message(
        role="developer",
        content=(
            unified_llm.ContentPart(kind="text", text="Hello"),
            unified_llm.ContentPart(kind="vendor.custom.block", text="ignored"),
            unified_llm.ContentPart(kind=unified_llm.ContentKind.TEXT, text=", world"),
        ),
        name="system-note",
    )

    assert message.role == unified_llm.Role.DEVELOPER
    assert isinstance(message.content, list)
    assert message.name == "system-note"
    assert message.text == "Hello, world"


def test_content_part_validates_payload_types_while_preserving_custom_kinds() -> None:
    custom_part = unified_llm.ContentPart(kind="vendor.custom.block", text="ignored")

    assert custom_part.kind == "vendor.custom.block"

    with pytest.raises(TypeError, match="text must be a string or None"):
        unified_llm.ContentPart(kind=unified_llm.ContentKind.TEXT, text=123)  # type: ignore[arg-type]

    with pytest.raises(TypeError, match="image must be an instance of ImageData or None"):
        unified_llm.ContentPart(
            kind=unified_llm.ContentKind.IMAGE,
            image="not-an-image",  # type: ignore[arg-type]
        )


def test_audio_and_document_data_preserve_public_fields() -> None:
    audio = unified_llm.AudioData(
        url="https://example.com/audio.mp3",
        media_type="audio/mp3",
    )
    document = unified_llm.DocumentData(
        data=b"%PDF-1.7",
        media_type="application/pdf",
        file_name="report.pdf",
    )

    assert audio.url == "https://example.com/audio.mp3"
    assert audio.data is None
    assert audio.media_type == "audio/mp3"
    assert document.url is None
    assert document.data == b"%PDF-1.7"
    assert document.media_type == "application/pdf"
    assert document.file_name == "report.pdf"


@pytest.mark.parametrize(
    ("factory", "kwargs"),
    [
        (unified_llm.AudioData, {}),
        (
            unified_llm.AudioData,
            {"url": "https://example.com/audio.mp3", "data": b"raw-bytes"},
        ),
        (unified_llm.DocumentData, {}),
        (
            unified_llm.DocumentData,
            {"url": "https://example.com/report.pdf", "data": b"raw-bytes"},
        ),
    ],
)
def test_audio_and_document_data_reject_invalid_url_data_combinations(
    factory,
    kwargs: dict[str, object],
) -> None:
    with pytest.raises(ValueError):
        factory(**kwargs)


@pytest.mark.parametrize(
    "kwargs",
    [
        {},
        {"url": "https://example.com/image.png", "data": b"raw-bytes"},
    ],
)
def test_image_data_rejects_invalid_media_combinations(kwargs: dict[str, object]) -> None:
    with pytest.raises(ValueError):
        unified_llm.ImageData(**kwargs)


def test_image_data_defaults_media_type_for_raw_image_data() -> None:
    image = unified_llm.ImageData(data=b"raw-bytes")
    url_image = unified_llm.ImageData(url="https://example.com/image.png")

    assert image.data == b"raw-bytes"
    assert image.media_type == "image/png"
    assert url_image.media_type is None


def test_tool_and_thinking_data_round_trip_through_public_records() -> None:
    tool_call = unified_llm.ToolCallData(
        id="call_123",
        name="weather",
        arguments={"city": "San Francisco"},
        type="function",
    )
    tool_result = unified_llm.ToolResultData(
        tool_call_id=tool_call.id,
        content={"temperature": 72, "unit": "F"},
        is_error=False,
        image_data=b"image-bytes",
        image_media_type="image/png",
    )
    thinking = unified_llm.ThinkingData(
        text="internal reasoning",
        signature="sig_abc123",
        redacted=False,
    )
    redacted_thinking = unified_llm.ThinkingData(
        text="opaque payload",
        signature=None,
        redacted=True,
    )

    tool_call_part = unified_llm.ContentPart(
        kind=unified_llm.ContentKind.TOOL_CALL,
        tool_call=tool_call,
    )
    tool_result_part = unified_llm.ContentPart(
        kind=unified_llm.ContentKind.TOOL_RESULT,
        tool_result=tool_result,
    )
    thinking_part = unified_llm.ContentPart(
        kind=unified_llm.ContentKind.THINKING,
        thinking=thinking,
    )
    redacted_part = unified_llm.ContentPart(
        kind="redacted_thinking",
        thinking=redacted_thinking,
    )

    assert tool_call_part.tool_call == tool_call
    assert tool_result_part.tool_result == tool_result
    assert thinking_part.thinking == thinking
    assert redacted_part.kind == unified_llm.ContentKind.REDACTED_THINKING
    assert redacted_part.thinking == redacted_thinking


def test_tool_call_data_accepts_raw_and_structured_arguments() -> None:
    raw_tool_call = unified_llm.ToolCallData(
        id="call_456",
        name="lookup_weather",
        arguments='{"city": "Paris"}',
    )
    structured_tool_call = unified_llm.ToolCallData(
        id="call_789",
        name="lookup_weather",
        arguments={"city": "Berlin"},
        type="custom",
    )

    assert raw_tool_call.arguments == '{"city": "Paris"}'
    assert raw_tool_call.type == "function"
    assert structured_tool_call.arguments == {"city": "Berlin"}
    assert structured_tool_call.type == "custom"


def test_tool_result_message_constructor_preserves_tool_result_payload() -> None:
    message = unified_llm.Message.tool_result(
        tool_call_id="call_123",
        content={"temperature": 72, "unit": "F"},
        is_error=True,
        image_data=b"image-bytes",
        image_media_type="image/png",
    )

    assert message.role == unified_llm.Role.TOOL
    assert message.tool_call_id == "call_123"
    assert message.text == ""
    assert message.content == [
        unified_llm.ContentPart(
            kind=unified_llm.ContentKind.TOOL_RESULT,
            tool_result=unified_llm.ToolResultData(
                tool_call_id="call_123",
                content={"temperature": 72, "unit": "F"},
                is_error=True,
                image_data=b"image-bytes",
                image_media_type="image/png",
            ),
        )
    ]


def test_request_construction_preserves_generation_settings_and_escape_hatches() -> None:
    system_message = unified_llm.Message.system("System context")
    user_message = unified_llm.Message.user("Explain the result")
    tool = unified_llm.Tool(
        name="get_weather",
        description="Fetch the weather for a location",
        parameters={"type": "object", "properties": {"location": {"type": "string"}}},
    )
    tool_choice = unified_llm.ToolChoice(mode="required", tool_name=None)
    response_format = unified_llm.ResponseFormat(
        type="json_schema",
        json_schema={"type": "object", "properties": {"answer": {"type": "string"}}},
        strict=True,
    )

    request = unified_llm.Request(
        model="gpt-5.2",
        messages=[system_message, user_message],
        provider="openai",
        tools=[tool],
        tool_choice=tool_choice,
        response_format=response_format,
        temperature=0.2,
        top_p=0.9,
        max_tokens=256,
        stop_sequences=["END"],
        reasoning_effort="high",
        metadata={"trace_id": "trace-123"},
        provider_options={"openai": {"reasoning": {"effort": "high"}}},
    )

    assert request.model == "gpt-5.2"
    assert request.messages == [system_message, user_message]
    assert request.provider == "openai"
    assert request.tools == [tool]
    assert request.tool_choice is tool_choice
    assert request.response_format == response_format
    assert request.temperature == 0.2
    assert request.top_p == 0.9
    assert request.max_tokens == 256
    assert request.stop_sequences == ["END"]
    assert request.reasoning_effort == "high"
    assert request.metadata == {"trace_id": "trace-123"}
    assert request.provider_options == {"openai": {"reasoning": {"effort": "high"}}}


def test_response_accessors_derive_text_tool_calls_reasoning_and_metadata() -> None:
    tool_call_data = unified_llm.ToolCallData(
        id="call_123",
        name="get_weather",
        arguments='{"location": "San Francisco", "unit": "celsius"}',
        type="custom",
    )
    message = unified_llm.Message(
        role=unified_llm.Role.ASSISTANT,
        content=[
            unified_llm.ContentPart(kind=unified_llm.ContentKind.TEXT, text="The result is "),
            unified_llm.ContentPart(
                kind=unified_llm.ContentKind.TOOL_CALL,
                tool_call=tool_call_data,
            ),
            unified_llm.ContentPart(
                kind=unified_llm.ContentKind.THINKING,
                thinking=unified_llm.ThinkingData(
                    text="internal reasoning",
                    signature="sig_123",
                    redacted=False,
                ),
            ),
            unified_llm.ContentPart(
                kind=unified_llm.ContentKind.REDACTED_THINKING,
                thinking=unified_llm.ThinkingData(
                    text="redacted reasoning",
                    signature=None,
                    redacted=True,
                ),
            ),
            unified_llm.ContentPart(kind=unified_llm.ContentKind.TEXT, text="done."),
        ],
    )
    finish_reason = unified_llm.FinishReason(
        reason=unified_llm.FinishReason.STOP,
        raw="end_turn",
    )
    usage = unified_llm.Usage(
        input_tokens=12,
        output_tokens=34,
        total_tokens=46,
        reasoning_tokens=9,
        cache_read_tokens=4,
        cache_write_tokens=2,
        raw={"input_tokens": 12, "output_tokens": 34},
    )
    warnings = [unified_llm.Warning(message="soft limit approached", code="rate_limit")]
    rate_limit = unified_llm.RateLimitInfo(
        requests_remaining=11,
        requests_limit=20,
        tokens_remaining=1000,
        tokens_limit=2000,
        reset_at="2026-04-21T00:00:00Z",
    )

    response = unified_llm.Response(
        id="resp_123",
        model="gpt-5.2",
        provider="openai",
        message=message,
        finish_reason=finish_reason,
        usage=usage,
        raw={"id": "resp_123", "provider": "openai"},
        warnings=warnings,
        rate_limit=rate_limit,
    )

    assert response.id == "resp_123"
    assert response.model == "gpt-5.2"
    assert response.provider == "openai"
    assert response.text == "The result is done."
    assert len(response.tool_calls) == 1
    assert response.tool_calls[0].id == "call_123"
    assert response.tool_calls[0].name == "get_weather"
    assert response.tool_calls[0].arguments == {
        "location": "San Francisco",
        "unit": "celsius",
    }
    assert response.tool_calls[0].raw_arguments == (
        '{"location": "San Francisco", "unit": "celsius"}'
    )
    assert response.tool_calls[0].type == "custom"
    assert response.reasoning == "internal reasoningredacted reasoning"
    assert response.finish_reason.reason == "stop"
    assert response.finish_reason.raw == "end_turn"
    assert response.usage == usage
    assert response.usage.reasoning_tokens == 9
    assert response.usage.cache_read_tokens == 4
    assert response.usage.cache_write_tokens == 2
    assert response.raw == {"id": "resp_123", "provider": "openai"}
    assert response.warnings == warnings
    assert response.rate_limit == rate_limit


def test_response_tool_call_extraction_parses_string_arguments_with_raw_copy() -> None:
    message = unified_llm.Message(
        role=unified_llm.Role.ASSISTANT,
        content=[
            unified_llm.ContentPart(
                kind=unified_llm.ContentKind.TOOL_CALL,
                tool_call=unified_llm.ToolCall(
                    id="call_456",
                    name="lookup_weather",
                    arguments='{"location": "Paris"}',
                    raw_arguments='{"location": "Paris"}',
                ),
            )
        ],
    )

    response = unified_llm.Response(message=message)

    assert len(response.tool_calls) == 1
    assert response.tool_calls[0].id == "call_456"
    assert response.tool_calls[0].arguments == {"location": "Paris"}
    assert response.tool_calls[0].raw_arguments == '{"location": "Paris"}'


def test_response_reasoning_falls_back_to_reasoning_part_text() -> None:
    message = unified_llm.Message(
        role=unified_llm.Role.ASSISTANT,
        content=[
            unified_llm.ContentPart(
                kind=unified_llm.ContentKind.THINKING,
                text="fallback reasoning",
            )
        ],
    )

    response = unified_llm.Response(message=message)

    assert response.reasoning == "fallback reasoning"


@pytest.mark.parametrize(
    ("reason", "raw", "expected"),
    [
        (unified_llm.FinishReason.STOP, "end_turn", "stop"),
        (unified_llm.FinishReason.LENGTH, "max_tokens", "length"),
        (unified_llm.FinishReason.TOOL_CALLS, "tool_calls", "tool_calls"),
        (unified_llm.FinishReason.CONTENT_FILTER, "content_filter", "content_filter"),
        (unified_llm.FinishReason.ERROR, "error", "error"),
        (unified_llm.FinishReason.OTHER, "provider_specific", "other"),
        ("vendor.custom.reason", "vendor_raw", "vendor.custom.reason"),
    ],
)
def test_finish_reason_preserves_portable_reason_strings_and_raw_values(
    reason: object,
    raw: str,
    expected: str,
) -> None:
    finish_reason = unified_llm.FinishReason(reason=reason, raw=raw)

    assert finish_reason.reason == expected
    assert finish_reason.raw == raw


def test_response_normalizes_finish_reason_strings() -> None:
    response = unified_llm.Response(finish_reason="stop")

    assert isinstance(response.finish_reason, unified_llm.FinishReason)
    assert response.finish_reason.reason == "stop"


def test_usage_addition_sums_required_fields_and_handles_optional_fields() -> None:
    left = unified_llm.Usage(
        input_tokens=10,
        output_tokens=20,
        total_tokens=30,
        reasoning_tokens=None,
        cache_read_tokens=5,
        cache_write_tokens=None,
        raw={"left": True},
    )
    right = unified_llm.Usage(
        input_tokens=1,
        output_tokens=2,
        total_tokens=3,
        reasoning_tokens=7,
        cache_read_tokens=None,
        cache_write_tokens=4,
        raw={"right": True},
    )

    combined = left + right
    empty_combined = unified_llm.Usage() + unified_llm.Usage()

    assert combined.input_tokens == 11
    assert combined.output_tokens == 22
    assert combined.total_tokens == 33
    assert combined.reasoning_tokens == 7
    assert combined.cache_read_tokens == 5
    assert combined.cache_write_tokens == 4
    assert combined.raw == {"left": True}
    assert empty_combined.input_tokens == 0
    assert empty_combined.output_tokens == 0
    assert empty_combined.total_tokens == 0
    assert empty_combined.reasoning_tokens is None
    assert empty_combined.cache_read_tokens is None
    assert empty_combined.cache_write_tokens is None


def test_stream_event_normalizes_tool_call_data_with_raw_arguments() -> None:
    event = unified_llm.StreamEvent(
        type="tool_call_end",
        tool_call=unified_llm.ToolCallData(
            id="call_123",
            name="get_weather",
            arguments='{"location": "Paris"}',
            type="custom",
        ),
    )

    assert event.type == unified_llm.StreamEventType.TOOL_CALL_END
    assert event.tool_call.id == "call_123"
    assert event.tool_call.name == "get_weather"
    assert event.tool_call.arguments == {"location": "Paris"}
    assert event.tool_call.raw_arguments == '{"location": "Paris"}'
    assert event.tool_call.type == "custom"


def test_stream_event_construction_supports_lifecycle_events_and_custom_types() -> None:
    text_event = unified_llm.StreamEvent(
        type=unified_llm.StreamEventType.TEXT_START,
        text_id="text-1",
    )
    reasoning_event = unified_llm.StreamEvent(
        type=unified_llm.StreamEventType.REASONING_DELTA,
        reasoning_delta="thinking...",
    )
    tool_call_event = unified_llm.StreamEvent(
        type=unified_llm.StreamEventType.TOOL_CALL_END,
        tool_call=unified_llm.ToolCall(
            id="call_123",
            name="get_weather",
            arguments={"location": "Paris"},
            raw_arguments='{"location": "Paris"}',
        ),
    )
    finish_event = unified_llm.StreamEvent(
        type=unified_llm.StreamEventType.FINISH,
        finish_reason=unified_llm.FinishReason(reason="stop", raw="end_turn"),
        usage=unified_llm.Usage(input_tokens=1, output_tokens=2, total_tokens=3),
        response=unified_llm.Response(id="resp_1", model="gpt", provider="openai"),
    )
    error_event = unified_llm.StreamEvent(
        type=unified_llm.StreamEventType.ERROR,
        error=unified_llm.SDKError("boom"),
    )
    raw_provider_event = unified_llm.StreamEvent(
        type="provider_event",
        raw={"event": "delta"},
    )
    custom_event = unified_llm.StreamEvent(
        type="vendor.custom.event",
        raw={"kind": "custom"},
    )
    step_finish_event = unified_llm.StreamEvent(
        type="step_finish",
        raw={"step": "complete"},
    )

    assert text_event.type == unified_llm.StreamEventType.TEXT_START
    assert text_event.text_id == "text-1"
    assert reasoning_event.type == unified_llm.StreamEventType.REASONING_DELTA
    assert reasoning_event.reasoning_delta == "thinking..."
    assert tool_call_event.type == unified_llm.StreamEventType.TOOL_CALL_END
    assert tool_call_event.tool_call.id == "call_123"
    assert tool_call_event.tool_call.name == "get_weather"
    assert tool_call_event.tool_call.arguments == {"location": "Paris"}
    assert tool_call_event.tool_call.raw_arguments == '{"location": "Paris"}'
    assert finish_event.type == unified_llm.StreamEventType.FINISH
    assert finish_event.finish_reason.reason == "stop"
    assert finish_event.usage.total_tokens == 3
    assert finish_event.response.text == ""
    assert error_event.type == unified_llm.StreamEventType.ERROR
    assert error_event.error.message == "boom"
    assert raw_provider_event.type == unified_llm.StreamEventType.PROVIDER_EVENT
    assert raw_provider_event.raw == {"event": "delta"}
    assert custom_event.type == "vendor.custom.event"
    assert custom_event.raw == {"kind": "custom"}
    assert step_finish_event.type == "step_finish"
    assert step_finish_event.raw == {"step": "complete"}
