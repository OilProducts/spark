from __future__ import annotations

import copy
import json
import logging
from collections.abc import AsyncIterator, Mapping, Sequence
from dataclasses import dataclass, field, replace
from typing import Any
from urllib.parse import urlsplit, urlunsplit

import httpx

from ..errors import (
    InvalidRequestError,
    NetworkError,
    ProviderError,
    RequestTimeoutError,
    UnsupportedToolChoiceError,
)
from ..provider_utils.errors import provider_error_from_response
from ..provider_utils.http import normalize_rate_limit, provider_options_for
from ..provider_utils.media import prepare_anthropic_image_block
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
    ThinkingData,
    ToolResultData,
    Usage,
)

logger = logging.getLogger(__name__)

DEFAULT_ANTHROPIC_BASE_URL = "https://api.anthropic.com"
DEFAULT_ANTHROPIC_VERSION = "2023-06-01"
DEFAULT_ANTHROPIC_CACHE_CONTROL = {"type": "ephemeral"}
PROMPT_CACHING_BETA = "prompt-caching-2024-07-31"
DEFAULT_STRUCTURED_OUTPUT_SYSTEM_INSTRUCTION = (
    "Return only valid JSON that matches the provided schema."
)


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


def _coerce_exact_text(value: Any, *, field_name: str) -> str | None:
    if value is None:
        return None
    if isinstance(value, str):
        return value
    if isinstance(value, (bytes, bytearray)):
        try:
            return bytes(value).decode("utf-8")
        except UnicodeDecodeError:
            logger.debug("Unable to decode %s as UTF-8", field_name, exc_info=True)
            return bytes(value).decode("utf-8", errors="replace")
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
        logger.exception("Unexpected failure serializing Anthropic payload value")
        raise


def normalize_anthropic_base_url(base_url: str | None) -> str:
    text = (base_url or "").strip() or DEFAULT_ANTHROPIC_BASE_URL
    parts = urlsplit(text)
    path = parts.path.rstrip("/")
    if path.endswith("/messages"):
        path = path[: -len("/messages")]
    if not path:
        path = "/v1"
    elif not path.endswith("/v1"):
        path = f"{path}/v1"
    return urlunsplit((parts.scheme, parts.netloc, path, parts.query, parts.fragment))


def build_anthropic_messages_url(base_url: str | None) -> str:
    normalized = normalize_anthropic_base_url(base_url)
    parts = urlsplit(normalized)
    path = parts.path.rstrip("/")
    if not path.endswith("/messages"):
        path = f"{path}/messages"
    return urlunsplit((parts.scheme, parts.netloc, path, parts.query, parts.fragment))


def _tool_definition(tool: Tool) -> dict[str, Any]:
    input_schema: dict[str, Any]
    if tool.parameters is None:
        input_schema = {"type": "object", "properties": {}}
    else:
        input_schema = copy.deepcopy(tool.parameters)

    return {
        "name": tool.name,
        "description": tool.description if tool.description is not None else "",
        "input_schema": input_schema,
    }


def _tool_choice_value(tool_choice: ToolChoice) -> dict[str, Any]:
    if tool_choice.is_none:
        return {"type": "none"}
    if tool_choice.is_named:
        return {"type": "tool", "name": tool_choice.tool_name}
    if tool_choice.is_required:
        return {"type": "any"}
    return {"type": "auto"}


def _requested_tool_choice_value(
    tool_choice: ToolChoice,
    *,
    tool_names: set[str],
) -> dict[str, Any] | None:
    if tool_choice.is_none:
        return None

    if tool_choice.is_named:
        if tool_choice.tool_name not in tool_names:
            logger.warning(
                "Anthropic tool_choice named %r requires a matching tool",
                tool_choice.tool_name,
            )
            raise UnsupportedToolChoiceError(
                f"Anthropic tool_choice named {tool_choice.tool_name!r} requires a matching tool",
            )
        return _tool_choice_value(tool_choice)

    if tool_choice.is_required:
        if not tool_names:
            logger.warning(
                "Anthropic tool_choice required requires at least one tool",
            )
            raise UnsupportedToolChoiceError(
                "Anthropic tool_choice required requires at least one tool",
            )
        return _tool_choice_value(tool_choice)

    if tool_names:
        return _tool_choice_value(tool_choice)

    return None


def _schema_instruction_text(schema: Mapping[str, Any]) -> str:
    schema_json = json.dumps(
        copy.deepcopy(dict(schema)),
        separators=(",", ":"),
        sort_keys=True,
    )
    return f"JSON Schema:\n```json\n{schema_json}\n```"


def _structured_output_schema_from_response_format(
    response_format: Any,
) -> dict[str, Any] | None:
    if response_format is None:
        return None

    response_format_type = _coerce_text(
        getattr(response_format, "type", None),
        field_name="request.response_format.type",
    )
    if response_format_type is None:
        logger.warning("Anthropic response_format type must be a string")
        raise InvalidRequestError(
            "Anthropic response_format type must be a string",
            provider="anthropic",
        )
    if response_format_type.casefold() != "json_schema":
        logger.warning(
            "Anthropic response_format only supports json_schema",
        )
        raise InvalidRequestError(
            "Anthropic response_format only supports json_schema",
            provider="anthropic",
        )

    schema = getattr(response_format, "json_schema", None)
    if schema is None:
        logger.warning("Anthropic json_schema response_format requires a json_schema")
        raise InvalidRequestError(
            "Anthropic json_schema response_format requires a json_schema",
            provider="anthropic",
        )
    if not isinstance(schema, Mapping):
        logger.warning(
            "Anthropic json_schema response_format requires a mapping schema",
        )
        raise InvalidRequestError(
            "Anthropic json_schema response_format requires a mapping schema",
            provider="anthropic",
        )
    return copy.deepcopy(dict(schema))


def _structured_output_strategy(structured_output: Mapping[str, Any]) -> str:
    strategy = _coerce_text(
        structured_output.get("strategy"),
        field_name="provider_options.anthropic.structured_output.strategy",
    )
    if strategy is None:
        return "schema-instruction"

    normalized = strategy.casefold()
    if normalized == "schema-instruction":
        return normalized

    logger.warning(
        "Anthropic structured_output strategy %r is not supported",
        strategy,
    )
    raise InvalidRequestError(
        f"Anthropic structured_output strategy {strategy!r} is not supported",
        provider="anthropic",
    )


