from __future__ import annotations

import copy
import inspect
import json
import logging
import os
from collections.abc import AsyncIterator, Mapping, Sequence
from dataclasses import dataclass, field, replace
from typing import Any

import httpx

from ..errors import (
    ConfigurationError,
    InvalidRequestError,
    NetworkError,
    ProviderError,
    RequestTimeoutError,
)
from ..provider_utils.errors import provider_error_from_response
from ..provider_utils.http import provider_options_for
from ..provider_utils.media import prepare_openai_image_input
from ..provider_utils.normalization import normalize_raw_payload
from ..provider_utils.openai import (
    build_openai_responses_url,
    normalize_openai_base_url,
    normalize_openai_response,
)
from ..provider_utils.sse import aiter_sse_events
from ..tools import Tool, ToolCall, ToolChoice
from ..types import (
    ContentKind,
    ContentPart,
    FinishReason,
    Message,
    Response,
    ResponseFormat,
    Role,
    StreamEvent,
    StreamEventType,
    ToolCallData,
)

logger = logging.getLogger(__name__)


def _env_value(name: str) -> str | None:
    value = os.environ.get(name)
    if value is None:
        return None
    value = value.strip()
    if not value:
        return None
    return value


def _coerce_identifier(value: Any) -> str | None:
    if value is None:
        return None
    if isinstance(value, str):
        text = value.strip()
        return text or None
    return str(value)


def _text_from_payload(payload: Any, *keys: str) -> str | None:
    if not isinstance(payload, Mapping):
        return None
    for key in keys:
        value = payload.get(key)
        if value is None:
            continue
        if isinstance(value, str):
            text = value.strip()
            if text:
                return text
            continue
        if isinstance(value, (bytes, bytearray)):
            try:
                text = bytes(value).decode("utf-8")
            except UnicodeDecodeError:
                logger.debug("Unable to decode OpenAI stream payload as UTF-8", exc_info=True)
                text = bytes(value).decode("utf-8", errors="replace")
            text = text.strip()
            if text:
                return text
            continue
        text = _coerce_identifier(value)
        if text:
            return text
    return None


def _item_from_payload(payload: Any) -> Any:
    if not isinstance(payload, Mapping):
        return payload
    for key in ("item", "output_item"):
        item = payload.get(key)
        if item is not None:
            return item
    return payload


def _tool_call_has_placeholder_arguments(tool_call: ToolCall) -> bool:
    arguments = tool_call.arguments
    if isinstance(arguments, dict):
        if arguments:
            return False
        raw_arguments = tool_call.raw_arguments
        if isinstance(raw_arguments, str):
            return raw_arguments.strip() in {"", "{}"}
        return True
    if isinstance(arguments, str):
        return arguments.strip() in {"", "{}"}
    return arguments is None


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
        logger.exception("Unexpected failure serializing OpenAI payload value")
        raise


def _tool_definition(tool: Tool) -> dict[str, Any]:
    definition: dict[str, Any] = {
        "type": "function",
        "name": tool.name,
        "strict": True,
    }
    if tool.description is not None:
        definition["description"] = tool.description
    if tool.parameters is not None:
        definition["parameters"] = copy.deepcopy(tool.parameters)
    return definition


def _tool_choice_value(tool_choice: ToolChoice) -> str | dict[str, Any]:
    if tool_choice.is_named:
        return {"type": "function", "name": tool_choice.tool_name}
    return tool_choice.mode


def _response_format_value(response_format: ResponseFormat) -> dict[str, Any]:
    if not isinstance(response_format.type, str):
        logger.warning("OpenAI response_format type must be a string")
        raise InvalidRequestError(
            "OpenAI response_format type must be a string",
            provider="openai",
        )
    if response_format.type.casefold() != "json_schema":
        logger.warning("OpenAI response_format only supports json_schema")
        raise InvalidRequestError(
            "OpenAI response_format only supports json_schema",
            provider="openai",
        )
    if response_format.json_schema is None:
        logger.warning("OpenAI json_schema response_format requires a json_schema")
        raise InvalidRequestError(
            "OpenAI json_schema response_format requires a json_schema",
            provider="openai",
        )
    if not isinstance(response_format.json_schema, Mapping):
        logger.warning(
            "OpenAI json_schema response_format requires a mapping schema",
        )
        raise InvalidRequestError(
            "OpenAI json_schema response_format requires a mapping schema",
            provider="openai",
        )
    return {
        "type": "json_schema",
        "json_schema": copy.deepcopy(response_format.json_schema),
        "strict": response_format.strict,
    }


