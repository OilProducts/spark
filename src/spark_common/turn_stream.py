from __future__ import annotations

from dataclasses import dataclass, field
from typing import Any, Literal, Optional


TurnStreamEventKind = Literal[
    "content_delta",
    "content_completed",
    "tool_call_started",
    "tool_call_updated",
    "tool_call_completed",
    "tool_call_failed",
    "token_usage_updated",
    "request_user_input_requested",
    "context_compaction_started",
    "context_compaction_completed",
    "turn_completed",
    "error",
]

TurnStreamChannel = Literal["assistant", "reasoning", "plan"]


@dataclass
class TurnStreamSource:
    backend: Optional[str] = None
    session_id: Optional[str] = None
    app_turn_id: Optional[str] = None
    item_id: Optional[str] = None
    response_id: Optional[str] = None
    summary_index: Optional[int] = None
    raw_kind: Optional[str] = None


@dataclass
class TurnStreamEvent:
    kind: TurnStreamEventKind | str
    channel: Optional[TurnStreamChannel | str] = None
    source: TurnStreamSource = field(default_factory=TurnStreamSource)
    content_delta: Optional[str] = None
    message: Optional[str] = None
    tool_call: Optional[Any] = None
    request_user_input: Optional[Any] = None
    token_usage: Optional[dict[str, Any]] = None
    error: Optional[str] = None
    phase: Optional[str] = None
    status: Optional[str] = None

    def __post_init__(self) -> None:
        self.kind = str(self.kind)
        if self.kind in {"content_delta", "content_completed"} and self.channel is None:
            raise ValueError("content TurnStreamEvent values must set channel")
