from __future__ import annotations

import json
import logging
from collections.abc import Iterable
from dataclasses import dataclass, field
from enum import StrEnum
from typing import TYPE_CHECKING, Any

from .errors import SDKError

if TYPE_CHECKING:
    from .tools import Tool, ToolCall, ToolChoice

logger = logging.getLogger(__name__)


class _PlaceholderRecord:
    def __init__(self, **fields: Any) -> None:
        self.__dict__.update(fields)

    def __repr__(self) -> str:
        fields = ", ".join(
            f"{name}={value!r}" for name, value in sorted(self.__dict__.items())
        )
        return f"{self.__class__.__name__}({fields})" if fields else f"{self.__class__.__name__}()"


class Role(StrEnum):
    SYSTEM = "system"
    USER = "user"
    ASSISTANT = "assistant"
    TOOL = "tool"
    DEVELOPER = "developer"


class ContentKind(StrEnum):
    TEXT = "text"
    IMAGE = "image"
    AUDIO = "audio"
    DOCUMENT = "document"
    TOOL_CALL = "tool_call"
    TOOL_RESULT = "tool_result"
    THINKING = "thinking"
    REDACTED_THINKING = "redacted_thinking"


class _FinishReasonValue(StrEnum):
    STOP = "stop"
    LENGTH = "length"
    TOOL_CALLS = "tool_calls"
    CONTENT_FILTER = "content_filter"
    ERROR = "error"
    OTHER = "other"


class StreamEventType(StrEnum):
    STREAM_START = "stream_start"
    TEXT_START = "text_start"
    TEXT_DELTA = "text_delta"
    TEXT_END = "text_end"
    REASONING_START = "reasoning_start"
    REASONING_DELTA = "reasoning_delta"
    REASONING_END = "reasoning_end"
    TOOL_CALL_START = "tool_call_start"
    TOOL_CALL_DELTA = "tool_call_delta"
    TOOL_CALL_END = "tool_call_end"
    FINISH = "finish"
    ERROR = "error"
    PROVIDER_EVENT = "provider_event"


def _validate_exactly_one_of_url_or_data(url: Any, data: Any) -> None:
    has_url = url is not None
    has_data = data is not None
    if has_url == has_data:
        raise ValueError("exactly one of url or data must be provided")


def _validate_optional_str(value: Any, field_name: str) -> None:
    if value is not None and not isinstance(value, str):
        raise TypeError(f"{field_name} must be a string or None")


def _validate_optional_instance(
    value: Any,
    field_name: str,
    expected_type: type[Any] | tuple[type[Any], ...],
) -> None:
    if value is not None and not isinstance(value, expected_type):
        if isinstance(expected_type, tuple):
            expected_name = " or ".join(type_.__name__ for type_ in expected_type)
        else:
            expected_name = expected_type.__name__
        raise TypeError(
            f"{field_name} must be an instance of {expected_name} or None"
        )


@dataclass(slots=True)
class ImageData:
    url: str | None = None
    data: bytes | None = None
    media_type: str | None = None
    detail: str | None = None

    def __post_init__(self) -> None:
        _validate_exactly_one_of_url_or_data(self.url, self.data)
        if self.url is not None and not isinstance(self.url, str):
            raise TypeError("url must be a string or None")
        if self.data is not None and not isinstance(self.data, bytes):
            raise TypeError("data must be bytes or None")
        if self.data is not None and self.media_type is None:
            self.media_type = "image/png"
        _validate_optional_str(self.media_type, "media_type")
        _validate_optional_str(self.detail, "detail")


@dataclass(slots=True)
class AudioData:
    url: str | None = None
    data: bytes | None = None
    media_type: str | None = None

    def __post_init__(self) -> None:
        _validate_exactly_one_of_url_or_data(self.url, self.data)
        if self.url is not None and not isinstance(self.url, str):
            raise TypeError("url must be a string or None")
        if self.data is not None and not isinstance(self.data, bytes):
            raise TypeError("data must be bytes or None")
        _validate_optional_str(self.media_type, "media_type")


@dataclass(slots=True)
class DocumentData:
    url: str | None = None
    data: bytes | None = None
    media_type: str | None = None
    file_name: str | None = None

    def __post_init__(self) -> None:
        _validate_exactly_one_of_url_or_data(self.url, self.data)
        if self.url is not None and not isinstance(self.url, str):
            raise TypeError("url must be a string or None")
        if self.data is not None and not isinstance(self.data, bytes):
            raise TypeError("data must be bytes or None")
        _validate_optional_str(self.media_type, "media_type")
        _validate_optional_str(self.file_name, "file_name")


