from __future__ import annotations

import copy
import json
import logging
from collections.abc import Mapping, Sequence
from dataclasses import dataclass, field, replace
from typing import Any
from urllib.parse import urlsplit, urlunsplit

import httpx

from ..errors import InvalidRequestError, NetworkError, ProviderError, RequestTimeoutError
from ..provider_utils.errors import provider_error_from_response
from ..provider_utils.http import normalize_rate_limit
from ..provider_utils.media import prepare_openai_image_input
from ..provider_utils.normalization import (
    normalize_finish_reason,
    normalize_raw_payload,
    normalize_usage,
    normalize_warnings,
)
from ..tools import Tool, ToolCall, ToolChoice
from ..types import (
    ContentKind,
    ContentPart,
    FinishReason,
    Message,
    Response,
    Role,
    StreamEvent,
    StreamEventType,
)

logger = logging.getLogger(__name__)

_STRONGER_TERMINAL_FINISH_REASONS = {
    FinishReason.LENGTH.value,
    FinishReason.CONTENT_FILTER.value,
    FinishReason.ERROR.value,
    FinishReason.TOOL_CALLS.value,
}


def normalize_openai_compatible_base_url(base_url: str | None) -> str:
    text = (base_url or "").strip() or "https://api.openai.com"
    parts = urlsplit(text)
    path = parts.path.rstrip("/")
    if path.endswith("/chat/completions"):
        path = path[: -len("/chat/completions")]
    elif path.endswith("/responses"):
        path = path[: -len("/responses")]
    path = path.rstrip("/")
    if not path:
        path = "/v1"
    elif not path.endswith("/v1"):
        path = f"{path}/v1"
    return urlunsplit((parts.scheme, parts.netloc, path, parts.query, parts.fragment))


def build_openai_compatible_chat_completions_url(base_url: str | None) -> str:
    normalized = normalize_openai_compatible_base_url(base_url)
    parts = urlsplit(normalized)
    path = parts.path.rstrip("/")
    if not path.endswith("/chat/completions"):
        path = f"{path}/chat/completions"
    return urlunsplit((parts.scheme, parts.netloc, path, parts.query, parts.fragment))


def _coerce_text(value: Any, *, field_name: str) -> str | None:
    if value is None:
        return None
    if isinstance(value, str):
        text = value.strip()
        return text or None
    if isinstance(value, (bytes, bytearray)):
        try:
            text = bytes(value).decode("utf-8")
        except UnicodeDecodeError:
            logger.debug("Unable to decode %s as UTF-8", field_name, exc_info=True)
            text = bytes(value).decode("utf-8", errors="replace")
        text = text.strip()
        return text or None
    logger.debug("Unexpected %s type: %s", field_name, type(value).__name__)
    return None


def _serialize_json_value(value: Any) -> str:
    if isinstance(value, str):
        return value
    if isinstance(value, (bytes, bytearray)):
        try:
            return bytes(value).decode("utf-8")
        except UnicodeDecodeError:
            logger.debug("Unable to decode JSON value as UTF-8", exc_info=True)
            return bytes(value).decode("utf-8", errors="replace")
    try:
        return json.dumps(value, separators=(",", ":"), sort_keys=True)
    except Exception:
        logger.exception("Unexpected failure serializing OpenAI-compatible payload value")
        raise


def _deep_merge_mappings(
    base: Mapping[str, Any] | None,
    additions: Mapping[str, Any],
) -> dict[str, Any]:
    merged: dict[str, Any] = copy.deepcopy(dict(base or {}))
    for key, value in additions.items():
        existing = merged.get(key)
        if isinstance(existing, Mapping) and isinstance(value, Mapping):
            merged[key] = _deep_merge_mappings(existing, value)
        else:
            merged[key] = copy.deepcopy(value)
    return merged


def _tool_definition(tool: Tool) -> dict[str, Any]:
    function: dict[str, Any] = {
        "name": tool.name,
    }
    if tool.description is not None:
        function["description"] = tool.description
    if tool.parameters is not None:
        function["parameters"] = copy.deepcopy(tool.parameters)
    return {
        "type": "function",
        "function": function,
    }


def _tool_choice_value(tool_choice: ToolChoice) -> str | dict[str, Any]:
    if tool_choice.is_named:
        return {
            "type": "function",
            "function": {"name": tool_choice.tool_name},
        }
    return tool_choice.mode


def _content_part_to_chat_block(part: ContentPart) -> dict[str, Any]:
    if part.kind == ContentKind.TEXT:
        if part.text is None:
            raise InvalidRequestError(
                "OpenAI-compatible text content requires text",
                provider="openai_compatible",
            )
        return {"type": "text", "text": part.text}

    if part.kind == ContentKind.IMAGE:
        image = part.image
        if image is None:
            raise InvalidRequestError(
                "OpenAI-compatible image content requires an image payload",
                provider="openai_compatible",
            )
        block: dict[str, Any] = {
            "type": "image_url",
            "image_url": {"url": prepare_openai_image_input(image)},
        }
        if image.detail is not None:
            block["image_url"]["detail"] = image.detail
        return block

    raise InvalidRequestError(
        f"OpenAI-compatible adapter does not support content kind {part.kind}",
        provider="openai_compatible",
    )