def _structured_output_instruction(
    request: Any,
    *,
    provider_options: Mapping[str, Any],
) -> str | None:
    system_instruction = _coerce_text(
        provider_options.get("system_instruction"),
        field_name="provider_options.anthropic.system_instruction",
    )
    response_format_schema = _structured_output_schema_from_response_format(
        getattr(request, "response_format", None),
    )

    structured_output = provider_options.get("structured_output")
    if structured_output is None:
        schema = response_format_schema
    else:
        if not isinstance(structured_output, Mapping):
            logger.warning("Anthropic structured_output must be a mapping")
            raise InvalidRequestError(
                "Anthropic structured_output must be a mapping",
                provider="anthropic",
            )

        _structured_output_strategy(structured_output)

        schema = structured_output.get("schema")
        if schema is not None:
            if not isinstance(schema, Mapping):
                logger.warning(
                    "Anthropic structured_output schema must be a mapping",
                )
                raise InvalidRequestError(
                    "Anthropic structured_output schema must be a mapping",
                    provider="anthropic",
                )
            schema = copy.deepcopy(dict(schema))
        else:
            schema = response_format_schema

        if schema is None:
            logger.warning("Anthropic structured_output requires a schema")
            raise InvalidRequestError(
                "Anthropic structured_output requires a schema",
                provider="anthropic",
            )

    if schema is None:
        return system_instruction

    schema_instruction = _schema_instruction_text(schema)
    if system_instruction is None:
        return (
            f"{DEFAULT_STRUCTURED_OUTPUT_SYSTEM_INSTRUCTION}\n\n"
            f"{schema_instruction}"
        )
    if system_instruction == DEFAULT_STRUCTURED_OUTPUT_SYSTEM_INSTRUCTION:
        return (
            f"{DEFAULT_STRUCTURED_OUTPUT_SYSTEM_INSTRUCTION}\n\n"
            f"{schema_instruction}"
        )
    return f"{system_instruction}\n\n{schema_instruction}"


def _tool_call_input_value(tool_call: Any) -> tuple[Any, str | None]:
    arguments = getattr(tool_call, "arguments", None)
    raw_arguments = getattr(tool_call, "raw_arguments", None)

    if isinstance(arguments, str):
        raw_text = raw_arguments if isinstance(raw_arguments, str) else arguments
        text = raw_text.strip()
        if not text:
            return {}, "{}"
        try:
            parsed = json.loads(text)
        except json.JSONDecodeError as exc:
            raise InvalidRequestError(
                "Anthropic tool_use input must be a JSON object",
                provider="anthropic",
            ) from exc
        if not isinstance(parsed, Mapping):
            raise InvalidRequestError(
                "Anthropic tool_use input must be a JSON object",
                provider="anthropic",
            )
        return copy.deepcopy(dict(parsed)), text

    if isinstance(arguments, Mapping):
        serialized = json.dumps(arguments, separators=(",", ":"), sort_keys=True)
        return copy.deepcopy(dict(arguments)), serialized

    if arguments is None:
        return {}, "{}"

    return arguments, None


def _tool_result_content_value(tool_result: Any) -> str | list[Any]:
    content = getattr(tool_result, "content", None)
    image_data = getattr(tool_result, "image_data", None)
    image_media_type = getattr(tool_result, "image_media_type", None)

    if isinstance(content, list):
        value: str | list[Any] = copy.deepcopy(content)
    elif isinstance(content, (str, bytes, bytearray)):
        text = _coerce_text(content, field_name="tool_result.content") or ""
        value = text
    elif isinstance(content, Mapping):
        value = _serialize_json_value(content)
    elif content is None:
        value = ""
    else:
        value = _serialize_json_value(content)

    if image_data is None:
        return value

    image_block = prepare_anthropic_image_block(
        image_data,
        media_type=image_media_type,
    )
    if isinstance(value, list):
        return [*value, image_block]
    blocks: list[dict[str, Any]] = []
    if value:
        blocks.append({"type": "text", "text": value})
    blocks.append(image_block)
    return blocks


def _system_text_from_message(message: Message) -> str:
    fragments: list[str] = []
    for part in message.content:
        if part.kind != ContentKind.TEXT:
            raise InvalidRequestError(
                "Anthropic system and developer messages must be text-only",
                provider="anthropic",
            )
        if part.text is not None:
            fragments.append(part.text)
    return "\n\n".join(fragment for fragment in fragments if fragment)


def _message_content_block(part: ContentPart, *, role: Role) -> dict[str, Any] | None:
    if part.kind == ContentKind.TEXT:
        if part.text is None:
            return None
        return {"type": "text", "text": part.text}

    if part.kind == ContentKind.IMAGE:
        image = part.image
        if image is None:
            raise InvalidRequestError(
                "Anthropic image content requires an image payload",
                provider="anthropic",
            )
        return prepare_anthropic_image_block(image)

    if part.kind in (ContentKind.THINKING, ContentKind.REDACTED_THINKING):
        thinking = part.thinking
        if thinking is None:
            if part.kind == ContentKind.REDACTED_THINKING and part.text is not None:
                return {"type": "redacted_thinking", "data": part.text}
            if part.text is None:
                return None
            return {"type": "thinking", "thinking": part.text}

        if thinking.redacted:
            return {"type": "redacted_thinking", "data": thinking.text}

        block: dict[str, Any] = {"type": "thinking", "thinking": thinking.text}
        if thinking.signature is not None:
            block["signature"] = thinking.signature
        return block

    if part.kind == ContentKind.TOOL_CALL:
        if role != Role.ASSISTANT:
            raise InvalidRequestError(
                "Anthropic tool_use blocks are only valid in assistant messages",
                provider="anthropic",
            )
        tool_call = part.tool_call
        if tool_call is None:
            raise InvalidRequestError(
                "Anthropic tool_use content requires a tool_call payload",
                provider="anthropic",
            )
        arguments, _ = _tool_call_input_value(tool_call)
        block = {
            "type": "tool_use",
            "id": tool_call.id,
            "name": tool_call.name,
            "input": arguments,
        }
        return block

    if part.kind == ContentKind.TOOL_RESULT:
        if role != Role.USER:
            raise InvalidRequestError(
                "Anthropic tool_result blocks are only valid in user messages",
                provider="anthropic",
            )
        tool_result = part.tool_result
        if tool_result is None:
            raise InvalidRequestError(
                "Anthropic tool_result content requires a tool_result payload",
                provider="anthropic",
            )
        tool_use_id = getattr(tool_result, "tool_call_id", None)
        if tool_use_id is None:
            raise InvalidRequestError(
                "Anthropic tool_result content requires a tool_call_id",
                provider="anthropic",
            )
        block: dict[str, Any] = {
            "type": "tool_result",
            "tool_use_id": tool_use_id,
            "content": _tool_result_content_value(tool_result),
        }
        if getattr(tool_result, "is_error", False):
            block["is_error"] = True
        return block

    raise InvalidRequestError(
        f"Anthropic adapter does not support content kind {part.kind}",
        provider="anthropic",
    )


