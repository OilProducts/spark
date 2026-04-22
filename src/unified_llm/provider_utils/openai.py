from __future__ import annotations

import json
import logging
from collections.abc import Mapping, Sequence
from typing import Any
from urllib.parse import urlsplit, urlunsplit

from ..errors import ProviderError
from ..tools import ToolCall
from ..types import (
    ContentKind,
    ContentPart,
    FinishReason,
    Message,
    Response,
    Role,
    ThinkingData,
    ToolResultData,
)
from .http import normalize_rate_limit
from .normalization import (
    normalize_finish_reason,
    normalize_raw_payload,
    normalize_usage,
    normalize_warnings,
)

logger = logging.getLogger(__name__)

_STRONGER_TERMINAL_FINISH_REASONS = {
    FinishReason.LENGTH.value,
    FinishReason.CONTENT_FILTER.value,
    FinishReason.ERROR.value,
    FinishReason.TOOL_CALLS.value,
}


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


def normalize_openai_base_url(base_url: str | None) -> str:
    text = (base_url or "").strip() or "https://api.openai.com"
    parts = urlsplit(text)
    path = parts.path.rstrip("/")
    if not path:
        path = "/v1"
    elif not path.endswith("/v1") and not path.endswith("/responses"):
        path = f"{path}/v1"
    return urlunsplit((parts.scheme, parts.netloc, path, parts.query, parts.fragment))


def build_openai_responses_url(base_url: str | None) -> str:
    normalized = normalize_openai_base_url(base_url)
    parts = urlsplit(normalized)
    path = parts.path.rstrip("/")
    if not path.endswith("/responses"):
        path = f"{path}/responses"
    return urlunsplit((parts.scheme, parts.netloc, path, parts.query, parts.fragment))


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


def _response_payload_source(payload: Any) -> Any:
    source = normalize_raw_payload(payload)
    if isinstance(source, Sequence) and not isinstance(source, (str, bytes, bytearray)):
        return {"output": list(source)}
    if isinstance(source, Mapping):
        nested = source.get("response")
        if isinstance(nested, Mapping):
            merged = dict(nested)
            for key in (
                "id",
                "model",
                "output",
                "output_text",
                "status",
                "finish_reason",
                "usage",
                "warnings",
                "incomplete_details",
            ):
                if key in source and key not in merged:
                    merged[key] = source[key]
            return merged
    return source


def _text_from_content_value(value: Any) -> str | None:
    if value is None:
        return None
    if isinstance(value, str):
        text = value.strip()
        return text or None
    if isinstance(value, Mapping):
        for key in ("text", "summary", "output", "content", "delta", "message", "refusal"):
            fragment = value.get(key)
            if isinstance(fragment, Sequence) and not isinstance(
                fragment,
                (str, bytes, bytearray),
            ):
                pieces = []
                for item in fragment:
                    piece = _text_from_content_value(item)
                    if piece:
                        pieces.append(piece)
                if pieces:
                    return "".join(pieces)
            else:
                text = _coerce_text(fragment, field_name=f"response.{key}")
                if text is not None:
                    return text
    if isinstance(value, Sequence) and not isinstance(value, (str, bytes, bytearray)):
        pieces: list[str] = []
        for item in value:
            text = _text_from_content_value(item)
            if text:
                pieces.append(text)
        if pieces:
            return "".join(pieces)
    return None


def _content_parts_from_content_value(content: Any) -> list[ContentPart]:
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
        return []

    parts: list[ContentPart] = []
    for item in content:
        parts.extend(_content_parts_from_output_item(item))
    return parts


def _content_parts_from_output_item(item: Any) -> list[ContentPart]:
    if isinstance(item, ContentPart):
        return [item]
    if not isinstance(item, Mapping):
        text = _coerce_text(item, field_name="response.output_item")
        if text is None:
            return []
        return [ContentPart(kind=ContentKind.TEXT, text=text)]

    item_type = _coerce_text(item.get("type"), field_name="response.output_item.type")
    if item_type is None:
        if "content" in item:
            return _content_parts_from_content_value(item.get("content"))
        if "text" in item:
            text = _coerce_text(item.get("text"), field_name="response.output_item.text")
            if text is not None:
                return [ContentPart(kind=ContentKind.TEXT, text=text)]
        return []

    item_type = item_type.casefold()
    if item_type in {"message", "assistant_message"}:
        parts = _content_parts_from_content_value(item.get("content"))
        if parts:
            return parts
        text = _text_from_content_value(item.get("text") or item.get("output_text"))
        if text is not None:
            return [ContentPart(kind=ContentKind.TEXT, text=text)]
        return []

    if item_type in {"output_text", "input_text", "text"}:
        text = _coerce_text(
            item.get("text") or item.get("delta") or item.get("content"),
            field_name="response.output_text",
        )
        if text is None:
            return []
        return [ContentPart(kind=ContentKind.TEXT, text=text)]

    if item_type in {"reasoning", "thinking", "redacted_thinking"}:
        text = _text_from_content_value(
            item.get("text") or item.get("summary") or item.get("content")
        )
        if text is None:
            return []
        return [
            ContentPart(
                kind=ContentKind.THINKING,
                text=text,
                thinking=ThinkingData(text=text, redacted=item_type == "redacted_thinking"),
            )
        ]

    if item_type in {"function_call", "custom_tool_call"}:
        tool_call_id = _coerce_text(
            item.get("call_id") or item.get("id") or item.get("tool_call_id"),
            field_name="response.tool_call.id",
        )
        name = _coerce_text(item.get("name"), field_name="response.tool_call.name")
        if tool_call_id is None:
            tool_call_id = name or "openai_call"
        if name is None:
            name = "tool"
        arguments = item.get("arguments")
        if arguments is None:
            arguments = item.get("input")
        raw_arguments = (
            arguments if isinstance(arguments, str) else _serialize_json_value(arguments or {})
        )
        return [
            ContentPart(
                kind=ContentKind.TOOL_CALL,
                tool_call=ToolCall(
                    id=tool_call_id,
                    name=name,
                    arguments=arguments if arguments is not None else raw_arguments,
                    raw_arguments=raw_arguments,
                    type="function",
                ),
            )
        ]

    if item_type == "function_call_output":
        call_id = _coerce_text(
            item.get("call_id") or item.get("tool_call_id") or item.get("id"),
            field_name="response.tool_result.call_id",
        )
        if call_id is None:
            call_id = "openai_call"
        output = item.get("output")
        if not isinstance(output, (str, dict, list)):
            output = _serialize_json_value(output)
        return [
            ContentPart(
                kind=ContentKind.TOOL_RESULT,
                tool_result=ToolResultData(
                    tool_call_id=call_id,
                    content=output,
                    is_error=bool(item.get("is_error") or item.get("error")),
                ),
            )
        ]

    return []