@dataclass(slots=True)
class ToolCallData:
    id: str
    name: str
    arguments: dict[str, Any] | str
    type: str = "function"

    def __post_init__(self) -> None:
        if not isinstance(self.id, str):
            raise TypeError("id must be a string")
        if not isinstance(self.name, str):
            raise TypeError("name must be a string")
        if not isinstance(self.arguments, (dict, str)):
            raise TypeError("arguments must be a dict or string")
        if not isinstance(self.type, str):
            raise TypeError("type must be a string")


@dataclass(slots=True)
class ToolResultData:
    tool_call_id: str
    content: str | dict[str, Any] | list[Any]
    is_error: bool
    image_data: bytes | None = None
    image_media_type: str | None = None

    def __post_init__(self) -> None:
        if not isinstance(self.tool_call_id, str):
            raise TypeError("tool_call_id must be a string")
        if not isinstance(self.content, (str, dict, list)):
            raise TypeError("content must be a string, dict, or list")
        if not isinstance(self.is_error, bool):
            raise TypeError("is_error must be a boolean")
        if self.image_data is not None and not isinstance(self.image_data, bytes):
            raise TypeError("image_data must be bytes or None")
        if self.image_media_type is not None and not isinstance(self.image_media_type, str):
            raise TypeError("image_media_type must be a string or None")


@dataclass(slots=True)
class ThinkingData:
    text: str
    signature: str | None = None
    redacted: bool = False

    def __post_init__(self) -> None:
        if not isinstance(self.text, str):
            raise TypeError("text must be a string")
        if self.signature is not None and not isinstance(self.signature, str):
            raise TypeError("signature must be a string or None")
        if not isinstance(self.redacted, bool):
            raise TypeError("redacted must be a boolean")


@dataclass(slots=True)
class ContentPart:
    kind: ContentKind | str
    text: str | None = None
    image: ImageData | None = None
    audio: AudioData | None = None
    document: DocumentData | None = None
    tool_call: ToolCallData | None = None
    tool_result: ToolResultData | None = None
    thinking: ThinkingData | None = None

    def __post_init__(self) -> None:
        from .tools import ToolCall, ToolResult

        self.kind = _coerce_content_kind(self.kind)
        _validate_optional_str(self.text, "text")
        _validate_optional_instance(self.image, "image", ImageData)
        _validate_optional_instance(self.audio, "audio", AudioData)
        _validate_optional_instance(self.document, "document", DocumentData)
        _validate_optional_instance(
            self.tool_call,
            "tool_call",
            (ToolCallData, ToolCall),
        )
        _validate_optional_instance(
            self.tool_result,
            "tool_result",
            (ToolResultData, ToolResult),
        )
        _validate_optional_instance(self.thinking, "thinking", ThinkingData)


def _coerce_role(role: Role | str) -> Role:
    if isinstance(role, Role):
        return role
    if isinstance(role, str):
        return Role(role)
    raise TypeError("role must be a Role or role string")


def _coerce_content_kind(kind: ContentKind | str) -> ContentKind | str:
    if isinstance(kind, ContentKind):
        return kind
    if isinstance(kind, str):
        try:
            return ContentKind(kind)
        except ValueError:
            return kind
    raise TypeError("kind must be a ContentKind or string")


def _normalize_content_parts(content: Iterable[ContentPart] | ContentPart) -> list[ContentPart]:
    if isinstance(content, ContentPart):
        parts = [content]
    elif isinstance(content, Iterable) and not isinstance(
        content, (str, bytes, bytearray, memoryview)
    ):
        parts = list(content)
    else:
        raise TypeError("content must be a ContentPart or iterable of ContentPart")

    for part in parts:
        if not isinstance(part, ContentPart):
            raise TypeError("content must contain only ContentPart instances")
    return list(parts)


def _message_content_from_input(
    content: str | ContentPart | Iterable[ContentPart],
) -> list[ContentPart]:
    if isinstance(content, str):
        return [ContentPart(kind=ContentKind.TEXT, text=content)]
    return _normalize_content_parts(content)


def _coerce_optional_sequence(value: Any, field_name: str) -> list[Any] | None:
    if value is None:
        return None
    if isinstance(value, Iterable) and not isinstance(
        value, (str, bytes, bytearray, memoryview)
    ):
        return list(value)
    raise TypeError(f"{field_name} must be an iterable or None")


def _coerce_required_sequence(value: Any, field_name: str) -> list[Any]:
    normalized = _coerce_optional_sequence(value, field_name)
    if normalized is None:
        raise TypeError(f"{field_name} must be an iterable")
    return normalized