def _message_payload(message: Message) -> dict[str, Any]:
    if message.role in (Role.SYSTEM, Role.DEVELOPER):
        text = _system_text_from_message(message)
        payload: dict[str, Any] = {"role": message.role.value, "content": text}
        if message.name is not None:
            payload["name"] = message.name
        return payload

    if message.role == Role.TOOL:
        blocks: list[dict[str, Any]] = []
        for part in message.content:
            if part.kind == ContentKind.TEXT:
                if part.text is not None:
                    blocks.append({"type": "text", "text": part.text})
                continue
            if part.kind != ContentKind.TOOL_RESULT:
                raise InvalidRequestError(
                    f"Anthropic adapter does not support content kind {part.kind} in tool messages",
                    provider="anthropic",
                )
            block = _message_content_block(part, role=Role.USER)
            if block is not None:
                blocks.append(block)

        payload = {
            "role": Role.USER.value,
            "content": blocks,
        }
        if message.name is not None:
            payload["name"] = message.name
        return payload

    blocks: list[dict[str, Any]] = []
    for part in message.content:
        block = _message_content_block(part, role=message.role)
        if block is not None:
            blocks.append(block)

    if message.role == Role.USER:
        tool_result_blocks = [block for block in blocks if block["type"] == "tool_result"]
        other_blocks = [block for block in blocks if block["type"] != "tool_result"]
        blocks = [*tool_result_blocks, *other_blocks] if tool_result_blocks else other_blocks

    payload = {"role": message.role.value, "content": blocks}
    if message.name is not None:
        payload["name"] = message.name
    return payload


def _build_message_payloads(messages: Sequence[Message]) -> tuple[list[dict[str, Any]], str | None]:
    merged_messages: list[dict[str, Any]] = []
    system_fragments: list[str] = []

    def _prioritize_tool_results(blocks: list[dict[str, Any]]) -> list[dict[str, Any]]:
        tool_result_blocks = [block for block in blocks if block["type"] == "tool_result"]
        if not tool_result_blocks:
            return blocks
        other_blocks = [block for block in blocks if block["type"] != "tool_result"]
        return [*tool_result_blocks, *other_blocks]

    def _append_message(
        role: str,
        content: str | list[dict[str, Any]],
        *,
        name: str | None = None,
    ) -> None:
        payload: dict[str, Any] = {"role": role, "content": content}
        if name is not None:
            payload["name"] = name

        if merged_messages and merged_messages[-1]["role"] == role:
            existing = merged_messages[-1]["content"]
            if isinstance(existing, str) and isinstance(content, str):
                merged_messages[-1]["content"] = (
                    f"{existing}\n\n{content}"
                    if existing and content
                    else existing or content
                )
                return
            if isinstance(existing, str):
                existing = [{"type": "text", "text": existing}]
                merged_messages[-1]["content"] = existing
            if isinstance(content, str):
                content = [{"type": "text", "text": content}]
            if isinstance(existing, list) and isinstance(content, list):
                existing.extend(copy.deepcopy(content))
                if role == "user":
                    merged_messages[-1]["content"] = _prioritize_tool_results(existing)
                return

        if role == "user" and isinstance(content, list):
            payload["content"] = _prioritize_tool_results(content)
        merged_messages.append(payload)

    for message in messages:
        if message.role in (Role.SYSTEM, Role.DEVELOPER):
            text = _system_text_from_message(message)
            if text:
                system_fragments.append(text)
            continue

        payload = _message_payload(message)
        _append_message(
            payload["role"],
            payload["content"],
            name=payload.get("name"),
        )

    system_text = "\n\n".join(fragment for fragment in system_fragments if fragment)
    return merged_messages, system_text or None


def _copy_provider_option_mapping(
    value: Any,
    *,
    field_name: str,
) -> dict[str, Any] | None:
    if value is None:
        return None
    if not isinstance(value, Mapping):
        logger.warning(
            "Unexpected %s type: %s",
            field_name,
            type(value).__name__,
        )
        raise InvalidRequestError(
            f"Anthropic {field_name.rsplit('.', 1)[-1]} must be a mapping",
            provider="anthropic",
        )

    try:
        return copy.deepcopy(dict(value))
    except Exception:
        logger.exception("Unexpected failure copying Anthropic %s", field_name)
        raise


def _normalize_beta_headers(provider_options: Mapping[str, Any]) -> list[str] | None:
    beta_headers = provider_options.get("beta_headers")
    if beta_headers is None:
        return None

    if isinstance(beta_headers, (str, bytes, bytearray)):
        text = _coerce_text(
            beta_headers,
            field_name="provider_options.anthropic.beta_headers",
        )
        if text is None:
            return None
        values = [fragment for fragment in (part.strip() for part in text.split(",")) if fragment]
        if not values:
            return None
        deduplicated: list[str] = []
        seen: set[str] = set()
        for value in values:
            if value in seen:
                continue
            deduplicated.append(value)
            seen.add(value)
        return deduplicated

    if isinstance(beta_headers, Sequence) and not isinstance(
        beta_headers,
        (str, bytes, bytearray),
    ):
        values: list[str] = []
        for item in beta_headers:
            if item is None:
                continue
            if not isinstance(item, (str, bytes, bytearray)):
                logger.warning(
                    "Unexpected beta_headers item type: %s",
                    type(item).__name__,
                )
                raise InvalidRequestError(
                    "Anthropic beta_headers must contain only strings",
                    provider="anthropic",
                )
            text = _coerce_text(
                item,
                field_name="provider_options.anthropic.beta_headers.item",
            )
            if text:
                values.append(text)
        if not values:
            return None
        deduplicated: list[str] = []
        seen: set[str] = set()
        for value in values:
            if value in seen:
                continue
            deduplicated.append(value)
            seen.add(value)
        return deduplicated

    logger.warning(
        "Unexpected provider_options.anthropic.beta_headers type: %s",
        type(beta_headers).__name__,
    )
    raise InvalidRequestError(
        "Anthropic beta_headers must be a string or sequence of strings",
        provider="anthropic",
    )


