from __future__ import annotations

import asyncio
from collections.abc import Iterable, Mapping
from dataclasses import dataclass, field
from typing import Any
from uuid import UUID, uuid4

from ..client import Client
from .events import EventKind, SessionEvent, _SessionEventStream
from .types import (
    ExecutionEnvironment,
    ProviderProfile,
    SessionClosedError,
    SessionConfig,
    SessionState,
    SessionStateError,
    SubAgentHandle,
    UserTurn,
)


def _coerce_uuid(value: UUID | str) -> UUID:
    if isinstance(value, UUID):
        return value
    return UUID(value)


def _coerce_state(value: SessionState | str) -> SessionState:
    if isinstance(value, SessionState):
        return value
    if isinstance(value, str):
        try:
            return SessionState(value)
        except ValueError:
            return SessionState[value]
    raise TypeError("state must be a SessionState or string")


def _normalize_active_subagents(
    active_subagents: Mapping[UUID | str, SubAgentHandle] | None,
) -> dict[UUID, SubAgentHandle]:
    normalized: dict[UUID, SubAgentHandle] = {}
    for key, handle in dict(active_subagents or {}).items():
        normalized_key = _coerce_uuid(key)
        normalized[normalized_key] = handle
    return normalized


@dataclass(init=False)
class Session:
    id: UUID | str = field(default_factory=uuid4)
    provider_profile: ProviderProfile = field(default_factory=ProviderProfile)
    execution_environment: ExecutionEnvironment = field(
        default_factory=ExecutionEnvironment
    )
    history: list[Any] = field(default_factory=list)
    event_queue: asyncio.Queue[SessionEvent] = field(default_factory=asyncio.Queue)
    config: SessionConfig = field(default_factory=SessionConfig)
    state: SessionState | str = SessionState.IDLE
    client: Any | None = None
    steering_queue: asyncio.Queue[str] = field(default_factory=asyncio.Queue)
    follow_up_queue: asyncio.Queue[str] = field(default_factory=asyncio.Queue)
    active_subagents: dict[UUID, SubAgentHandle] = field(default_factory=dict)
    pending_user_question: str | None = field(default=None, init=False, repr=False)
    abort_signaled: bool = field(default=False, init=False, repr=False)
    _closed_event_emitted: bool = field(default=False, init=False, repr=False)

    def __init__(
        self,
        provider_profile: ProviderProfile | None = None,
        execution_environment: ExecutionEnvironment | None = None,
        *,
        id: UUID | str | None = None,
        profile: ProviderProfile | None = None,
        execution_env: ExecutionEnvironment | None = None,
        llm_client: Any | None = None,
        client: Any | None = None,
        history: Iterable[Any] | None = None,
        event_queue: asyncio.Queue[SessionEvent] | None = None,
        config: SessionConfig | None = None,
        state: SessionState | str = SessionState.IDLE,
        steering_queue: asyncio.Queue[str] | None = None,
        follow_up_queue: asyncio.Queue[str] | None = None,
        active_subagents: Mapping[UUID | str, SubAgentHandle] | None = None,
    ) -> None:
        if provider_profile is None:
            provider_profile = profile
        if execution_environment is None:
            execution_environment = execution_env

        resolved_client = client if client is not None else llm_client

        self.id = _coerce_uuid(id or uuid4())
        self.provider_profile = (
            provider_profile if provider_profile is not None else ProviderProfile()
        )
        self.execution_environment = (
            execution_environment if execution_environment is not None else ExecutionEnvironment()
        )
        self.history = list(history or [])
        self.event_queue = event_queue if event_queue is not None else asyncio.Queue()
        self.config = config if config is not None else SessionConfig()
        self.state = _coerce_state(state)
        self.client = resolved_client if resolved_client is not None else Client()
        self.steering_queue = steering_queue if steering_queue is not None else asyncio.Queue()
        self.follow_up_queue = (
            follow_up_queue if follow_up_queue is not None else asyncio.Queue()
        )
        self.active_subagents = _normalize_active_subagents(active_subagents)
        self.pending_user_question = None
        self.abort_signaled = False
        self._closed_event_emitted = False
        self._emit_session_start()

    @property
    def session_id(self) -> UUID:
        return self.id

    @property
    def profile(self) -> ProviderProfile:
        return self.provider_profile

    @profile.setter
    def profile(self, value: ProviderProfile) -> None:
        self.provider_profile = value

    @property
    def environment(self) -> ExecutionEnvironment:
        return self.execution_environment

    @environment.setter
    def environment(self, value: ExecutionEnvironment) -> None:
        self.execution_environment = value

    @property
    def execution_env(self) -> ExecutionEnvironment:
        return self.execution_environment

    @execution_env.setter
    def execution_env(self, value: ExecutionEnvironment) -> None:
        self.execution_environment = value

    @property
    def llm_client(self) -> Any:
        return self.client

    @llm_client.setter
    def llm_client(self, value: Any) -> None:
        self.client = value

    @property
    def event_emitter(self) -> asyncio.Queue[SessionEvent]:
        return self.event_queue

    @event_emitter.setter
    def event_emitter(self, value: asyncio.Queue[SessionEvent]) -> None:
        self.event_queue = value

    @property
    def followup_queue(self) -> asyncio.Queue[str]:
        return self.follow_up_queue

    @followup_queue.setter
    def followup_queue(self, value: asyncio.Queue[str]) -> None:
        self.follow_up_queue = value

    @property
    def subagents(self) -> dict[UUID, SubAgentHandle]:
        return self.active_subagents

    @subagents.setter
    def subagents(self, value: Mapping[UUID | str, SubAgentHandle]) -> None:
        self.active_subagents = _normalize_active_subagents(value)

    @property
    def pending_question(self) -> str | None:
        return self.pending_user_question

    @pending_question.setter
    def pending_question(self, value: str | None) -> None:
        self.pending_user_question = value

    def emit_event(self, event: SessionEvent) -> None:
        if not isinstance(event, SessionEvent):
            raise TypeError("event must be a SessionEvent")
        if self.state == SessionState.CLOSED and event.kind != EventKind.SESSION_END:
            raise SessionClosedError("session is closed")
        if event.session_id != self.id:
            event = SessionEvent(kind=event.kind, session_id=self.id, data=event.data)
        self.event_queue.put_nowait(event)

    def _emit_event(self, kind: EventKind | str, data: Mapping[str, Any] | None = None) -> None:
        self.event_queue.put_nowait(
            SessionEvent(kind=kind, session_id=self.id, data=dict(data or {}))
        )

    def _emit_session_start(self) -> None:
        self._emit_event(EventKind.SESSION_START, {"state": self.state.value})

    @property
    def event_stream(self) -> _SessionEventStream:
        return _SessionEventStream(self.event_queue)

    def events(self) -> _SessionEventStream:
        return self.event_stream

    def __aiter__(self) -> _SessionEventStream:
        return self.events()

    def steer(self, message: str) -> str:
        if not isinstance(message, str):
            raise TypeError("message must be a string")
        if self.state == SessionState.CLOSED:
            raise SessionClosedError("session is closed")
        self.steering_queue.put_nowait(message)
        return message

    def follow_up(self, message: str) -> str:
        if not isinstance(message, str):
            raise TypeError("message must be a string")
        if self.state == SessionState.CLOSED:
            raise SessionClosedError("session is closed")
        self.follow_up_queue.put_nowait(message)
        return message

    def mark_awaiting_input(self, question: str | None = None) -> SessionState:
        if self.state == SessionState.CLOSED:
            raise SessionClosedError("session is closed")
        self.pending_user_question = question
        self.state = SessionState.AWAITING_INPUT
        return self.state

    def mark_user_answer(self) -> SessionState:
        if self.state == SessionState.CLOSED:
            raise SessionClosedError("session is closed")
        self.pending_user_question = None
        self.state = SessionState.PROCESSING
        return self.state

    def mark_natural_completion(self) -> SessionState:
        if self.state == SessionState.CLOSED:
            raise SessionClosedError("session is closed")
        self.state = SessionState.IDLE
        self._emit_event(EventKind.PROCESSING_END, {"state": self.state.value})
        return self.state

    def mark_turn_limit(
        self,
        *,
        round_count: int | None = None,
        total_turns: int | None = None,
    ) -> SessionState:
        if self.state == SessionState.CLOSED:
            raise SessionClosedError("session is closed")
        payload: dict[str, Any] = {"state": SessionState.IDLE.value}
        if round_count is not None:
            payload["round_count"] = round_count
        if total_turns is not None:
            payload["total_turns"] = total_turns
        self.state = SessionState.IDLE
        self._emit_event(EventKind.TURN_LIMIT, payload)
        self._emit_event(EventKind.PROCESSING_END, {"state": self.state.value})
        return self.state

    async def mark_unrecoverable_error(self, error: BaseException | str) -> SessionState:
        if self.state == SessionState.CLOSED:
            raise SessionClosedError("session is closed")
        error_message = str(error)
        self._emit_event(EventKind.ERROR, {"error": error_message})
        await self.close()
        return self.state

    async def process_input(self, user_input: str | Iterable[Any]) -> None:
        if self.state == SessionState.CLOSED:
            raise SessionClosedError("session is closed")
        if self.state == SessionState.PROCESSING:
            raise SessionStateError("session is already processing input")

        if self.state == SessionState.AWAITING_INPUT:
            self.mark_user_answer()

        user_turn = UserTurn(content=user_input)
        self.history.append(user_turn)
        self.state = SessionState.PROCESSING
        self._emit_event(EventKind.USER_INPUT, {"content": user_turn.content})

    async def submit(self, user_input: str | Iterable[Any]) -> None:
        await self.process_input(user_input)

    async def close(self) -> SessionState:
        if self._closed_event_emitted:
            self.state = SessionState.CLOSED
            return self.state
        self.state = SessionState.CLOSED
        self._emit_session_end()
        self._closed_event_emitted = True
        return self.state

    async def abort(self) -> SessionState:
        if self.state == SessionState.CLOSED:
            self.abort_signaled = True
            return self.state
        self.abort_signaled = True
        await self.close()
        return self.state

    def _emit_session_end(self) -> None:
        self._emit_event(EventKind.SESSION_END, {"state": self.state.value})


__all__ = ["Session"]