def _extract_reasoning_text(content: Iterable[ContentPart]) -> str | None:
    segments: list[str] = []
    saw_reasoning = False
    for part in content:
        if part.kind in (ContentKind.THINKING, ContentKind.REDACTED_THINKING):
            saw_reasoning = True
            if part.thinking is not None:
                segments.append(part.thinking.text)
            elif part.text is not None:
                segments.append(part.text)
    if not saw_reasoning:
        return None
    return "".join(segments)


def _extract_tool_call_data(part: ContentPart) -> Any | None:
    tool_call = part.tool_call
    if tool_call is None:
        return None

    return _coerce_tool_call_value(tool_call)


def _coerce_finish_reason_value(value: Any) -> FinishReason:
    if isinstance(value, FinishReason):
        return value
    if isinstance(value, StrEnum):
        value = value.value
    if isinstance(value, str):
        return FinishReason(reason=value)
    raise TypeError("finish_reason must be a FinishReason or string")


def _coerce_tool_call_value(tool_call: Any) -> Any:
    from .tools import ToolCall

    arguments = getattr(tool_call, "arguments", None)
    raw_arguments = getattr(tool_call, "raw_arguments", None)
    tool_call_type = getattr(tool_call, "type", "function")
    if tool_call_type is None:
        tool_call_type = "function"
    elif not isinstance(tool_call_type, str):
        raise TypeError("type must be a string")
    if isinstance(arguments, str):
        if raw_arguments is None:
            raw_arguments = arguments
        try:
            arguments = json.loads(arguments)
        except json.JSONDecodeError:
            pass

    return ToolCall(
        id=getattr(tool_call, "id"),
        name=getattr(tool_call, "name"),
        arguments=arguments,
        raw_arguments=raw_arguments,
        type=tool_call_type,
    )


@dataclass(slots=True)
class Message:
    role: Role
    content: list[ContentPart]
    name: str | None = None
    tool_call_id: str | None = None

    def __post_init__(self) -> None:
        self.role = _coerce_role(self.role)
        self.content = _normalize_content_parts(self.content)
        if self.name is not None and not isinstance(self.name, str):
            raise TypeError("name must be a string or None")
        if self.tool_call_id is not None and not isinstance(self.tool_call_id, str):
            raise TypeError("tool_call_id must be a string or None")

    @property
    def text(self) -> str:
        return "".join(
            part.text or ""
            for part in self.content
            if part.kind == ContentKind.TEXT
        )

    @classmethod
    def system(
        cls,
        content: str | ContentPart | Iterable[ContentPart],
        name: str | None = None,
    ) -> Message:
        return cls(role=Role.SYSTEM, content=_message_content_from_input(content), name=name)

    @classmethod
    def user(
        cls,
        content: str | ContentPart | Iterable[ContentPart],
        name: str | None = None,
    ) -> Message:
        return cls(role=Role.USER, content=_message_content_from_input(content), name=name)

    @classmethod
    def assistant(
        cls,
        content: str | ContentPart | Iterable[ContentPart],
        name: str | None = None,
    ) -> Message:
        return cls(role=Role.ASSISTANT, content=_message_content_from_input(content), name=name)

    @classmethod
    def tool_result(
        cls,
        tool_call_id: str,
        content: Any,
        is_error: bool = False,
        *,
        name: str | None = None,
        image_data: bytes | None = None,
        image_media_type: str | None = None,
    ) -> Message:
        return cls(
            role=Role.TOOL,
            content=[
                ContentPart(
                    kind=ContentKind.TOOL_RESULT,
                    tool_result=ToolResultData(
                        tool_call_id=tool_call_id,
                        content=content,
                        is_error=is_error,
                        image_data=image_data,
                        image_media_type=image_media_type,
                    ),
                )
            ],
            name=name,
            tool_call_id=tool_call_id,
        )


@dataclass(slots=True)
class FinishReason:
    reason: str | _FinishReasonValue
    raw: str | None = None

    def __post_init__(self) -> None:
        if isinstance(self.reason, StrEnum):
            self.reason = self.reason.value
        elif not isinstance(self.reason, str):
            raise TypeError("reason must be a string or FinishReason enum value")


for _name, _value in _FinishReasonValue.__members__.items():
    setattr(FinishReason, _name, _value)


@dataclass(slots=True)
class Request:
    model: str
    messages: list[Message]
    provider: str | None = None
    tools: list[Tool] | None = None
    tool_choice: ToolChoice | None = None
    response_format: ResponseFormat | None = None
    temperature: float | None = None
    top_p: float | None = None
    max_tokens: int | None = None
    stop_sequences: list[str] | None = None
    reasoning_effort: str | None = None
    metadata: dict[str, str] | None = None
    provider_options: dict[str, Any] | None = None

    def __post_init__(self) -> None:
        self.messages = _coerce_required_sequence(self.messages, "messages")
        self.tools = _coerce_optional_sequence(self.tools, "tools")
        self.stop_sequences = _coerce_optional_sequence(
            self.stop_sequences,
            "stop_sequences",
        )