def _serialize_tool_result_content(value: Any) -> str:
    if isinstance(value, str):
        return value
    if isinstance(value, (bytes, bytearray)):
        try:
            return bytes(value).decode("utf-8")
        except UnicodeDecodeError:
            logger.debug("Unable to decode tool result content as UTF-8", exc_info=True)
            return bytes(value).decode("utf-8", errors="replace")
    if isinstance(value, (dict, list)):
        return _serialize_json_value(value)
    try:
        return json.dumps(value, separators=(",", ":"), sort_keys=True)
    except Exception:
        logger.exception("Unexpected failure serializing OpenAI-compatible tool result")
        raise


def _collapse_text_blocks(
    text_blocks: list[dict[str, Any]],
    *,
    joiner: str = "",
) -> str | list[dict[str, Any]] | None:
    if not text_blocks:
        return None
    if len(text_blocks) == 1:
        return text_blocks[0]["text"]
    return joiner.join(block["text"] for block in text_blocks)


def _request_message_payload(message: Message) -> dict[str, Any]:
    if message.role in (Role.SYSTEM, Role.DEVELOPER):
        text_blocks: list[dict[str, Any]] = []
        for part in message.content:
            if part.kind != ContentKind.TEXT:
                raise InvalidRequestError(
                    (
                        "OpenAI-compatible adapter does not support "
                        "non-text system or developer content"
                    ),
                    provider="openai_compatible",
                )
            if part.text is not None:
                text_blocks.append({"type": "text", "text": part.text})

        content = _collapse_text_blocks(text_blocks, joiner="\n\n")
        payload: dict[str, Any] = {"role": message.role.value, "content": content or ""}
        if message.name is not None:
            payload["name"] = message.name
        return payload

    if message.role == Role.USER:
        blocks: list[dict[str, Any]] = []
        saw_image = False
        text_fragments: list[str] = []
        for part in message.content:
            if part.kind not in (ContentKind.TEXT, ContentKind.IMAGE):
                raise InvalidRequestError(
                    (
                        f"OpenAI-compatible adapter does not support content kind "
                        f"{part.kind} in user messages"
                    ),
                    provider="openai_compatible",
                )
            block = _content_part_to_chat_block(part)
            blocks.append(block)
            if block["type"] == "text" and isinstance(block.get("text"), str):
                text_fragments.append(block["text"])
            else:
                saw_image = True

        content: str | list[dict[str, Any]] = "".join(text_fragments) if not saw_image else blocks
        payload = {"role": message.role.value, "content": content or blocks}
        if message.name is not None:
            payload["name"] = message.name
        return payload

    if message.role == Role.ASSISTANT:
        text_blocks: list[dict[str, Any]] = []
        tool_calls: list[dict[str, Any]] = []

        for part in message.content:
            if part.kind == ContentKind.TEXT:
                if part.text is not None:
                    text_blocks.append({"type": "text", "text": part.text})
                continue

            if part.kind == ContentKind.TOOL_CALL:
                tool_call = part.tool_call
                if tool_call is None:
                    raise InvalidRequestError(
                        "OpenAI-compatible tool_call content requires a tool_call payload",
                        provider="openai_compatible",
                    )
                arguments = tool_call.raw_arguments
                if arguments is None:
                    arguments = tool_call.arguments
                if arguments is None:
                    arguments = "{}"
                elif not isinstance(arguments, str):
                    arguments = _serialize_json_value(arguments)
                tool_calls.append(
                    {
                        "id": tool_call.id,
                        "type": tool_call.type,
                        "function": {
                            "name": tool_call.name,
                            "arguments": arguments,
                        },
                    }
                )
                continue

            raise InvalidRequestError(
                (
                    f"OpenAI-compatible adapter does not support content kind "
                    f"{part.kind} in assistant messages"
                ),
                provider="openai_compatible",
            )

        payload: dict[str, Any] = {"role": message.role.value}
        content = _collapse_text_blocks(text_blocks)
        if content is not None:
            payload["content"] = content
        if tool_calls:
            payload["tool_calls"] = tool_calls
        if message.name is not None:
            payload["name"] = message.name
        return payload

    if message.role == Role.TOOL:
        call_id = message.tool_call_id
        tool_result_output: Any = None
        saw_tool_result = False
        text_blocks: list[dict[str, Any]] = []

        for part in message.content:
            if part.kind == ContentKind.TEXT:
                if part.text is not None:
                    text_blocks.append({"type": "text", "text": part.text})
                continue

            if part.kind == ContentKind.TOOL_RESULT:
                tool_result = part.tool_result
                if tool_result is None:
                    raise InvalidRequestError(
                        "OpenAI-compatible tool_result content requires a tool_result payload",
                        provider="openai_compatible",
                    )
                if call_id is None:
                    call_id = tool_result.tool_call_id
                elif call_id != tool_result.tool_call_id:
                    raise InvalidRequestError(
                        "OpenAI-compatible tool messages must use a single tool_call_id",
                        provider="openai_compatible",
                    )
                if saw_tool_result:
                    raise InvalidRequestError(
                        "OpenAI-compatible tool messages support only one tool_result payload",
                        provider="openai_compatible",
                    )
                tool_result_output = tool_result.content
                saw_tool_result = True
                continue

            raise InvalidRequestError(
                (
                    f"OpenAI-compatible adapter does not support content kind "
                    f"{part.kind} in tool messages"
                ),
                provider="openai_compatible",
            )

        if call_id is None:
            raise InvalidRequestError(
                "OpenAI-compatible tool messages require a tool_call_id",
                provider="openai_compatible",
            )

        if saw_tool_result and text_blocks:
            raise InvalidRequestError(
                "OpenAI-compatible tool messages cannot mix tool_result content with text content",
                provider="openai_compatible",
            )

        payload = {
            "role": message.role.value,
            "tool_call_id": call_id,
        }
        content = _collapse_text_blocks(text_blocks)
        if saw_tool_result:
            content = _serialize_tool_result_content(tool_result_output)
        elif content is None:
            content = ""
        payload["content"] = content
        if message.name is not None:
            payload["name"] = message.name
        return payload

    raise InvalidRequestError(
        f"OpenAI-compatible adapter does not support role {message.role}",
        provider="openai_compatible",
    )