def _auto_cache_enabled(provider_options: Mapping[str, Any]) -> bool:
    auto_cache = provider_options.get("auto_cache")
    if auto_cache is None:
        return True
    if isinstance(auto_cache, bool):
        return auto_cache

    logger.warning(
        "Unexpected provider_options.anthropic.auto_cache type: %s",
        type(auto_cache).__name__,
    )
    raise InvalidRequestError(
        "Anthropic auto_cache must be a boolean",
        provider="anthropic",
    )


# Stable-prefix caching marks the reusable tail of each prompt section:
# tools, system, and the last cached conversation turn before the live user tail.
def _system_payload_with_cache_control(
    system_text: str | None,
    *,
    cache_control: Mapping[str, Any] | None,
) -> tuple[str | list[dict[str, Any]] | None, bool]:
    if system_text is None:
        return None, False
    if cache_control is None:
        return system_text, False

    system_block = {
        "type": "text",
        "text": system_text,
        "cache_control": copy.deepcopy(dict(cache_control)),
    }
    return [system_block], True


def _tool_payload_with_cache_control(
    tools: Sequence[Tool] | None,
    *,
    cache_control: Mapping[str, Any] | None,
) -> tuple[list[dict[str, Any]] | None, bool]:
    if not tools:
        return None, False

    payload = [_tool_definition(tool) for tool in tools]
    if cache_control is None:
        return payload, False

    payload[-1]["cache_control"] = copy.deepcopy(dict(cache_control))
    return payload, True


def _conversation_prefix_cache_target_index(
    messages: Sequence[dict[str, Any]],
) -> int | None:
    if not messages:
        return None

    last_index = len(messages) - 1
    last_role = messages[last_index].get("role")
    if isinstance(last_role, str) and last_role.casefold() == Role.USER.value:
        if last_index == 0:
            return None
        return last_index - 1
    return last_index


def _messages_payload_with_cache_control(
    messages: Sequence[dict[str, Any]],
    *,
    cache_control: Mapping[str, Any] | None,
) -> tuple[list[dict[str, Any]], bool]:
    payload = copy.deepcopy(list(messages))
    if cache_control is None:
        return payload, False

    target_index = _conversation_prefix_cache_target_index(payload)
    if target_index is None:
        return payload, False

    target_message = payload[target_index]
    content = target_message.get("content")
    cache_control_value = copy.deepcopy(dict(cache_control))

    if isinstance(content, list) and content:
        content[-1]["cache_control"] = cache_control_value
        return payload, True

    if isinstance(content, str) and content:
        target_message["content"] = [
            {
                "type": "text",
                "text": content,
                "cache_control": cache_control_value,
            }
        ]
        return payload, True

    return payload, False


def build_anthropic_messages_request(
    request: Any,
    *,
    provider_options: Mapping[str, Any] | None = None,
    stream: bool = False,
) -> tuple[dict[str, Any], dict[str, str]]:
    if not hasattr(request, "messages"):
        raise TypeError("request must be a Request")

    options = dict(
        provider_options
        if provider_options is not None
        else provider_options_for(request, "anthropic")
    )
    messages, system_text = _build_message_payloads(request.messages)

    structured_output_instruction = _structured_output_instruction(
        request,
        provider_options=options,
    )
    if structured_output_instruction is not None:
        system_text = (
            f"{system_text}\n\n{structured_output_instruction}"
            if system_text
            else structured_output_instruction
        )

    body: dict[str, Any] = {
        "model": request.model,
        "messages": messages,
        "max_tokens": request.max_tokens if request.max_tokens is not None else 4096,
    }
    if request.temperature is not None:
        body["temperature"] = request.temperature
    if request.top_p is not None:
        body["top_p"] = request.top_p
    if request.stop_sequences:
        body["stop_sequences"] = list(request.stop_sequences)
    if request.metadata:
        body["metadata"] = copy.deepcopy(request.metadata)
    if stream:
        body["stream"] = True

    thinking = _copy_provider_option_mapping(
        options.get("thinking"),
        field_name="provider_options.anthropic.thinking",
    )
    if thinking is not None:
        body["thinking"] = thinking

    output_config = _copy_provider_option_mapping(
        options.get("output_config"),
        field_name="provider_options.anthropic.output_config",
    )
    if output_config is not None:
        body["output_config"] = output_config

    explicit_cache_control = _copy_provider_option_mapping(
        options.get("cache_control"),
        field_name="provider_options.anthropic.cache_control",
    )
    auto_cache_enabled = _auto_cache_enabled(options)
    cache_control: dict[str, Any] | None
    if explicit_cache_control is not None:
        cache_control = explicit_cache_control
    elif auto_cache_enabled:
        cache_control = copy.deepcopy(DEFAULT_ANTHROPIC_CACHE_CONTROL)
    else:
        cache_control = None

    cache_annotations_present = False

    system_payload, system_cached = _system_payload_with_cache_control(
        system_text,
        cache_control=cache_control,
    )
    cache_annotations_present = cache_annotations_present or system_cached
    if system_payload is not None:
        body["system"] = system_payload

    tool_names = {tool.name for tool in request.tools or ()}
    tool_choice_payload: dict[str, Any] | None = None
    tools_payload: list[dict[str, Any]] | None = None

    if request.tool_choice is not None:
        tool_choice_payload = _requested_tool_choice_value(
            request.tool_choice,
            tool_names=tool_names,
        )

    if request.tools and not (
        request.tool_choice is not None and request.tool_choice.is_none
    ):
        tools_payload, tools_cached = _tool_payload_with_cache_control(
            request.tools,
            cache_control=cache_control,
        )
        cache_annotations_present = cache_annotations_present or tools_cached
        if tools_payload is not None:
            body["tools"] = tools_payload

    if tool_choice_payload is not None:
        body["tool_choice"] = tool_choice_payload

    messages, messages_cached = _messages_payload_with_cache_control(
        messages,
        cache_control=cache_control,
    )
    cache_annotations_present = cache_annotations_present or messages_cached

    headers: dict[str, str] = {"anthropic-version": DEFAULT_ANTHROPIC_VERSION}
    beta_header_values = _normalize_beta_headers(options)
    if cache_annotations_present:
        if beta_header_values is None:
            beta_header_values = [PROMPT_CACHING_BETA]
        elif PROMPT_CACHING_BETA not in beta_header_values:
            beta_header_values = [*beta_header_values, PROMPT_CACHING_BETA]
    if beta_header_values is not None:
        headers["anthropic-beta"] = ",".join(beta_header_values)

    body["messages"] = messages

    return body, headers