def _message_content_item(
    part: ContentPart,
    *,
    role: Role,
) -> dict[str, Any] | None:
    if part.kind == ContentKind.TEXT:
        text = part.text
        if text is None:
            return None
        content_type = "output_text" if role == Role.ASSISTANT else "input_text"
        return {
            "type": content_type,
            "text": text,
        }

    if part.kind == ContentKind.IMAGE:
        image = part.image
        if image is None:
            raise InvalidRequestError(
                "OpenAI image content requires an image payload",
                provider="openai",
            )
        item: dict[str, Any] = {
            "type": "input_image",
            "image_url": prepare_openai_image_input(image),
        }
        if image.detail is not None:
            item["detail"] = image.detail
        return item

    if part.kind in (ContentKind.THINKING, ContentKind.REDACTED_THINKING):
        logger.warning(
            "OpenAI Responses adapter does not support reasoning content kind %s in %s messages",
            part.kind,
            role.value,
        )
        raise InvalidRequestError(
            f"OpenAI Responses adapter does not support reasoning content kind {part.kind}",
            provider="openai",
        )

    raise InvalidRequestError(
        f"OpenAI Responses adapter does not support content kind {part.kind}",
        provider="openai",
    )


def _function_call_input_item(
    tool_call: ToolCall | ToolCallData,
) -> dict[str, Any]:
    call_id = getattr(tool_call, "id", None)
    if not isinstance(call_id, str) or not call_id:
        raise InvalidRequestError(
            "OpenAI tool_call content requires a string id",
            provider="openai",
        )

    name = getattr(tool_call, "name", None)
    if not isinstance(name, str) or not name:
        raise InvalidRequestError(
            "OpenAI tool_call content requires a string name",
            provider="openai",
        )

    raw_arguments = getattr(tool_call, "raw_arguments", None)
    if raw_arguments is not None:
        if not isinstance(raw_arguments, str):
            raise InvalidRequestError(
                "OpenAI tool_call raw_arguments must be a string or None",
                provider="openai",
            )
        arguments = raw_arguments
    else:
        arguments = getattr(tool_call, "arguments", None)
        if arguments is None:
            raise InvalidRequestError(
                "OpenAI tool_call content requires arguments",
                provider="openai",
            )
        if not isinstance(arguments, str):
            try:
                arguments = _serialize_json_value(arguments)
            except Exception as exc:
                raise InvalidRequestError(
                    "OpenAI tool_call arguments must be JSON-serializable",
                    provider="openai",
                ) from exc

    return {
        "type": "function_call",
        "id": call_id,
        "name": name,
        "arguments": arguments,
    }


def _instruction_fragment_from_part(part: ContentPart, *, role: Role) -> str | None:
    if part.kind == ContentKind.TEXT:
        return part.text
    if part.kind in (ContentKind.THINKING, ContentKind.REDACTED_THINKING):
        if part.text is not None:
            return part.text
        if part.thinking is not None:
            return part.thinking.text
        logger.warning(
            "OpenAI Responses adapter does not support empty reasoning content in %s messages",
            role.value,
        )
        raise InvalidRequestError(
            (
                "OpenAI Responses adapter does not support empty reasoning content "
                f"in {role.value} messages"
            ),
            provider="openai",
        )
    logger.warning(
        "OpenAI Responses adapter does not support content kind %s in %s messages",
        part.kind,
        role.value,
    )
    raise InvalidRequestError(
        (
            "OpenAI Responses adapter does not support content kind "
            f"{part.kind} in {role.value} messages"
        ),
        provider="openai",
    )


def _extract_instructions(messages: Sequence[Message]) -> str | None:
    fragments: list[str] = []
    for message in messages:
        if message.role not in (Role.SYSTEM, Role.DEVELOPER):
            continue
        for part in message.content:
            fragment = _instruction_fragment_from_part(part, role=message.role)
            if fragment:
                fragments.append(fragment)
    if not fragments:
        return None
    return "\n\n".join(fragments)