def _provider_tools_from_provider_options(raw_tools: Any) -> list[dict[str, Any]]:
    if raw_tools is None:
        return []
    if not isinstance(raw_tools, Sequence) or isinstance(
        raw_tools,
        (str, bytes, bytearray),
    ):
        raise InvalidRequestError(
            "OpenAI-compatible provider_options['openai_compatible']['tools'] must be a sequence",
            provider="openai_compatible",
        )

    translated_tools: list[dict[str, Any]] = []
    for tool in raw_tools:
        if not isinstance(tool, Mapping):
            raise InvalidRequestError(
                (
                    "OpenAI-compatible provider_options['openai_compatible']"
                    "['tools'] entries must be mappings"
                ),
                provider="openai_compatible",
            )
        tool_type = _coerce_text(tool.get("type"), field_name="provider tool.type")
        if tool_type is not None and tool_type.casefold() != "function" and "function" not in tool:
            logger.warning(
                "OpenAI-compatible adapter ignores unsupported Responses tool type %s",
                tool_type,
            )
            continue
        translated_tools.append(copy.deepcopy(dict(tool)))
    return translated_tools


def _request_messages(messages: Sequence[Message]) -> list[dict[str, Any]]:
    return [_request_message_payload(message) for message in messages]


def build_openai_compatible_chat_completions_request(
    request: Any,
    *,
    provider_options: Mapping[str, Any] | None = None,
    stream: bool = False,
) -> tuple[dict[str, Any], dict[str, Any] | None]:
    body: dict[str, Any] = {
        "model": request.model,
        "messages": _request_messages(request.messages),
    }

    if request.response_format is not None:
        logger.warning(
            (
                "OpenAI-compatible adapter ignores request.response_format "
                "because Chat Completions structured output is not part of "
                "this adapter"
            ),
        )

    if request.reasoning_effort is not None:
        logger.warning(
            (
                "OpenAI-compatible adapter ignores reasoning_effort because "
                "Chat Completions does not expose reasoning token breakdowns"
            ),
        )

    if stream:
        body["stream"] = True

    native_provider_options = copy.deepcopy(dict(provider_options or {}))
    headers_overrides = native_provider_options.pop("headers", None)
    if headers_overrides is not None and not isinstance(headers_overrides, Mapping):
        raise InvalidRequestError(
            "OpenAI-compatible provider_options['openai_compatible']['headers'] must be a mapping",
            provider="openai_compatible",
        )

    raw_tools = native_provider_options.pop("tools", None)
    if native_provider_options:
        body = _deep_merge_mappings(body, native_provider_options)

    body["model"] = request.model
    body["messages"] = _request_messages(request.messages)

    if request.tools is not None:
        body["tools"] = [_tool_definition(tool) for tool in request.tools]
    if request.tool_choice is not None:
        body["tool_choice"] = _tool_choice_value(request.tool_choice)
    if request.temperature is not None:
        body["temperature"] = request.temperature
    if request.top_p is not None:
        body["top_p"] = request.top_p
    if request.max_tokens is not None:
        body["max_tokens"] = request.max_tokens
    if request.stop_sequences is not None:
        body["stop"] = list(request.stop_sequences)
    if request.metadata is not None:
        body["metadata"] = dict(request.metadata)
    if stream:
        body["stream"] = True
    else:
        body.pop("stream", None)

    if raw_tools is not None:
        translated_tools = _provider_tools_from_provider_options(raw_tools)
        if translated_tools:
            merged_tools = list(body.get("tools") or [])
            merged_tools.extend(translated_tools)
            body["tools"] = merged_tools

    return body, copy.deepcopy(dict(headers_overrides)) if headers_overrides is not None else None