@dataclass(slots=True)
class Usage:
    input_tokens: int = 0
    output_tokens: int = 0
    total_tokens: int = 0
    reasoning_tokens: int | None = None
    cache_read_tokens: int | None = None
    cache_write_tokens: int | None = None
    raw: Any | None = None

    def __add__(self, other: Usage) -> Usage:
        if not isinstance(other, Usage):
            return NotImplemented

        return Usage(
            input_tokens=self.input_tokens + other.input_tokens,
            output_tokens=self.output_tokens + other.output_tokens,
            total_tokens=self.total_tokens + other.total_tokens,
            reasoning_tokens=_sum_optional_ints(
                self.reasoning_tokens,
                other.reasoning_tokens,
            ),
            cache_read_tokens=_sum_optional_ints(
                self.cache_read_tokens,
                other.cache_read_tokens,
            ),
            cache_write_tokens=_sum_optional_ints(
                self.cache_write_tokens,
                other.cache_write_tokens,
            ),
            raw=self.raw if self.raw is not None else other.raw,
        )

    def __radd__(self, other: object) -> Usage:
        if other == 0:
            return self
        return NotImplemented


def _sum_optional_ints(left: int | None, right: int | None) -> int | None:
    if left is None and right is None:
        return None
    return (left or 0) + (right or 0)


@dataclass(slots=True)
class ResponseFormat:
    type: str
    json_schema: dict[str, Any] | None = None
    strict: bool = False


@dataclass(slots=True)
class Warning:
    message: str
    code: str | None = None


@dataclass(slots=True)
class RateLimitInfo:
    requests_remaining: int | None = None
    requests_limit: int | None = None
    tokens_remaining: int | None = None
    tokens_limit: int | None = None
    reset_at: Any | None = None


@dataclass(slots=True)
class Response:
    id: str = ""
    model: str = ""
    provider: str = ""
    message: Message = field(
        default_factory=lambda: Message(role=Role.ASSISTANT, content=[])
    )
    finish_reason: FinishReason = field(
        default_factory=lambda: FinishReason(reason=_FinishReasonValue.OTHER)
    )
    usage: Usage = field(default_factory=Usage)
    raw: Any | None = None
    warnings: list[Warning] = field(default_factory=list)
    rate_limit: RateLimitInfo | None = None

    def __post_init__(self) -> None:
        if not isinstance(self.finish_reason, FinishReason):
            self.finish_reason = _coerce_finish_reason_value(self.finish_reason)
        self.warnings = _coerce_optional_sequence(self.warnings, "warnings") or []

    @property
    def text(self) -> str:
        return self.message.text

    @property
    def tool_calls(self) -> list[ToolCall]:
        return [
            tool_call
            for part in self.message.content
            if part.kind == ContentKind.TOOL_CALL
            and (tool_call := _extract_tool_call_data(part)) is not None
        ]

    @property
    def reasoning(self) -> str | None:
        return _extract_reasoning_text(self.message.content)


@dataclass(slots=True)
class StreamEvent:
    type: StreamEventType | str
    delta: str | None = None
    text_id: str | None = None
    reasoning_delta: str | None = None
    tool_call: ToolCall | None = None
    finish_reason: FinishReason | None = None
    usage: Usage | None = None
    response: Response | None = None
    error: SDKError | None = None
    raw: Any | None = None

    def __post_init__(self) -> None:
        self.type = _coerce_stream_event_type(self.type)
        if self.finish_reason is not None and not isinstance(
            self.finish_reason,
            FinishReason,
        ):
            self.finish_reason = _coerce_finish_reason_value(self.finish_reason)
        if self.tool_call is not None:
            self.tool_call = _coerce_tool_call_value(self.tool_call)


def _coerce_stream_event_type(value: StreamEventType | str) -> StreamEventType | str:
    if isinstance(value, StreamEventType):
        return value
    if isinstance(value, str):
        try:
            return StreamEventType(value)
        except ValueError:
            return value
    raise TypeError("type must be a StreamEventType or string")


__all__ = [
    "AudioData",
    "ContentKind",
    "ContentPart",
    "DocumentData",
    "FinishReason",
    "ImageData",
    "Message",
    "RateLimitInfo",
    "Request",
    "Response",
    "ResponseFormat",
    "Role",
    "StreamEvent",
    "StreamEventType",
    "ThinkingData",
    "ToolCallData",
    "ToolResultData",
    "Usage",
    "Warning",
]