def _response_payload_source(payload: Any) -> Any:
    source = normalize_raw_payload(payload)
    if isinstance(source, Mapping):
        nested = source.get("message")
        if isinstance(nested, Mapping):
            merged = dict(nested)
            for key in (
                "id",
                "model",
                "content",
                "stop_reason",
                "stop_sequence",
                "usage",
                "warnings",
                "role",
            ):
                if key in source and key not in merged:
                    merged[key] = source[key]
            return merged
    return source


def _content_parts_from_block(item: Any) -> list[ContentPart]:
    if isinstance(item, ContentPart):
        return [item]

    if not isinstance(item, Mapping):
        text = _coerce_text(item, field_name="response.content")
        if text is None:
            return []
        return [ContentPart(kind=ContentKind.TEXT, text=text)]

    item_type = _coerce_text(item.get("type"), field_name="response.content.type")
    if item_type is None:
        text = _coerce_text(item.get("text"), field_name="response.content.text")
        if text is not None:
            return [ContentPart(kind=ContentKind.TEXT, text=text)]
        return []

    kind = item_type.casefold()
    if kind == "text":
        text = _coerce_text(item.get("text"), field_name="response.text")
        if text is None:
            return []
        return [ContentPart(kind=ContentKind.TEXT, text=text)]

    if kind == "thinking":
        thinking_text = _coerce_exact_text(
            item.get("thinking"),
            field_name="response.thinking",
        )
        if thinking_text is None:
            return []
        signature = _coerce_exact_text(
            item.get("signature"),
            field_name="response.signature",
        )
        return [
            ContentPart(
                kind=ContentKind.THINKING,
                thinking=ThinkingData(text=thinking_text, signature=signature),
            )
        ]

    if kind == "redacted_thinking":
        data = _coerce_exact_text(
            item.get("data"),
            field_name="response.redacted_thinking.data",
        )
        if data is None:
            return []
        return [
            ContentPart(
                kind=ContentKind.REDACTED_THINKING,
                thinking=ThinkingData(text=data, redacted=True),
            )
        ]

    if kind == "tool_use":
        tool_call_id = _coerce_text(item.get("id"), field_name="response.tool_use.id")
        name = _coerce_text(item.get("name"), field_name="response.tool_use.name")
        tool_input = item.get("input")
        if tool_call_id is None or name is None:
            return []
        if isinstance(tool_input, str):
            raw_arguments = tool_input
            try:
                arguments: Any = json.loads(tool_input)
            except json.JSONDecodeError:
                arguments = tool_input
        elif isinstance(tool_input, Mapping):
            arguments = copy.deepcopy(dict(tool_input))
            raw_arguments = json.dumps(arguments, separators=(",", ":"), sort_keys=True)
        else:
            arguments = tool_input
            raw_arguments = None
        return [
            ContentPart(
                kind=ContentKind.TOOL_CALL,
                tool_call=ToolCall(
                    id=tool_call_id,
                    name=name,
                    arguments=arguments,
                    raw_arguments=raw_arguments,
                ),
            )
        ]

    if kind == "tool_result":
        tool_use_id = _coerce_text(
            item.get("tool_use_id") or item.get("toolUseId"),
            field_name="response.tool_result.tool_use_id",
        )
        if tool_use_id is None:
            return []
        return [
            ContentPart(
                kind=ContentKind.TOOL_RESULT,
                tool_result=ToolResultData(
                    tool_call_id=tool_use_id,
                    content=item.get("content") if item.get("content") is not None else "",
                    is_error=bool(item.get("is_error", False)),
                ),
            )
        ]

    return []


def _content_parts_from_value(content: Any) -> list[ContentPart]:
    if content is None:
        return []
    if isinstance(content, ContentPart):
        return [content]
    if isinstance(content, (str, bytes, bytearray)):
        text = _coerce_text(content, field_name="response.content")
        if text is None:
            return []
        return [ContentPart(kind=ContentKind.TEXT, text=text)]
    if isinstance(content, Mapping):
        return _content_parts_from_block(content)
    if not isinstance(content, Sequence):
        text = _coerce_text(content, field_name="response.content")
        if text is None:
            return []
        return [ContentPart(kind=ContentKind.TEXT, text=text)]

    parts: list[ContentPart] = []
    for item in content:
        parts.extend(_content_parts_from_block(item))
    return parts


def _response_has_tool_call_parts(parts: Sequence[ContentPart]) -> bool:
    return any(part.kind == ContentKind.TOOL_CALL for part in parts)


def _response_message_from_parts(parts: Sequence[ContentPart]) -> Message:
    if parts and all(part.kind == ContentKind.TOOL_RESULT for part in parts):
        tool_call_ids = {
            part.tool_result.tool_call_id
            for part in parts
            if part.tool_result is not None
        }
        return Message(
            role=Role.TOOL,
            content=list(parts),
            tool_call_id=tool_call_ids.pop() if len(tool_call_ids) == 1 else None,
        )

    assistant_parts = [part for part in parts if part.kind != ContentKind.TOOL_RESULT]
    return Message(role=Role.ASSISTANT, content=assistant_parts)


