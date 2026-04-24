from __future__ import annotations

from collections.abc import Iterable
from typing import Any

from unified_llm.tools import ToolCall
from unified_llm.types import (
    ContentKind,
    ContentPart,
    Message,
    ThinkingData,
    ToolCallData,
    ToolResultData,
)

from .types import AssistantTurn, SteeringTurn, SystemTurn, ToolResultsTurn, UserTurn


def _normalize_tool_call(tool_call: Any) -> ToolCallData | ToolCall:
    tool_call_id = getattr(tool_call, "id")
    name = getattr(tool_call, "name")
    arguments = getattr(tool_call, "arguments")
    tool_call_type = getattr(tool_call, "type", "function")
    if tool_call_type is None:
        tool_call_type = "function"
    raw_arguments = getattr(tool_call, "raw_arguments", None)
    if raw_arguments is not None:
        return ToolCall(
            id=tool_call_id,
            name=name,
            arguments=arguments,
            raw_arguments=raw_arguments,
            type=tool_call_type,
        )
    try:
        return ToolCallData(
            id=tool_call_id,
            name=name,
            arguments=arguments,
            type=tool_call_type,
        )
    except TypeError:
        return ToolCall(
            id=tool_call_id,
            name=name,
            arguments=arguments,
            type=tool_call_type,
        )


def _normalize_tool_result(tool_result: Any) -> ToolResultData | Any:
    if isinstance(tool_result, ToolResultData):
        return tool_result

    tool_call_id = getattr(tool_result, "tool_call_id")
    content = getattr(tool_result, "content")
    is_error = getattr(tool_result, "is_error")
    image_data = getattr(tool_result, "image_data", None)
    image_media_type = getattr(tool_result, "image_media_type", None)
    try:
        return ToolResultData(
            tool_call_id=tool_call_id,
            content=content,
            is_error=is_error,
            image_data=image_data,
            image_media_type=image_media_type,
        )
    except TypeError:
        return tool_result


def _clone_content_part(part: ContentPart) -> ContentPart:
    return ContentPart(
        kind=part.kind,
        text=part.text,
        image=part.image,
        audio=part.audio,
        document=part.document,
        tool_call=_normalize_tool_call(part.tool_call)
        if part.tool_call is not None
        else None,
        tool_result=_normalize_tool_result(part.tool_result)
        if part.tool_result is not None
        else None,
        thinking=part.thinking,
    )


def _copy_content(content: str | Iterable[ContentPart]) -> str | list[ContentPart]:
    if isinstance(content, str):
        return content
    return [_clone_content_part(part) for part in content]


def _content_has_kind(parts: Iterable[ContentPart], *kinds: ContentKind) -> bool:
    target_kinds = set(kinds)
    return any(part.kind in target_kinds for part in parts)


def _assistant_tool_call_part(tool_call: Any) -> ContentPart:
    return ContentPart(
        kind=ContentKind.TOOL_CALL,
        tool_call=_normalize_tool_call(tool_call),
    )


def _merge_assistant_tool_calls(
    parts: list[ContentPart],
    tool_calls: Iterable[Any],
) -> list[ContentPart]:
    merged_parts = list(parts)
    for tool_call in tool_calls:
        normalized_tool_call = _normalize_tool_call(tool_call)
        tool_call_id = getattr(normalized_tool_call, "id", None)
        replaced = False
        if tool_call_id is not None:
            for index, part in enumerate(merged_parts):
                if part.kind != ContentKind.TOOL_CALL or part.tool_call is None:
                    continue
                if getattr(part.tool_call, "id", None) != tool_call_id:
                    continue
                merged_parts[index] = ContentPart(
                    kind=part.kind,
                    text=part.text,
                    image=part.image,
                    audio=part.audio,
                    document=part.document,
                    tool_call=normalized_tool_call,
                    tool_result=part.tool_result,
                    thinking=part.thinking,
                )
                replaced = True
                break
        if not replaced:
            merged_parts.append(_assistant_tool_call_part(normalized_tool_call))
    return merged_parts


def _assistant_message_content(turn: AssistantTurn) -> list[ContentPart]:
    parts = [_clone_content_part(part) for part in turn.content_parts]
    if turn.reasoning is not None and not _content_has_kind(
        parts,
        ContentKind.THINKING,
        ContentKind.REDACTED_THINKING,
    ):
        parts.append(
            ContentPart(
                kind=ContentKind.THINKING,
                thinking=ThinkingData(text=turn.reasoning),
                text=turn.reasoning,
            )
        )
    if turn.tool_calls:
        parts = _merge_assistant_tool_calls(parts, turn.tool_calls)
    return parts


def turn_to_messages(turn: Any) -> list[Message]:
    if isinstance(turn, UserTurn):
        return [Message.user(_copy_content(turn.content))]
    if isinstance(turn, SystemTurn):
        return [Message.system(_copy_content(turn.content))]
    if isinstance(turn, SteeringTurn):
        return [Message.user(_copy_content(turn.content))]
    if isinstance(turn, AssistantTurn):
        return [Message.assistant(_assistant_message_content(turn))]
    if isinstance(turn, ToolResultsTurn):
        return [
            Message.tool_result(
                tool_call_id=getattr(result, "tool_call_id"),
                content=getattr(result, "content"),
                is_error=getattr(result, "is_error"),
                image_data=getattr(result, "image_data", None),
                image_media_type=getattr(result, "image_media_type", None),
            )
            for result in turn.result_list
        ]
    raise TypeError(f"unsupported turn type: {type(turn).__name__}")


def history_to_messages(history: Iterable[Any]) -> list[Message]:
    messages: list[Message] = []
    for turn in history:
        messages.extend(turn_to_messages(turn))
    return messages


__all__ = ["history_to_messages", "turn_to_messages"]