def _message_content_parts(content: Any) -> list[ContentPart]:
    if content is None:
        return []
    if isinstance(content, ContentPart):
        return [content]
    if isinstance(content, (str, bytes, bytearray)):
        text = _coerce_text(content, field_name="response.content")
        if text is None:
            return []
        return [ContentPart(kind=ContentKind.TEXT, text=text)]
    if not isinstance(content, Sequence):
        text = _coerce_text(content, field_name="response.content")
        if text is None:
            return []
        return [ContentPart(kind=ContentKind.TEXT, text=text)]

    parts: list[ContentPart] = []
    for item in content:
        if isinstance(item, ContentPart):
            parts.append(item)
            continue
        if not isinstance(item, Mapping):
            text = _coerce_text(item, field_name="response.content_item")
            if text is not None:
                parts.append(ContentPart(kind=ContentKind.TEXT, text=text))
            continue

        item_type = _coerce_text(item.get("type"), field_name="response.content_item.type")
        if item_type in {"text", "output_text", "input_text"}:
            text = _coerce_text(
                item.get("text") or item.get("content") or item.get("delta"),
                field_name="response.content_item.text",
            )
            if text is not None:
                parts.append(ContentPart(kind=ContentKind.TEXT, text=text))
            continue
        if item_type in {"image_url", "input_image"}:
            logger.warning(
                "OpenAI-compatible adapter ignores unsupported assistant image content",
            )
            continue

        text = _coerce_text(
            item.get("text") or item.get("content"), field_name="response.content_item.text"
        )
        if text is not None:
            parts.append(ContentPart(kind=ContentKind.TEXT, text=text))
    return parts


def _tool_call_from_payload(payload: Mapping[str, Any]) -> ContentPart:
    function = payload.get("function")
    if function is not None and not isinstance(function, Mapping):
        raise ProviderError(
            "unexpected non-object tool call payload",
            provider="openai_compatible",
            raw=payload,
            retryable=False,
        )

    call_id = _coerce_text(
        payload.get("id") or payload.get("call_id") or payload.get("tool_call_id"),
        field_name="response.tool_call.id",
    )
    if call_id is None:
        call_id = "openai_call"

    name = _coerce_text(
        (function.get("name") if isinstance(function, Mapping) else None) or payload.get("name"),
        field_name="response.tool_call.name",
    )
    if name is None:
        name = "tool"

    raw_arguments = None
    arguments = None
    if isinstance(function, Mapping):
        raw_arguments = (
            function.get("arguments") if isinstance(function.get("arguments"), str) else None
        )
        arguments = function.get("arguments")
        if arguments is None:
            arguments = function.get("input")
    if arguments is None:
        arguments = payload.get("arguments")
    if raw_arguments is None and isinstance(payload.get("arguments"), str):
        raw_arguments = payload.get("arguments")
    if arguments is None:
        arguments = "{}"
    if raw_arguments is None:
        if isinstance(arguments, str):
            raw_arguments = arguments
        else:
            raw_arguments = _serialize_json_value(arguments)
    if not isinstance(arguments, str):
        arguments = _serialize_json_value(arguments)

    tool_type = (
        _coerce_text(payload.get("type"), field_name="response.tool_call.type") or "function"
    )
    return ContentPart(
        kind=ContentKind.TOOL_CALL,
        tool_call=ToolCall(
            id=call_id,
            name=name,
            arguments=arguments,
            raw_arguments=raw_arguments,
            type=tool_type,
        ),
    )


def _message_content_parts_from_payload(message_payload: Any) -> list[ContentPart]:
    if not isinstance(message_payload, Mapping):
        return _message_content_parts(message_payload)

    parts = _message_content_parts(message_payload.get("content"))
    tool_calls = message_payload.get("tool_calls")
    if tool_calls is None:
        function_call = message_payload.get("function_call")
        if function_call is not None:
            if isinstance(function_call, Mapping):
                parts.append(_tool_call_from_payload(function_call))
            else:
                raise ProviderError(
                    "unexpected non-object function_call payload",
                    provider="openai_compatible",
                    raw=message_payload,
                    retryable=False,
                )
        return parts

    if isinstance(tool_calls, Mapping):
        tool_calls = [tool_calls]
    if not isinstance(tool_calls, Sequence) or isinstance(tool_calls, (str, bytes, bytearray)):
        raise ProviderError(
            "unexpected non-sequence tool_calls payload",
            provider="openai_compatible",
            raw=message_payload,
            retryable=False,
        )

    for tool_call in tool_calls:
        if not isinstance(tool_call, Mapping):
            raise ProviderError(
                "unexpected non-object tool call payload",
                provider="openai_compatible",
                raw=message_payload,
                retryable=False,
            )
        parts.append(_tool_call_from_payload(tool_call))
    return parts