def _estimate_reasoning_tokens(parts: Sequence[ContentPart]) -> int | None:
    estimated_total = 0
    saw_reasoning = False

    for part in parts:
        if part.kind not in (ContentKind.THINKING, ContentKind.REDACTED_THINKING):
            continue

        saw_reasoning = True
        reasoning = part.thinking.text if part.thinking is not None else part.text
        if not reasoning:
            continue

        estimated_total += max(1, (len(reasoning) + 3) // 4)

    if not saw_reasoning:
        return None
    return estimated_total


def normalize_anthropic_response(
    payload: Any,
    *,
    provider: str = "anthropic",
    headers: Mapping[str, Any] | Any | None = None,
    raw: Any = None,
) -> Response:
    raw_payload = raw if raw is not None else payload
    try:
        source = _response_payload_source(payload)

        if isinstance(source, str):
            text = source.strip()
            parts = [ContentPart(kind=ContentKind.TEXT, text=text)] if text else []
            return Response(
                provider=provider,
                message=Message(role=Role.ASSISTANT, content=parts),
                finish_reason=FinishReason(reason=FinishReason.OTHER),
                usage=normalize_usage(None, provider=provider, raw=None),
                raw=raw_payload,
                warnings=[],
                rate_limit=normalize_rate_limit(headers),
            )

        if isinstance(source, Mapping):
            content_parts = _content_parts_from_value(source.get("content"))
            finish_reason = normalize_finish_reason(source.get("stop_reason"), provider=provider)
            if (
                _response_has_tool_call_parts(content_parts)
                and finish_reason.reason == FinishReason.OTHER
            ):
                finish_reason = FinishReason(
                    reason=FinishReason.TOOL_CALLS,
                    raw=finish_reason.raw,
                )

            usage_payload = source.get("usage")
            warnings = normalize_warnings(source.get("warnings"))
            response_id = _coerce_text(source.get("id"), field_name="response.id")
            model = _coerce_text(source.get("model"), field_name="response.model")
            usage = normalize_usage(usage_payload, provider=provider, raw=usage_payload)
            estimated_reasoning_tokens = _estimate_reasoning_tokens(content_parts)
            if estimated_reasoning_tokens is not None and usage.reasoning_tokens is None:
                usage.reasoning_tokens = estimated_reasoning_tokens

            return Response(
                id=response_id or "",
                model=model or "",
                provider=provider,
                message=_response_message_from_parts(content_parts),
                finish_reason=finish_reason,
                usage=usage,
                raw=raw_payload,
                warnings=warnings,
                rate_limit=normalize_rate_limit(headers),
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
                role=Role.ASSISTANT,
                content=[ContentPart(kind=ContentKind.TEXT, text=text)],
            ),
            finish_reason=FinishReason(reason=FinishReason.OTHER),
            usage=normalize_usage(None, provider=provider, raw=None),
            raw=raw_payload,
            warnings=[],
            rate_limit=normalize_rate_limit(headers),
        )
    except ProviderError:
        raise
    except Exception as exc:
        logger.exception("Unexpected failure normalizing Anthropic response")
        raise ProviderError(
            "failed to normalize Anthropic response",
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


def _stream_parse_error(
    *,
    provider: str,
    event_type: str,
    raw_payload: Any,
) -> StreamEvent:
    logger.error(
        "Anthropic stream event %s payload is not a JSON object",
        event_type,
    )
    return StreamEvent(
        type=StreamEventType.ERROR,
        error=ProviderError(
            "failed to normalize Anthropic stream event",
            provider=provider,
            raw=raw_payload,
            retryable=False,
        ),
        raw=raw_payload,
    )


@dataclass(slots=True)
class _StreamState:
    provider: str
    headers: Mapping[str, Any] | None
    response_template: Response | None = None
    finish_reason: FinishReason | None = None
    usage: Usage | None = None
    active_blocks: dict[int, dict[str, Any]] = field(default_factory=dict)
    completed_blocks: dict[int, ContentPart] = field(default_factory=dict)
    raw_payloads: list[Any] = field(default_factory=list)

    def _block_state(self, index: int, block_type: str | None = None) -> dict[str, Any]:
        state = self.active_blocks.setdefault(index, {})
        if block_type is not None:
            state.setdefault("type", block_type)
        return state

    def _merge_usage(self, usage_payload: Any) -> None:
        if usage_payload is None:
            return

        usage = normalize_usage(usage_payload, provider=self.provider, raw=usage_payload)
        if self.usage is None:
            self.usage = usage
            return

        existing = self.usage
        input_tokens = max(existing.input_tokens, usage.input_tokens)
        output_tokens = max(existing.output_tokens, usage.output_tokens)
        total_tokens = max(
            existing.total_tokens,
            usage.total_tokens,
            input_tokens + output_tokens,
        )

        reasoning_tokens: int | None
        if existing.reasoning_tokens is None and usage.reasoning_tokens is None:
            reasoning_tokens = None
        else:
            reasoning_tokens = max(
                existing.reasoning_tokens or 0,
                usage.reasoning_tokens or 0,
            )

        cache_read_tokens: int | None
        if existing.cache_read_tokens is None and usage.cache_read_tokens is None:
            cache_read_tokens = None
        else:
            cache_read_tokens = max(
                existing.cache_read_tokens or 0,
                usage.cache_read_tokens or 0,
            )

        cache_write_tokens: int | None
        if existing.cache_write_tokens is None and usage.cache_write_tokens is None:
            cache_write_tokens = None
        else:
            cache_write_tokens = max(
                existing.cache_write_tokens or 0,
                usage.cache_write_tokens or 0,
            )

        self.usage = Usage(
            input_tokens=input_tokens,
            output_tokens=output_tokens,
            total_tokens=total_tokens,
            reasoning_tokens=reasoning_tokens,
            cache_read_tokens=cache_read_tokens,
            cache_write_tokens=cache_write_tokens,
            raw=usage.raw if usage.raw is not None else existing.raw,
        )

    def _finalize_block_state(
        self,
        index: int,
        block_state: dict[str, Any],
    ) -> ContentPart | None:
        block_type = _coerce_text(block_state.get("type"), field_name="stream.block.type")
        if block_type is None:
            return None

        block_kind = block_type.casefold()
        if block_kind == "text":
            text_parts = block_state.get("text_parts") or []
            text = "".join(text_parts) if isinstance(text_parts, list) else ""
            if not text:
                return None
            return ContentPart(kind=ContentKind.TEXT, text=text)

        if block_kind in {"thinking", "redacted_thinking"}:
            text_parts = block_state.get("thinking_parts") or []
            text = "".join(text_parts) if isinstance(text_parts, list) else ""
            if not text:
                return None

            signature = _coerce_exact_text(
                block_state.get("signature"),
                field_name="stream.signature",
            )
            redacted = bool(block_state.get("redacted")) or block_kind == "redacted_thinking"
            return ContentPart(
                kind=ContentKind.REDACTED_THINKING if redacted else ContentKind.THINKING,
                thinking=ThinkingData(text=text, signature=signature, redacted=redacted),
            )

        if block_kind == "tool_use":
            chunks = block_state.get("input_chunks") or []
            raw_arguments = "".join(chunks) if isinstance(chunks, list) else None
            tool_input: Any = block_state.get("input")
            arguments: Any = tool_input

            if isinstance(tool_input, str):
                raw_arguments = (
                    f"{tool_input}{raw_arguments or ''}"
                    if raw_arguments
                    else tool_input
                )
                try:
                    parsed = json.loads(raw_arguments)
                except json.JSONDecodeError:
                    arguments = raw_arguments
                else:
                    if isinstance(parsed, Mapping):
                        arguments = copy.deepcopy(dict(parsed))
                    else:
                        arguments = parsed
            elif isinstance(tool_input, Mapping):
                if raw_arguments:
                    try:
                        parsed = json.loads(raw_arguments)
                    except json.JSONDecodeError:
                        arguments = copy.deepcopy(dict(tool_input))
                    else:
                        if isinstance(parsed, Mapping):
                            arguments = copy.deepcopy(dict(parsed))
                        else:
                            arguments = parsed
                else:
                    arguments = copy.deepcopy(dict(tool_input))
                    raw_arguments = (
                        json.dumps(arguments, separators=(",", ":"), sort_keys=True)
                        if arguments
                        else None
                    )

            tool_call_id = _coerce_text(
                block_state.get("id"),
                field_name="stream.tool_use.id",
            )
            name = _coerce_text(
                block_state.get("name"),
                field_name="stream.tool_use.name",
            )
            if tool_call_id is None or name is None:
                return None

            return ContentPart(
                kind=ContentKind.TOOL_CALL,
                tool_call=ToolCall(
                    id=tool_call_id,
                    name=name,
                    arguments=arguments if arguments is not None else {},
                    raw_arguments=raw_arguments,
                ),
            )

        return None

    def _flush_active_blocks(self) -> None:
        if not self.active_blocks:
            return

        for index in sorted(self.active_blocks):
            block = self._finalize_block_state(index, self.active_blocks[index])
            if block is not None:
                self.completed_blocks[index] = block
        self.active_blocks.clear()

    def _resolved_raw(self) -> Any:
        if not self.raw_payloads:
            return None
        if len(self.raw_payloads) == 1:
            return self.raw_payloads[0]
        return list(self.raw_payloads)

    def _build_response(self) -> Response:
        response = self.response_template
        if response is None:
            response = Response(provider=self.provider)

        parts = list(response.message.content)
        for index in sorted(self.completed_blocks):
            parts.append(self.completed_blocks[index])

        message = replace(
            response.message,
            role=Role.ASSISTANT,
            content=parts,
        )
        usage = self.usage if self.usage is not None else response.usage
        if usage.reasoning_tokens is None:
            estimated_reasoning_tokens = _estimate_reasoning_tokens(parts)
            if estimated_reasoning_tokens is not None:
                usage = replace(usage, reasoning_tokens=estimated_reasoning_tokens)

        return replace(
            response,
            message=message,
            finish_reason=self.finish_reason or response.finish_reason,
            usage=usage,
            raw=self._resolved_raw(),
            rate_limit=normalize_rate_limit(self.headers),
        )

    def translate(self, event_type: str, payload: Any) -> list[StreamEvent]:
        self.raw_payloads.append(payload)
        if not isinstance(payload, Mapping):
            return [
                _stream_parse_error(
                    provider=self.provider,
                    event_type=event_type,
                    raw_payload=payload,
                )
            ]

        if event_type == "message_start":
            response = normalize_anthropic_response(
                payload,
                provider=self.provider,
                headers=self.headers,
                raw=payload,
            )
            self.response_template = response
            if self.usage is None:
                self.usage = response.usage
            return [
                StreamEvent(
                    type=StreamEventType.STREAM_START,
                    response=response,
                    raw=payload,
                )
            ]

        if event_type == "message_delta":
            if isinstance(payload, Mapping):
                delta = payload.get("delta")
                if isinstance(delta, Mapping):
                    stop_reason = delta.get("stop_reason")
                    if stop_reason is not None:
                        self.finish_reason = normalize_finish_reason(
                            stop_reason,
                            provider=self.provider,
                        )
                self._merge_usage(payload.get("usage"))
            return []

        if event_type == "message_stop":
            self._flush_active_blocks()
            response = self._build_response()
            return [
                StreamEvent(
                    type=StreamEventType.FINISH,
                    finish_reason=response.finish_reason,
                    usage=response.usage,
                    response=response,
                    raw=payload,
                )
            ]

        if event_type == "content_block_start":
            index = payload.get("index")
            content_block = payload.get("content_block")
            if not isinstance(index, int) or not isinstance(content_block, Mapping):
                return [StreamEvent(type=StreamEventType.PROVIDER_EVENT, raw=payload)]

            block_type = _coerce_text(
                content_block.get("type"),
                field_name="stream.content_block.type",
            )
            if block_type is None:
                return [StreamEvent(type=StreamEventType.PROVIDER_EVENT, raw=payload)]

            state = self._block_state(index, block_type.casefold())
            if block_type.casefold() == "text":
                text = _coerce_exact_text(
                    content_block.get("text"),
                    field_name="stream.text",
                )
                if text is not None:
                    state.setdefault("text_parts", []).append(text)
                return [
                    StreamEvent(
                        type=StreamEventType.TEXT_START,
                        delta=text,
                        raw=payload,
                    )
                ]

            if block_type.casefold() in {"thinking", "redacted_thinking"}:
                if block_type.casefold() == "thinking":
                    text = _coerce_exact_text(
                        content_block.get("thinking"),
                        field_name="stream.thinking",
                    )
                else:
                    text = _coerce_exact_text(
                        content_block.get("data"),
                        field_name="stream.redacted_thinking",
                    )
                if text is not None:
                    state.setdefault("thinking_parts", []).append(text)
                if block_type.casefold() == "redacted_thinking":
                    state["redacted"] = True
                signature = _coerce_exact_text(
                    content_block.get("signature"),
                    field_name="stream.signature",
                )
                if signature is not None:
                    state["signature"] = signature
                return [
                    StreamEvent(
                        type=StreamEventType.REASONING_START,
                        reasoning_delta=text,
                        raw=payload,
                    )
                ]

            if block_type.casefold() == "tool_use":
                tool_call_id = _coerce_text(
                    content_block.get("id"),
                    field_name="stream.tool_use.id",
                )
                name = _coerce_text(
                    content_block.get("name"),
                    field_name="stream.tool_use.name",
                )
                tool_input = content_block.get("input")
                state["id"] = tool_call_id
                state["name"] = name
                state["input"] = (
                    copy.deepcopy(tool_input)
                    if isinstance(tool_input, Mapping)
                    else tool_input
                )
                tool_call_arguments = tool_input if isinstance(tool_input, str) else ""
                return [
                    StreamEvent(
                        type=StreamEventType.TOOL_CALL_START,
                        tool_call=ToolCall(
                            id=tool_call_id or "",
                            name=name or "tool",
                            arguments=tool_call_arguments,
                            raw_arguments=tool_call_arguments,
                        ),
                        raw=payload,
                    )
                ]

            return [StreamEvent(type=StreamEventType.PROVIDER_EVENT, raw=payload)]

        if event_type == "content_block_delta":
            index = payload.get("index")
            delta = payload.get("delta")
            if not isinstance(index, int) or not isinstance(delta, Mapping):
                return [StreamEvent(type=StreamEventType.PROVIDER_EVENT, raw=payload)]

            block_state = self._block_state(index)
            delta_type = _coerce_text(delta.get("type"), field_name="stream.delta.type")
            if delta_type is None:
                return [StreamEvent(type=StreamEventType.PROVIDER_EVENT, raw=payload)]

            delta_kind = delta_type.casefold()
            if delta_kind == "text_delta":
                text = _coerce_exact_text(
                    delta.get("text"),
                    field_name="stream.text_delta",
                )
                if text is not None:
                    block_state.setdefault("text_parts", []).append(text)
                return [
                    StreamEvent(
                        type=StreamEventType.TEXT_DELTA,
                        delta=text,
                        raw=payload,
                    )
                ]

            if delta_kind == "thinking_delta":
                text = _coerce_exact_text(
                    delta.get("thinking"),
                    field_name="stream.thinking_delta",
                )
                if text is not None:
                    block_state.setdefault("thinking_parts", []).append(text)
                return [
                    StreamEvent(
                        type=StreamEventType.REASONING_DELTA,
                        reasoning_delta=text,
                        raw=payload,
                    )
                ]

            if delta_kind == "signature_delta":
                block_state["signature"] = _coerce_exact_text(
                    delta.get("signature"),
                    field_name="stream.signature_delta",
                )
                return []

            if delta_kind == "input_json_delta":
                fragment = _coerce_exact_text(
                    delta.get("partial_json"),
                    field_name="stream.input_json_delta",
                )
                if fragment is not None:
                    block_state.setdefault("input_chunks", []).append(fragment)
                tool_call_id = _coerce_text(block_state.get("id"), field_name="stream.tool_use.id")
                name = _coerce_text(block_state.get("name"), field_name="stream.tool_use.name")
                return [
                    StreamEvent(
                        type=StreamEventType.TOOL_CALL_DELTA,
                        tool_call=ToolCall(
                            id=tool_call_id or "",
                            name=name or "tool",
                            arguments=fragment or "",
                            raw_arguments=fragment or "",
                        ),
                        raw=payload,
                    )
                ]

            return [StreamEvent(type=StreamEventType.PROVIDER_EVENT, raw=payload)]

        if event_type == "content_block_stop":
            index = payload.get("index")
            if not isinstance(index, int):
                return [StreamEvent(type=StreamEventType.PROVIDER_EVENT, raw=payload)]

            block_state = self.active_blocks.pop(index, {})
            block = self._finalize_block_state(index, block_state)
            if block is not None:
                self.completed_blocks[index] = block
            block_type = _coerce_text(block_state.get("type"), field_name="stream.block.type")
            if block_type == "text":
                return [
                    StreamEvent(
                        type=StreamEventType.TEXT_END,
                        raw=payload,
                    )
                ]

            if block_type in {"thinking", "redacted_thinking"}:
                return [
                    StreamEvent(
                        type=StreamEventType.REASONING_END,
                        raw=payload,
                    )
                ]

            if block_type == "tool_use":
                return [
                    StreamEvent(
                        type=StreamEventType.TOOL_CALL_END,
                        tool_call=block.tool_call if block is not None else ToolCall(
                            id="",
                            name="tool",
                            arguments={},
                            raw_arguments=None,
                        ),
                        raw=payload,
                    )
                ]

            return [StreamEvent(type=StreamEventType.PROVIDER_EVENT, raw=payload)]

        if event_type == "error":
            error_payload = payload.get("error")
            message = None
            if isinstance(error_payload, Mapping):
                message = _coerce_text(
                    error_payload.get("message")
                    or error_payload.get("detail")
                    or error_payload.get("description"),
                    field_name="stream.error.message",
                )
            if message is None:
                message = _coerce_text(payload.get("message"), field_name="stream.error")
            error = ProviderError(
                message or f"{self.provider} stream error",
                provider=self.provider,
                raw=payload,
                retryable=False,
            )
            return [
                StreamEvent(
                    type=StreamEventType.ERROR,
                    error=error,
                    raw=payload,
                )
            ]

        return [StreamEvent(type=StreamEventType.PROVIDER_EVENT, raw=payload)]


async def normalize_anthropic_stream_events(
    response: httpx.Response,
    *,
    provider: str = "anthropic",
) -> AsyncIterator[StreamEvent]:
    from ..provider_utils.sse import aiter_sse_events

    state = _StreamState(provider=provider, headers=response.headers)
    async for event in aiter_sse_events(response.aiter_lines()):
        raw_payload = event.data
        try:
            payload = normalize_raw_payload(raw_payload)
            translated_events = state.translate(event.type, payload)
        except Exception as exc:
            logger.exception("Unexpected failure normalizing Anthropic stream event")
            yield StreamEvent(
                type=StreamEventType.ERROR,
                error=ProviderError(
                    "failed to normalize Anthropic stream event",
                    provider=provider,
                    raw=raw_payload,
                    cause=exc,
                    retryable=False,
                ),
                raw=raw_payload,
            )
            return
        for translated_event in translated_events:
            yield translated_event
            if translated_event.type in (StreamEventType.FINISH, StreamEventType.ERROR):
                return


__all__ = [
    "build_anthropic_messages_request",
    "build_anthropic_messages_url",
    "normalize_anthropic_base_url",
    "normalize_anthropic_response",
    "normalize_anthropic_stream_events",
]