def _response_has_tool_call_parts(content_parts: Sequence[ContentPart]) -> bool:
    return any(part.kind == ContentKind.TOOL_CALL for part in content_parts)


def _normalize_openai_finish_reason(payload: Mapping[str, Any]) -> FinishReason:
    def _finish_reason_from_candidate(
        value: Any,
        *,
        raw: str | None = None,
    ) -> FinishReason:
        if isinstance(value, str):
            normalized = value.strip().casefold()
            if normalized in {"completed", "complete", "done", "success", "succeeded"}:
                return FinishReason(reason=FinishReason.STOP, raw=raw if raw is not None else value)
            if normalized in {"incomplete", "length", "max_tokens", "max_output_tokens"}:
                return FinishReason(
                    reason=FinishReason.LENGTH,
                    raw=raw if raw is not None else value,
                )
            if normalized in {"tool_calls", "tool_use"}:
                return FinishReason(
                    reason=FinishReason.TOOL_CALLS,
                    raw=raw if raw is not None else value,
                )
            if normalized in {"content_filter", "safety", "refusal"}:
                return FinishReason(
                    reason=FinishReason.CONTENT_FILTER,
                    raw=raw if raw is not None else value,
                )
            if normalized in {"error", "failed", "cancelled", "canceled"}:
                return FinishReason(
                    reason=FinishReason.ERROR,
                    raw=raw if raw is not None else value,
                )

        normalized_finish_reason = normalize_finish_reason(value, provider="openai")
        if raw is not None:
            return FinishReason(reason=normalized_finish_reason.reason, raw=raw)
        return normalized_finish_reason

    candidate = None
    for key in ("finish_reason", "finishReason", "status"):
        candidate = payload.get(key)
        if candidate is not None:
            break

    incomplete_details = payload.get("incomplete_details") or payload.get("incompleteDetails")
    incomplete_reason = None
    if isinstance(incomplete_details, Mapping):
        incomplete_reason = incomplete_details.get("reason")

    if isinstance(candidate, str):
        normalized = candidate.strip().casefold()
        if normalized == "incomplete" and incomplete_reason is not None:
            return _finish_reason_from_candidate(incomplete_reason, raw=candidate)

    if candidate is None:
        candidate = incomplete_reason

    if candidate is None:
        return FinishReason(reason=FinishReason.OTHER)

    return _finish_reason_from_candidate(candidate)


def normalize_openai_response(
    payload: Any,
    *,
    provider: str = "openai",
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
            content_parts: list[ContentPart] = []
            output = source.get("output")
            if isinstance(output, Sequence) and not isinstance(output, (str, bytes, bytearray)):
                for item in output:
                    content_parts.extend(_content_parts_from_output_item(item))
            else:
                output_text = source.get("output_text")
                if output_text is None:
                    output_text = source.get("text")
                text = _text_from_content_value(output_text)
                if text is not None:
                    content_parts.append(ContentPart(kind=ContentKind.TEXT, text=text))

            response_id = _coerce_text(
                source.get("id") or source.get("response_id"),
                field_name="response.id",
            )
            model = _coerce_text(source.get("model"), field_name="response.model")
            finish_reason = _normalize_openai_finish_reason(source)
            if (
                _response_has_tool_call_parts(content_parts)
                and finish_reason.reason not in _STRONGER_TERMINAL_FINISH_REASONS
            ):
                finish_reason = FinishReason(
                    reason=FinishReason.TOOL_CALLS,
                    raw=finish_reason.raw,
                )
            usage_payload = source.get("usage")
            warnings = normalize_warnings(source.get("warnings"))
            return Response(
                id=response_id or "",
                model=model or "",
                provider=provider,
                message=Message(role=Role.ASSISTANT, content=content_parts),
                finish_reason=finish_reason,
                usage=normalize_usage(usage_payload, provider=provider, raw=usage_payload),
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
                role=Role.ASSISTANT, content=[ContentPart(kind=ContentKind.TEXT, text=text)]
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
        logger.exception("Unexpected failure normalizing OpenAI response")
        raise ProviderError(
            "failed to normalize OpenAI response",
            provider=provider,
            raw=raw_payload,
            cause=exc,
            retryable=False,
        ) from exc


__all__ = [
    "build_openai_responses_url",
    "normalize_openai_base_url",
    "normalize_openai_response",
]