def _tool_message_output_item(message: Message) -> dict[str, Any]:
    call_id = message.tool_call_id
    tool_result_output: Any = None
    saw_tool_result = False
    text_fragments: list[str] = []

    for part in message.content:
        if part.kind == ContentKind.TEXT:
            if part.text is not None:
                text_fragments.append(part.text)
            continue

        if part.kind == ContentKind.TOOL_RESULT:
            tool_result = part.tool_result
            if tool_result is None:
                raise InvalidRequestError(
                    "OpenAI tool_result content requires a tool_result payload",
                    provider="openai",
                )
            if call_id is None:
                call_id = tool_result.tool_call_id
            elif call_id != tool_result.tool_call_id:
                raise InvalidRequestError(
                    "OpenAI tool messages must use a single tool_call_id",
                    provider="openai",
                )
            if saw_tool_result:
                raise InvalidRequestError(
                    "OpenAI tool messages support only one tool_result payload",
                    provider="openai",
                )
            tool_result_output = tool_result.content
            saw_tool_result = True
            continue

        raise InvalidRequestError(
            f"OpenAI Responses adapter does not support content kind {part.kind} in tool messages",
            provider="openai",
        )

    if call_id is None:
        raise InvalidRequestError(
            "OpenAI tool messages require a tool_call_id",
            provider="openai",
        )

    if not saw_tool_result and not text_fragments:
        raise InvalidRequestError(
            "OpenAI tool messages require text or tool_result content",
            provider="openai",
        )

    if saw_tool_result and text_fragments:
        raise InvalidRequestError(
            "OpenAI tool messages cannot mix tool_result content with text content",
            provider="openai",
        )

    if saw_tool_result:
        output = tool_result_output
    else:
        output = "".join(text_fragments)

    if isinstance(output, list):
        output = copy.deepcopy(output)
    elif isinstance(output, dict):
        output = _serialize_json_value(output)
    elif not isinstance(output, str):
        output = _serialize_json_value(output)

    return {
        "type": "function_call_output",
        "call_id": call_id,
        "output": output,
    }


def _message_input_items(message: Message) -> list[dict[str, Any]]:
    if message.role == Role.TOOL:
        return [_tool_message_output_item(message)]

    items: list[dict[str, Any]] = []
    content: list[dict[str, Any]] = []

    def flush_message() -> None:
        if not content:
            return
        item: dict[str, Any] = {
            "type": "message",
            "role": message.role.value,
            "content": list(content),
        }
        if message.name is not None:
            item["name"] = message.name
        items.append(item)
        content.clear()

    for part in message.content:
        if part.kind == ContentKind.TOOL_CALL:
            flush_message()
            tool_call = part.tool_call
            if tool_call is None:
                raise InvalidRequestError(
                    "OpenAI tool_call content requires a tool_call payload",
                    provider="openai",
                )
            items.append(_function_call_input_item(tool_call))
            continue

        if part.kind == ContentKind.TOOL_RESULT:
            flush_message()
            tool_result = part.tool_result
            if tool_result is None:
                raise InvalidRequestError(
                    "OpenAI tool_result content requires a tool_result payload",
                    provider="openai",
                )
            output = tool_result.content
            if isinstance(output, list):
                output = copy.deepcopy(output)
            elif isinstance(output, dict):
                output = _serialize_json_value(output)
            elif not isinstance(output, str):
                output = _serialize_json_value(output)
            items.append(
                {
                    "type": "function_call_output",
                    "call_id": tool_result.tool_call_id,
                    "output": output,
                }
            )
            continue

        content_item = _message_content_item(part, role=message.role)
        if content_item is not None:
            content.append(content_item)

    flush_message()
    return items


def _native_provider_options(
    provider_options: Mapping[str, Any] | None,
) -> dict[str, Any] | None:
    if provider_options is None:
        return None
    native_provider_options = copy.deepcopy(dict(provider_options))
    native_provider_options.pop("structured_output", None)
    return native_provider_options