def _normalize_chat_finish_reason(value: Any) -> FinishReason:
    if isinstance(value, str) and value.strip().casefold() == "function_call":
        return FinishReason(reason=FinishReason.TOOL_CALLS, raw=value)
    finish_reason = normalize_finish_reason(value, provider="openai_compatible")
    if isinstance(value, str) and finish_reason.raw is None:
        return FinishReason(reason=finish_reason.reason, raw=value)
    return finish_reason


def _normalize_chat_usage(value: Any) -> Any:
    usage = normalize_usage(value, provider="openai_compatible", raw=value)
    return replace(usage, reasoning_tokens=None)


def _response_from_payload(
    source: Mapping[str, Any],
    *,
    provider: str,
    headers: Mapping[str, Any] | Any | None,
    raw: Any,
) -> Response:
    response_id = _coerce_text(source.get("id"), field_name="response.id")
    model = _coerce_text(source.get("model"), field_name="response.model")
    content_parts: list[ContentPart] = []
    finish_reason = FinishReason(reason=FinishReason.OTHER)
    usage = _normalize_chat_usage(source.get("usage"))
    warnings = normalize_warnings(source.get("warnings"))

    choices = source.get("choices")
    if (
        isinstance(choices, Sequence)
        and not isinstance(choices, (str, bytes, bytearray))
        and choices
    ):
        choice = choices[0]
        if not isinstance(choice, Mapping):
            raise TypeError("choices must contain mappings")

        if response_id is None:
            response_id = _coerce_text(choice.get("id"), field_name="response.id")
        if model is None:
            model = _coerce_text(choice.get("model"), field_name="response.model")
        message_payload = choice.get("message")
        if message_payload is None and any(
            key in choice for key in ("content", "tool_calls", "function_call", "role")
        ):
            message_payload = choice
        if message_payload is not None:
            content_parts = _message_content_parts_from_payload(message_payload)
            if content_parts and any(part.kind == ContentKind.TOOL_CALL for part in content_parts):
                finish_reason = _normalize_chat_finish_reason(choice.get("finish_reason"))
                if finish_reason.reason not in _STRONGER_TERMINAL_FINISH_REASONS:
                    finish_reason = FinishReason(
                        reason=FinishReason.TOOL_CALLS,
                        raw=finish_reason.raw,
                    )
            else:
                finish_reason = _normalize_chat_finish_reason(choice.get("finish_reason"))
        else:
            finish_reason = _normalize_chat_finish_reason(choice.get("finish_reason"))
    elif isinstance(source, Mapping) and any(
        key in source for key in ("content", "tool_calls", "function_call", "role")
    ):
        content_parts = _message_content_parts_from_payload(source)
        finish_reason = _normalize_chat_finish_reason(source.get("finish_reason"))
        if content_parts and any(part.kind == ContentKind.TOOL_CALL for part in content_parts):
            if finish_reason.reason not in _STRONGER_TERMINAL_FINISH_REASONS:
                finish_reason = FinishReason(
                    reason=FinishReason.TOOL_CALLS,
                    raw=finish_reason.raw,
                )
    else:
        raise TypeError("OpenAI-compatible response is missing choices")

    return Response(
        id=response_id or "",
        model=model or "",
        provider=provider,
        message=Message(role=Role.ASSISTANT, content=content_parts),
        finish_reason=finish_reason,
        usage=usage,
        raw=raw,
        warnings=warnings,
        rate_limit=normalize_rate_limit(headers),
    )