def _build_openai_responses_body(
    request: Any,
    *,
    provider_options: Mapping[str, Any] | None = None,
    stream: bool = False,
) -> dict[str, Any]:
    body: dict[str, Any] = {"model": request.model}

    instructions = _extract_instructions(request.messages)
    if instructions is not None:
        body["instructions"] = instructions

    input_items: list[dict[str, Any]] = []
    for message in request.messages:
        if message.role in (Role.SYSTEM, Role.DEVELOPER):
            continue
        input_items.extend(_message_input_items(message))
    if input_items:
        body["input"] = input_items

    if request.tools is not None:
        body["tools"] = [_tool_definition(tool) for tool in request.tools]

    if request.tool_choice is not None:
        body["tool_choice"] = _tool_choice_value(request.tool_choice)

    if request.temperature is not None:
        body["temperature"] = request.temperature
    if request.top_p is not None:
        body["top_p"] = request.top_p
    if request.max_tokens is not None:
        body["max_output_tokens"] = request.max_tokens
    if request.stop_sequences is not None:
        body["stop"] = list(request.stop_sequences)
    if request.metadata is not None:
        body["metadata"] = dict(request.metadata)
    if request.reasoning_effort is not None:
        body["reasoning"] = {"effort": request.reasoning_effort}
    if request.response_format is not None:
        body["response_format"] = _response_format_value(request.response_format)

    if stream:
        body["stream"] = True

    native_provider_options = _native_provider_options(provider_options)
    native_tools: Sequence[Any] | None = None
    if native_provider_options is not None:
        raw_tools = native_provider_options.pop("tools", None)
        if raw_tools is not None:
            if not isinstance(raw_tools, Sequence) or isinstance(
                raw_tools,
                (str, bytes, bytearray),
            ):
                raise InvalidRequestError(
                    "OpenAI provider_options['openai']['tools'] must be a sequence",
                    provider="openai",
                )
            native_tools = raw_tools
    if native_provider_options:
        body = _deep_merge_mappings(body, native_provider_options)

    body["model"] = request.model
    if instructions is not None:
        body["instructions"] = instructions
    if input_items:
        body["input"] = input_items
    if request.tools is not None:
        body["tools"] = [_tool_definition(tool) for tool in request.tools]
    if request.tool_choice is not None:
        body["tool_choice"] = _tool_choice_value(request.tool_choice)
    if request.temperature is not None:
        body["temperature"] = request.temperature
    if request.top_p is not None:
        body["top_p"] = request.top_p
    if request.max_tokens is not None:
        body["max_output_tokens"] = request.max_tokens
    if request.stop_sequences is not None:
        body["stop"] = list(request.stop_sequences)
    if request.metadata is not None:
        body["metadata"] = dict(request.metadata)
    if request.reasoning_effort is not None:
        body["reasoning"] = {"effort": request.reasoning_effort}
    if request.response_format is not None:
        body["response_format"] = _response_format_value(request.response_format)
    if stream:
        body["stream"] = True

    if native_tools is not None:
        merged_tools = list(body.get("tools") or [])
        merged_tools.extend(copy.deepcopy(list(native_tools)))
        body["tools"] = merged_tools
    return body


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
    content_parts: list[ContentPart] = field(default_factory=list)
    active_text: list[str] | None = None
    active_text_id: str | None = None
    active_tool_call: dict[str, Any] | None = None
    active_tool_call_id: str | None = None

    def _merge_final_fragment(self, fragments: list[str], fragment: str) -> None:
        current = "".join(fragments)
        if current and (fragment == current or fragment.startswith(current)):
            fragments[:] = [fragment]
            return
        fragments.append(fragment)

    def _flush_text(self) -> None:
        if self.active_text is None:
            return
        text = "".join(self.active_text)
        self.active_text = None
        self.active_text_id = None
        if text:
            self.content_parts.append(
                ContentPart(kind=ContentKind.TEXT, text=text),
            )

    def _flush_tool_call(self) -> None:
        if self.active_tool_call is None:
            return
        tool_call = ToolCall(**self.active_tool_call)
        self.active_tool_call = None
        self.active_tool_call_id = None
        self.content_parts.append(
            ContentPart(kind=ContentKind.TOOL_CALL, tool_call=tool_call),
        )

    def _merge_final_tool_call(self, tool_call: ToolCall) -> ToolCall:
        if self.active_tool_call is None:
            self.active_tool_call = dict(tool_call.__dict__)
            self.active_tool_call_id = tool_call.id
            return tool_call

        merged = dict(self.active_tool_call)
        final_state = dict(tool_call.__dict__)

        # OpenAI may stream a provisional item id and only expose the final call_id
        # when the function-call item is done. Keep the accumulated arguments when
        # the done payload is just carrying terminal metadata.
        merged["id"] = final_state.get("id") or merged.get("id") or "openai_call"
        merged["name"] = final_state.get("name") or merged.get("name") or "tool"
        merged["type"] = final_state.get("type") or merged.get("type") or "function"

        if _tool_call_has_placeholder_arguments(tool_call) and merged.get("raw_arguments"):
            self.active_tool_call = merged
            self.active_tool_call_id = merged["id"]
            return ToolCall(**merged)

        merged.update(final_state)
        self.active_tool_call = merged
        self.active_tool_call_id = merged["id"]
        return ToolCall(**merged)

    def _finalize_blocks(self) -> None:
        self._flush_text()
        self._flush_tool_call()

    def _start_text(
        self,
        text_id: str | None,
        fragment: str,
        raw: Any,
    ) -> list[StreamEvent]:
        if self.active_text is not None and text_id is not None:
            if self.active_text_id is None:
                self.active_text_id = text_id
            elif text_id != self.active_text_id:
                self._flush_text()
        started = self.active_text is None
        if started:
            self.active_text = []
            self.active_text_id = text_id
        resolved_text_id = self.active_text_id or text_id
        self.active_text.append(fragment)
        events: list[StreamEvent] = []
        if started:
            events.append(
                StreamEvent(
                    type=StreamEventType.TEXT_START,
                    text_id=resolved_text_id,
                )
            )
        events.append(
            StreamEvent(
                type=StreamEventType.TEXT_DELTA,
                delta=fragment,
                text_id=resolved_text_id,
                raw=raw,
            )
        )
        return events

    def _finish_text(
        self,
        text_id: str | None,
        fragment: str | None,
        raw: Any,
    ) -> StreamEvent:
        if self.active_text is not None and text_id is not None:
            if self.active_text_id is None:
                self.active_text_id = text_id
            elif text_id != self.active_text_id:
                self._flush_text()
        if self.active_text is None:
            self.active_text = []
            self.active_text_id = text_id
        if fragment is not None:
            self._merge_final_fragment(self.active_text, fragment)
        resolved_text_id = self.active_text_id
        self._flush_text()
        return StreamEvent(
            type=StreamEventType.TEXT_END,
            delta=fragment,
            text_id=resolved_text_id or text_id,
            raw=raw,
        )

    def _start_tool_call(
        self,
        tool_call_id: str | None,
        name: str | None,
        fragment: str,
        raw: Any,
    ) -> list[StreamEvent]:
        if (
            self.active_tool_call is not None
            and tool_call_id is not None
            and self.active_tool_call_id is not None
            and tool_call_id != self.active_tool_call_id
        ):
            self._flush_tool_call()
        started = self.active_tool_call is None
        if self.active_tool_call is None:
            self.active_tool_call = {
                "id": tool_call_id or "openai_call",
                "name": name or "tool",
                "arguments": "",
                "raw_arguments": "",
                "type": "function",
            }
            self.active_tool_call_id = tool_call_id
        else:
            if tool_call_id is not None and self.active_tool_call_id is None:
                self.active_tool_call_id = tool_call_id
                self.active_tool_call["id"] = tool_call_id
            if name and self.active_tool_call.get("name") in (None, "tool"):
                self.active_tool_call["name"] = name

        current_arguments = self.active_tool_call.get("arguments")
        if isinstance(current_arguments, dict):
            current_arguments = json.dumps(
                current_arguments,
                separators=(",", ":"),
                sort_keys=True,
            )
        if not isinstance(current_arguments, str):
            current_arguments = ""
        current_raw_arguments = self.active_tool_call.get("raw_arguments")
        if not isinstance(current_raw_arguments, str):
            current_raw_arguments = current_arguments
        self.active_tool_call["arguments"] = current_arguments + fragment
        self.active_tool_call["raw_arguments"] = current_raw_arguments + fragment
        resolved_tool_call_id = (
            self.active_tool_call_id
            or tool_call_id
            or self.active_tool_call.get("id")
            or "openai_call"
        )
        resolved_name = self.active_tool_call.get("name") or name or "tool"
        events: list[StreamEvent] = []
        if started:
            events.append(
                StreamEvent(
                    type=StreamEventType.TOOL_CALL_START,
                    tool_call=ToolCall(
                        id=resolved_tool_call_id,
                        name=resolved_name,
                        arguments="",
                        raw_arguments="",
                        type="function",
                    ),
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
                    type="function",
                ),
                raw=raw,
            )
        )
        return events

    def _finish_tool_call(
        self,
        tool_call: ToolCall | None,
        raw: Any,
    ) -> StreamEvent:
        if tool_call is None:
            return StreamEvent(type=StreamEventType.PROVIDER_EVENT, raw=raw)

        merged_tool_call = self._merge_final_tool_call(tool_call)
        self._flush_tool_call()
        return StreamEvent(
            type=StreamEventType.TOOL_CALL_END,
            tool_call=merged_tool_call,
            raw=raw,
        )

    def translate(self, event_type: str, payload: Any) -> list[StreamEvent]:
        if event_type.startswith("response.") and not isinstance(payload, Mapping):
            raise ProviderError(
                "unexpected non-JSON OpenAI stream payload",
                provider=self.provider,
                raw=payload,
                retryable=False,
            )

        if event_type == "response.created":
            response = normalize_openai_response(
                payload,
                provider=self.provider,
                headers=self.headers,
                raw=payload,
            )
            return [
                StreamEvent(
                    type=StreamEventType.STREAM_START,
                    response=response,
                    raw=payload,
                )
            ]

        if event_type == "response.in_progress":
            return [StreamEvent(type=StreamEventType.PROVIDER_EVENT, raw=payload)]

        if event_type == "response.output_text.delta":
            text_id = _coerce_identifier(
                _text_from_payload(payload, "item_id", "output_index", "content_index", "id")
            )
            fragment = _text_from_payload(payload, "delta", "text")
            if fragment is None:
                raise ProviderError(
                    "missing text delta in OpenAI stream payload",
                    provider=self.provider,
                    raw=payload,
                    retryable=False,
                )
            return self._start_text(text_id, fragment, payload)

        if event_type == "response.output_text.done":
            text_id = _coerce_identifier(
                _text_from_payload(payload, "item_id", "output_index", "content_index", "id")
            )
            fragment = _text_from_payload(payload, "text", "delta")
            return [self._finish_text(text_id, fragment, payload)]

        if event_type == "response.function_call_arguments.delta":
            tool_call_id = _coerce_identifier(
                _text_from_payload(payload, "call_id", "item_id", "output_index", "id")
            )
            name = _text_from_payload(payload, "name", "function_name")
            fragment = _text_from_payload(payload, "delta", "arguments", "input")
            if fragment is None:
                raise ProviderError(
                    "missing function-call argument delta in OpenAI stream payload",
                    provider=self.provider,
                    raw=payload,
                    retryable=False,
                )
            return self._start_tool_call(tool_call_id, name, fragment, payload)

        if event_type == "response.output_item.done":
            item = _item_from_payload(payload)
            if not isinstance(item, Mapping):
                raise ProviderError(
                    "unexpected non-object OpenAI output item",
                    provider=self.provider,
                    raw=payload,
                    retryable=False,
                )
            parsed = normalize_openai_response(
                {"output": [item]},
                provider=self.provider,
                headers=self.headers,
                raw=item,
            )
            if parsed.tool_calls:
                events: list[StreamEvent] = []
                for tool_call in parsed.tool_calls:
                    events.append(self._finish_tool_call(tool_call, payload))
                return events
            if parsed.text:
                text_id = _coerce_identifier(
                    _text_from_payload(item, "item_id", "output_index", "content_index", "id")
                )
                return [self._finish_text(text_id, parsed.text, payload)]
            return [StreamEvent(type=StreamEventType.PROVIDER_EVENT, raw=payload)]

        if event_type in {"error", "response.error", "response.failed"}:
            message = "OpenAI stream reported an error"
            if isinstance(payload, Mapping):
                nested_error = payload.get("error")
                if isinstance(nested_error, Mapping):
                    message = _text_from_payload(
                        nested_error,
                        "message",
                        "detail",
                        "description",
                    ) or message
                else:
                    message = (
                        _text_from_payload(payload, "message", "detail", "description")
                        or message
                    )
            elif isinstance(payload, str) and payload.strip():
                message = payload.strip()

            return [
                StreamEvent(
                    type=StreamEventType.ERROR,
                    error=ProviderError(
                        message,
                        provider=self.provider,
                        raw=payload,
                        retryable=False,
                    ),
                    raw=payload,
                )
            ]

        if event_type == "response.completed":
            self._finalize_blocks()
            response = normalize_openai_response(
                payload,
                provider=self.provider,
                headers=self.headers,
                raw=payload,
            )
            if self.content_parts:
                response = replace(
                    response,
                    message=replace(response.message, content=list(self.content_parts)),
                )
                if (
                    any(part.kind == ContentKind.TOOL_CALL for part in self.content_parts)
                    and response.finish_reason.reason
                    not in {
                        FinishReason.LENGTH,
                        FinishReason.CONTENT_FILTER,
                        FinishReason.ERROR,
                        FinishReason.TOOL_CALLS,
                    }
                ):
                    response = replace(
                        response,
                        finish_reason=FinishReason(
                            reason=FinishReason.TOOL_CALLS,
                            raw=response.finish_reason.raw,
                        ),
                    )
            return [
                StreamEvent(
                    type=StreamEventType.FINISH,
                    finish_reason=response.finish_reason,
                    usage=response.usage,
                    response=response,
                    raw=payload,
                )
            ]

        return [StreamEvent(type=StreamEventType.PROVIDER_EVENT, raw=payload)]


class OpenAIAdapter:
    name = "openai"

    def __init__(
        self,
        api_key: str | None = None,
        base_url: str | None = None,
        organization: str | None = None,
        project: str | None = None,
        timeout: float | httpx.Timeout | None = None,
        default_headers: Mapping[str, Any] | None = None,
        client: httpx.AsyncClient | Any | None = None,
        transport: httpx.AsyncBaseTransport | None = None,
        *,
        owns_client: bool = False,
    ) -> None:
        if client is not None and transport is not None:
            raise ValueError("client and transport are mutually exclusive")

        resolved_api_key = api_key if api_key is not None else _env_value("OPENAI_API_KEY")
        resolved_base_url = base_url if base_url is not None else _env_value("OPENAI_BASE_URL")
        resolved_organization = (
            organization if organization is not None else _env_value("OPENAI_ORG_ID")
        )
        resolved_project = project if project is not None else _env_value("OPENAI_PROJECT_ID")

        self.api_key = resolved_api_key
        self.base_url = normalize_openai_base_url(resolved_base_url)
        self.organization = resolved_organization
        self.project = resolved_project
        self.timeout = timeout
        self.default_headers = dict(default_headers or {})
        self.config = {
            "api_key": self.api_key,
            "base_url": self.base_url,
            "organization": self.organization,
            "project": self.project,
        }
        self._responses_url = build_openai_responses_url(self.base_url)
        self._client = client
        self._owns_client = owns_client or client is None
        self._client_closed = False

        if self._client is None:
            client_kwargs: dict[str, Any] = {}
            if transport is not None:
                client_kwargs["transport"] = transport
            if self.timeout is not None:
                client_kwargs["timeout"] = self.timeout
            self._client = httpx.AsyncClient(**client_kwargs)

    def _request_headers(self) -> httpx.Headers:
        if self.api_key is None:
            raise ConfigurationError("OpenAI API key is required")

        headers = httpx.Headers(self.default_headers)
        headers["Authorization"] = f"Bearer {self.api_key}"
        if self.organization is not None:
            headers["OpenAI-Organization"] = self.organization
        if self.project is not None:
            headers["OpenAI-Project"] = self.project
        return headers

    def _request_kwargs(
        self,
        request: Any,
        *,
        stream: bool = False,
    ) -> dict[str, Any]:
        body = _build_openai_responses_body(
            request,
            provider_options=provider_options_for(request, self.name),
            stream=stream,
        )
        kwargs: dict[str, Any] = {
            "headers": self._request_headers(),
            "json": body,
        }
        if self.timeout is not None:
            kwargs["timeout"] = self.timeout
        return kwargs

    async def complete(self, request: Any) -> Response:
        if not hasattr(request, "messages"):
            raise TypeError("request must be a Request")

        client = self._client
        if client is None:
            raise ConfigurationError("OpenAI HTTP client is not available")

        try:
            response = await client.post(self._responses_url, **self._request_kwargs(request))
        except httpx.HTTPError as exc:
            raise _provider_error_from_httpx_error(exc, provider=self.name) from exc

        if response.status_code >= 400:
            raise provider_error_from_response(
                response,
                provider=self.name,
                raw=normalize_raw_payload(response.text),
            )

        payload = normalize_raw_payload(response.text)
        return normalize_openai_response(
            payload,
            provider=self.name,
            headers=response.headers,
            raw=payload,
        )

    def stream(self, request: Any) -> AsyncIterator[StreamEvent]:
        async def _stream() -> AsyncIterator[StreamEvent]:
            if not hasattr(request, "messages"):
                raise TypeError("request must be a Request")

            client = self._client
            if client is None:
                raise ConfigurationError("OpenAI HTTP client is not available")

            try:
                async with client.stream(
                    "POST",
                    self._responses_url,
                    **self._request_kwargs(request, stream=True),
                ) as response:
                    if response.status_code >= 400:
                        await response.aread()
                        raise provider_error_from_response(
                            response,
                            provider=self.name,
                            raw=normalize_raw_payload(response.text),
                        )

                    content_type = response.headers.get("content-type", "")
                    if "text/event-stream" not in content_type.casefold():
                        payload = normalize_raw_payload(await response.aread())
                        normalized = normalize_openai_response(
                            payload,
                            provider=self.name,
                            headers=response.headers,
                            raw=payload,
                        )
                        yield StreamEvent(
                            type=StreamEventType.STREAM_START,
                            response=normalized,
                            raw=payload,
                        )
                        yield StreamEvent(
                            type=StreamEventType.FINISH,
                            finish_reason=normalized.finish_reason,
                            usage=normalized.usage,
                            response=normalized,
                            raw=payload,
                        )
                        return

                    state = _StreamState(provider=self.name, headers=response.headers)
                    async for event in aiter_sse_events(response.aiter_lines()):
                        payload = normalize_raw_payload(event.data)
                        event_type = event.type
                        if isinstance(payload, Mapping):
                            payload_type = payload.get("type")
                            if isinstance(payload_type, str) and payload_type:
                                event_type = payload_type
                        try:
                            translated_events = state.translate(event_type, payload)
                        except ProviderError:
                            raise
                        except Exception:
                            logger.exception("Unexpected failure translating OpenAI stream event")
                            raise

                        for translated_event in translated_events:
                            yield translated_event
                            if translated_event.type in (
                                StreamEventType.FINISH,
                                StreamEventType.ERROR,
                            ):
                                return
            except httpx.HTTPError as exc:
                raise _provider_error_from_httpx_error(exc, provider=self.name) from exc

        return _stream()

    def supports_tool_choice(self, mode: str) -> bool:
        return mode.casefold() in {"auto", "none", "required", "named"}

    async def close(self) -> None:
        if self._client_closed or not self._owns_client:
            return None

        client = self._client
        if client is None:
            self._client_closed = True
            return None

        close = getattr(client, "aclose", None)
        if close is None or not callable(close):
            self._client_closed = True
            return None

        try:
            result = close()
            if inspect.isawaitable(result):
                await result
            self._client_closed = True
        except Exception:
            logger.exception("Unexpected error closing OpenAI HTTP client")


__all__ = ["OpenAIAdapter"]