def normalize_openai_compatible_response(
    payload: Any,
    *,
    provider: str = "openai_compatible",
    headers: Mapping[str, Any] | Any | None = None,
    raw: Any = None,
) -> Response:
    raw_payload = raw if raw is not None else payload
    try:
        source = normalize_raw_payload(payload)

        if isinstance(source, str):
            text = source.strip()
            parts = [ContentPart(kind=ContentKind.TEXT, text=text)] if text else []
            return Response(
                provider=provider,
                message=Message(role=Role.ASSISTANT, content=parts),
                finish_reason=FinishReason(reason=FinishReason.OTHER),
                usage=_normalize_chat_usage(None),
                raw=raw_payload,
                warnings=[],
                rate_limit=normalize_rate_limit(headers),
            )

        if isinstance(source, Mapping):
            return _response_from_payload(
                source, provider=provider, headers=headers, raw=raw_payload
            )

        text = _coerce_text(source, field_name="response")
        if text is None:
            return Response(
                provider=provider,
                raw=raw_payload,
                warnings=[],
                rate_limit=normalize_rate_limit(headers),
            )

        return Response(
            provider=provider,
            message=Message(
                role=Role.ASSISTANT, content=[ContentPart(kind=ContentKind.TEXT, text=text)]
            ),
            finish_reason=FinishReason(reason=FinishReason.OTHER),
            usage=_normalize_chat_usage(None),
            raw=raw_payload,
            warnings=[],
            rate_limit=normalize_rate_limit(headers),
        )
    except ProviderError:
        raise
    except Exception as exc:
        logger.exception("Unexpected failure normalizing OpenAI-compatible response")
        raise ProviderError(
            "failed to normalize OpenAI-compatible response",
            provider=provider,
            raw=raw_payload,
            cause=exc,
            retryable=False,
        ) from exc


def _provider_error_from_httpx_error(
    error: httpx.HTTPError,
    *,
    provider: str,
) -> Exception:
    if isinstance(error, httpx.HTTPStatusError):
        response = getattr(error, "response", None)
        if response is not None:
            raw = normalize_raw_payload(response.text)
            return provider_error_from_response(
                response,
                provider=provider,
                raw=raw,
                cause=error,
            )

    if isinstance(error, httpx.TimeoutException):
        message = str(error).strip() or f"{provider} request timed out"
        return RequestTimeoutError(message, provider=provider, cause=error)

    message = str(error).strip() or f"{provider} network error"
    return NetworkError(message, provider=provider, cause=error)


@dataclass(slots=True)
class _StreamState:
    provider: str
    headers: Mapping[str, Any] | None
    response_id: str | None = None
    model: str | None = None
    content_parts: list[ContentPart] = field(default_factory=list)
    active_text: list[str] | None = None
    active_tool_calls: dict[int, dict[str, Any]] = field(default_factory=dict)
    tool_call_order: list[int] = field(default_factory=list)
    finish_reason: FinishReason | None = None
    usage: Any | None = None
    started: bool = False
    last_payload: Any = None

    def _emit_start_response(self, raw_payload: Any) -> Response:
        return Response(
            id=self.response_id or "",
            model=self.model or "",
            provider=self.provider,
            finish_reason=FinishReason(reason=FinishReason.OTHER),
            usage=self.usage if self.usage is not None else _normalize_chat_usage(None),
            raw=raw_payload,
            rate_limit=normalize_rate_limit(self.headers),
        )

    def _emit_final_response(self, raw_payload: Any, finish_reason: FinishReason) -> Response:
        return Response(
            id=self.response_id or "",
            model=self.model or "",
            provider=self.provider,
            message=Message(role=Role.ASSISTANT, content=list(self.content_parts)),
            finish_reason=finish_reason,
            usage=self.usage if self.usage is not None else _normalize_chat_usage(None),
            raw=raw_payload,
            rate_limit=normalize_rate_limit(self.headers),
        )

    def _close_text(self, raw: Any) -> list[StreamEvent]:
        if self.active_text is not None:
            text = "".join(self.active_text)
            self.active_text = None
            if text:
                self.content_parts.append(ContentPart(kind=ContentKind.TEXT, text=text))
            return [StreamEvent(type=StreamEventType.TEXT_END, delta=text or None, raw=raw)]
        return []

    def _close_tool_calls(self, raw: Any) -> list[StreamEvent]:
        events: list[StreamEvent] = []
        if not self.active_tool_calls:
            return events

        for index in list(self.tool_call_order):
            tool_call = self.active_tool_calls.pop(index, None)
            if tool_call is None:
                continue
            parsed_tool_call = ToolCall(**tool_call)
            self.content_parts.append(
                ContentPart(kind=ContentKind.TOOL_CALL, tool_call=parsed_tool_call),
            )
            events.append(
                StreamEvent(
                    type=StreamEventType.TOOL_CALL_END,
                    tool_call=parsed_tool_call,
                    raw=raw,
                )
            )
        self.tool_call_order = []
        return events

    def _finalize_active_blocks(self, raw: Any) -> list[StreamEvent]:
        events: list[StreamEvent] = []
        events.extend(self._close_text(raw))
        events.extend(self._close_tool_calls(raw))
        return events

    def _start_text(self, fragment: str, raw: Any) -> list[StreamEvent]:
        events: list[StreamEvent] = []
        events.extend(self._close_tool_calls(raw))

        started = self.active_text is None
        if started:
            self.active_text = []
        self.active_text.append(fragment)
        if started:
            events.append(
                StreamEvent(
                    type=StreamEventType.TEXT_START,
                    raw=raw,
                )
            )
        events.append(
            StreamEvent(
                type=StreamEventType.TEXT_DELTA,
                delta=fragment,
                raw=raw,
            )
        )
        return events

    def _start_tool_call(
        self,
        *,
        index: int,
        tool_call_id: str | None,
        name: str | None,
        fragment: str,
        raw: Any,
        tool_type: str | None = None,
    ) -> list[StreamEvent]:
        events: list[StreamEvent] = []
        events.extend(self._close_text(raw))

        started = index not in self.active_tool_calls
        if started:
            self.active_tool_calls[index] = {
                "id": tool_call_id or f"chatcmpl_call_{index}",
                "name": name or "tool",
                "arguments": "",
                "raw_arguments": "",
                "type": tool_type or "function",
            }
            self.tool_call_order.append(index)
        else:
            current = self.active_tool_calls[index]
            if tool_call_id and current.get("id") in (None, "", f"chatcmpl_call_{index}"):
                current["id"] = tool_call_id
            if name and current.get("name") in (None, "tool"):
                current["name"] = name
            if tool_type and current.get("type") in (None, "function"):
                current["type"] = tool_type

        current = self.active_tool_calls[index]
        current_arguments = current.get("arguments")
        if not isinstance(current_arguments, str):
            current_arguments = ""
        current_raw_arguments = current.get("raw_arguments")
        if not isinstance(current_raw_arguments, str):
            current_raw_arguments = current_arguments
        current["arguments"] = current_arguments + fragment
        current["raw_arguments"] = current_raw_arguments + fragment
        resolved_tool_call_id = current.get("id") or tool_call_id or f"chatcmpl_call_{index}"
        resolved_name = current.get("name") or name or "tool"
        resolved_type = current.get("type") or tool_type or "function"
        if started:
            events.append(
                StreamEvent(
                    type=StreamEventType.TOOL_CALL_START,
                    tool_call=ToolCall(
                        id=resolved_tool_call_id,
                        name=resolved_name,
                        arguments="",
                        raw_arguments="",
                        type=resolved_type,
                    ),
                    raw=raw,
                )
            )
        events.append(
            StreamEvent(
                type=StreamEventType.TOOL_CALL_DELTA,
                tool_call=ToolCall(
                    id=resolved_tool_call_id,
                    name=resolved_name,
                    arguments=fragment,
                    raw_arguments=fragment,
                    type=resolved_type,
                ),
                raw=raw,
            )
        )
        return events

    def _normalize_stream_finish_reason(self, choice_finish_reason: Any) -> FinishReason:
        finish_reason = _normalize_chat_finish_reason(choice_finish_reason)
        if self.tool_call_order or any(
            part.kind == ContentKind.TOOL_CALL for part in self.content_parts
        ):
            if finish_reason.reason not in _STRONGER_TERMINAL_FINISH_REASONS:
                finish_reason = FinishReason(
                    reason=FinishReason.TOOL_CALLS,
                    raw=finish_reason.raw,
                )
        return finish_reason

    def _choice_payload(self, payload: Mapping[str, Any]) -> Mapping[str, Any] | None:
        choices = payload.get("choices")
        if (
            isinstance(choices, Sequence)
            and not isinstance(choices, (str, bytes, bytearray))
            and choices
        ):
            choice = choices[0]
            if isinstance(choice, Mapping):
                return choice
        return None

    def _update_usage(self, payload: Mapping[str, Any]) -> None:
        usage_payload = payload.get("usage")
        if usage_payload is not None:
            self.usage = _normalize_chat_usage(usage_payload)

    def translate(self, payload: Any) -> list[StreamEvent]:
        if isinstance(payload, str):
            text = payload.strip()
            if text == "[DONE]":
                if self.started or self.last_payload is not None:
                    if self.finish_reason is None:
                        if self.tool_call_order:
                            self.finish_reason = FinishReason(reason=FinishReason.TOOL_CALLS)
                        else:
                            self.finish_reason = FinishReason(reason=FinishReason.STOP)
                    return self.finalize()
                return []
            if text:
                logger.warning(
                    "OpenAI-compatible adapter received unexpected plain-text stream payload",
                )
                return [StreamEvent(type=StreamEventType.PROVIDER_EVENT, raw=payload)]
            return []

        if not isinstance(payload, Mapping):
            logger.debug(
                "Unexpected OpenAI-compatible stream payload type: %s", type(payload).__name__
            )
            return [StreamEvent(type=StreamEventType.PROVIDER_EVENT, raw=payload)]

        self.last_payload = payload
        payload_type = _coerce_text(payload.get("type"), field_name="stream.payload.type")
        if isinstance(payload_type, str) and payload_type.casefold().startswith("response."):
            logger.warning(
                "OpenAI-compatible adapter received unsupported Responses stream event %s",
                payload_type,
            )
            return [StreamEvent(type=StreamEventType.PROVIDER_EVENT, raw=payload)]

        choice = self._choice_payload(payload)
        if choice is None:
            self._update_usage(payload)
            return []

        if not self.started:
            self.started = True
            start_event = StreamEvent(
                type=StreamEventType.STREAM_START,
                response=self._emit_start_response(payload),
                raw=payload,
            )
            events: list[StreamEvent] = [start_event]
        else:
            events = []

        self._update_usage(payload)
        if response_id := _coerce_text(payload.get("id"), field_name="response.id"):
            self.response_id = self.response_id or response_id
        if model := _coerce_text(payload.get("model"), field_name="response.model"):
            self.model = self.model or model

        delta = choice.get("delta")
        if isinstance(delta, Mapping):
            if content := delta.get("content"):
                fragment = _coerce_text(content, field_name="stream.delta.content")
                if fragment is not None:
                    events.extend(self._start_text(fragment, payload))

            tool_calls = delta.get("tool_calls")
            if tool_calls is not None:
                if isinstance(tool_calls, Mapping):
                    tool_calls = [tool_calls]
                if not isinstance(tool_calls, Sequence) or isinstance(
                    tool_calls, (str, bytes, bytearray)
                ):
                    raise ProviderError(
                        "unexpected non-sequence tool_calls delta",
                        provider=self.provider,
                        raw=payload,
                        retryable=False,
                    )
                for tool_call in tool_calls:
                    if not isinstance(tool_call, Mapping):
                        raise ProviderError(
                            "unexpected non-object tool call delta",
                            provider=self.provider,
                            raw=payload,
                            retryable=False,
                        )
                    index = tool_call.get("index", 0)
                    if not isinstance(index, int):
                        try:
                            index = int(index)
                        except (TypeError, ValueError) as exc:
                            raise ProviderError(
                                "unexpected tool_call index",
                                provider=self.provider,
                                raw=payload,
                                retryable=False,
                            ) from exc
                    tool_call_id = _coerce_text(
                        tool_call.get("id")
                        or tool_call.get("call_id")
                        or tool_call.get("tool_call_id"),
                        field_name="stream.tool_call.id",
                    )
                    tool_type = _coerce_text(
                        tool_call.get("type"), field_name="stream.tool_call.type"
                    )
                    function = tool_call.get("function")
                    if function is not None and not isinstance(function, Mapping):
                        raise ProviderError(
                            "unexpected non-object tool call function delta",
                            provider=self.provider,
                            raw=payload,
                            retryable=False,
                        )
                    name = _coerce_text(
                        (function.get("name") if isinstance(function, Mapping) else None)
                        or tool_call.get("name"),
                        field_name="stream.tool_call.name",
                    )
                    fragment = None
                    if isinstance(function, Mapping):
                        fragment = function.get("arguments")
                        if fragment is None:
                            fragment = function.get("input")
                    if fragment is None:
                        fragment = tool_call.get("arguments")
                    if fragment is None:
                        fragment = tool_call.get("input")
                    fragment_text = _coerce_text(fragment, field_name="stream.tool_call.arguments")
                    if fragment_text is None:
                        fragment_text = ""
                    events.extend(
                        self._start_tool_call(
                            index=index,
                            tool_call_id=tool_call_id,
                            name=name,
                            fragment=fragment_text,
                            raw=payload,
                            tool_type=tool_type,
                        )
                    )

        finish_reason = choice.get("finish_reason")
        if finish_reason is not None:
            self.finish_reason = self._normalize_stream_finish_reason(finish_reason)
            events.extend(self._finalize_active_blocks(payload))
            response = self._emit_final_response(payload, self.finish_reason)
            events.append(
                StreamEvent(
                    type=StreamEventType.FINISH,
                    finish_reason=response.finish_reason,
                    usage=response.usage,
                    response=response,
                    raw=payload,
                )
            )
            return events

        return events

    def finalize(self) -> list[StreamEvent]:
        if self.finish_reason is None:
            if self.tool_call_order:
                self.finish_reason = FinishReason(reason=FinishReason.TOOL_CALLS)
            else:
                self.finish_reason = FinishReason(reason=FinishReason.STOP)
        events = self._finalize_active_blocks(self.last_payload)
        response = self._emit_final_response(self.last_payload, self.finish_reason)
        self.started = True
        events.append(
            StreamEvent(
                type=StreamEventType.FINISH,
                finish_reason=response.finish_reason,
                usage=response.usage,
                response=response,
                raw=self.last_payload,
            )
        )
        return events


__all__ = [
    "build_openai_compatible_chat_completions_request",
    "build_openai_compatible_chat_completions_url",
    "normalize_openai_compatible_base_url",
    "normalize_openai_compatible_response",
    "_provider_error_from_httpx_error",
    "_StreamState",
]
