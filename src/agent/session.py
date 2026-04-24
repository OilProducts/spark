from __future__ import annotations

import asyncio
import inspect
import logging
from collections.abc import Iterable, Mapping
from dataclasses import dataclass, field
from typing import Any
from uuid import UUID, uuid4

from unified_llm.client import Client
from unified_llm.errors import SDKError
from unified_llm.streaming import StreamAccumulator
from unified_llm.tools import Tool as SDKTool
from unified_llm.tools import ToolChoice as SDKToolChoice
from unified_llm.types import Message, Request, Response, StreamEventType

from .context import check_context_usage
from .environment import ExecutionEnvironment
from .errors import (
    is_authentication_error,
    is_context_length_error,
    is_transient_sdk_error,
)
from .events import EventKind, SessionEvent, _SessionEventStream
from .history import history_to_messages
from .local_environment import LocalExecutionEnvironment
from .loop_detection import LOOP_DETECTION_WARNING, detect_loop
from .subagents import close_active_subagents
from .tool_execution import execute_tool_calls
from .types import (
    AssistantTurn,
    ProviderProfile,
    SessionAbortedError,
    SessionClosedError,
    SessionConfig,
    SessionState,
    SessionStateError,
    SteeringTurn,
    SubAgentHandle,
    ToolDefinition,
    ToolResultsTurn,
    UserTurn,
)

logger = logging.getLogger(__name__)


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


def _sdk_tool_from_definition(tool_definition: ToolDefinition) -> SDKTool:
    return SDKTool(
        name=tool_definition.name,
        description=tool_definition.description,
        parameters=dict(tool_definition.parameters),
    )


def _build_session_system_prompt(
    provider_profile: Any,
    execution_environment: ExecutionEnvironment,
) -> str:
    build_system_prompt = getattr(provider_profile, "build_system_prompt", None)
    if callable(build_system_prompt):
        return build_system_prompt(execution_environment, None) or ""

    tool_registry = getattr(provider_profile, "tool_registry", None)
    if tool_registry is None:
        tool_registry = {}

    capabilities = getattr(provider_profile, "capabilities", None)
    if capabilities is None:
        capabilities = getattr(provider_profile, "capability_flags", {})

    fallback_profile = ProviderProfile(
        id=getattr(provider_profile, "id", ""),
        model=getattr(provider_profile, "model", ""),
        tool_registry=tool_registry,
        capabilities=capabilities or {},
        provider_options_map=getattr(provider_profile, "provider_options_map", {}) or {},
        context_window_size=getattr(provider_profile, "context_window_size", None),
        display_name=getattr(provider_profile, "display_name", None),
        knowledge_cutoff=getattr(provider_profile, "knowledge_cutoff", None),
        knowledge_cutoff_date=getattr(provider_profile, "knowledge_cutoff_date", None),
        supports_reasoning=getattr(provider_profile, "supports_reasoning", False),
        supports_streaming=getattr(provider_profile, "supports_streaming", False),
        supports_parallel_tool_calls=getattr(
            provider_profile,
            "supports_parallel_tool_calls",
            False,
        ),
    )
    return fallback_profile.build_system_prompt(execution_environment, None) or ""


@dataclass(init=False)
class Session:
    id: UUID | str = field(default_factory=uuid4)
    provider_profile: ProviderProfile = field(default_factory=ProviderProfile)
    execution_environment: ExecutionEnvironment = field(
        default_factory=LocalExecutionEnvironment
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
    _processing_task: asyncio.Task[Any] | None = field(default=None, init=False, repr=False)
    _context_warning_emitted: bool = field(default=False, init=False, repr=False)
    _closing: bool = field(default=False, init=False, repr=False)
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
            execution_environment
            if execution_environment is not None
            else LocalExecutionEnvironment()
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
        self._processing_task = None
        self._context_warning_emitted = False
        self._closing = False
        self._closed_event_emitted = False
        self._system_prompt = _build_session_system_prompt(
            self.provider_profile,
            self.execution_environment,
        )
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

    def _provider_tools(self) -> list[SDKTool]:
        return [_sdk_tool_from_definition(tool) for tool in self.provider_profile.tools()]

    def _provider_options(self) -> dict[str, Any] | None:
        provider = self.provider_profile.id or None
        if provider is None:
            return None

        provider_options_method = getattr(self.provider_profile, "provider_options", None)
        if not callable(provider_options_method):
            return None

        try:
            signature = inspect.signature(provider_options_method)
        except (TypeError, ValueError):
            signature = None

        accepts_session_config = False
        if signature is not None:
            parameters = list(signature.parameters.values())
            accepts_session_config = any(
                parameter.kind == inspect.Parameter.VAR_POSITIONAL
                for parameter in parameters
            ) or sum(
                parameter.kind
                in (
                    inspect.Parameter.POSITIONAL_ONLY,
                    inspect.Parameter.POSITIONAL_OR_KEYWORD,
                )
                for parameter in parameters
            ) >= 1

        if accepts_session_config:
            try:
                options = provider_options_method(self.config)
            except TypeError:
                options = provider_options_method()
        else:
            options = provider_options_method()

        if not isinstance(options, Mapping):
            return None
        return {provider: dict(options)}

    def _drain_steering_queue(self) -> list[SteeringTurn]:
        drained: list[SteeringTurn] = []
        while True:
            try:
                message = self.steering_queue.get_nowait()
            except asyncio.QueueEmpty:
                break
            steering_turn = SteeringTurn(content=message)
            self.history.append(steering_turn)
            self._emit_event(
                EventKind.STEERING_INJECTED,
                {"content": steering_turn.content},
            )
            drained.append(steering_turn)
        return drained

    def _maybe_emit_loop_detection_warning(self) -> bool:
        if not self.config.enable_loop_detection:
            return False
        if not detect_loop(self.history, window=self.config.loop_detection_window):
            return False

        warning_turn = SteeringTurn(content=LOOP_DETECTION_WARNING)
        self.history.append(warning_turn)
        self._emit_event(
            EventKind.LOOP_DETECTION,
            {"message": LOOP_DETECTION_WARNING},
        )
        return True

    def _emit_session_warning(self, message: str) -> None:
        if self.state == SessionState.CLOSED:
            return
        self._emit_event(EventKind.WARNING, {"message": message})

    async def _cancel_processing_task(self) -> None:
        processing_task = self._processing_task
        current_task = asyncio.current_task()
        if (
            processing_task is None
            or processing_task is current_task
            or processing_task.done()
        ):
            return

        processing_task.cancel()
        try:
            await asyncio.wait_for(asyncio.shield(processing_task), timeout=2)
        except TimeoutError:
            logger.warning("Timed out waiting for session processing task to stop")
        except asyncio.CancelledError:
            pass
        except Exception:
            logger.exception("Unexpected error while waiting for session shutdown")

    async def _cleanup_execution_environment(self) -> None:
        cleanup = getattr(self.execution_environment, "cleanup", None)
        if not callable(cleanup):
            return

        try:
            result = cleanup()
            if inspect.isawaitable(result):
                await result
        except Exception:
            logger.exception("Unexpected error while cleaning up the execution environment")

    async def _handle_model_error(self, error: BaseException) -> None:
        if self.state == SessionState.CLOSED:
            return

        if is_authentication_error(error):
            logger.error("Authentication error while processing session input: %s", error)
            self._emit_event(EventKind.ERROR, {"error": str(error)})
            await self.close()
            return

        if is_context_length_error(error):
            logger.warning("Context length error while processing session input: %s", error)
            self._emit_session_warning(str(error))
            self.mark_natural_completion()
            return

        if isinstance(error, SDKError):
            if is_transient_sdk_error(error):
                logger.warning("Transient SDK error while processing session input: %s", error)
                self.mark_natural_completion()
                return

            logger.error("SDK error while processing session input: %s", error)
            await self.mark_unrecoverable_error(error)
            return

        logger.exception("Unexpected error while processing session input")
        await self.mark_unrecoverable_error(error)

    def _emit_assistant_text_events(
        self,
        *,
        response_text: str,
        reasoning: str | None,
        response_id: str | None = None,
    ) -> None:
        self._emit_assistant_text_start(response_id)
        self._emit_assistant_text_delta(response_text, response_id)
        self._emit_assistant_text_end(response_text, reasoning)

    def _emit_assistant_text_start(self, response_id: str | None = None) -> None:
        payload: dict[str, Any] = {}
        if response_id not in (None, ""):
            payload["response_id"] = response_id
        self._emit_event(EventKind.ASSISTANT_TEXT_START, payload)

    def _emit_assistant_text_delta(
        self,
        delta: str,
        response_id: str | None = None,
    ) -> None:
        payload: dict[str, Any] = {"delta": delta}
        if response_id not in (None, ""):
            payload["response_id"] = response_id
        self._emit_event(EventKind.ASSISTANT_TEXT_DELTA, payload)

    def _emit_assistant_text_end(self, text: str, reasoning: str | None) -> None:
        self._emit_event(
            EventKind.ASSISTANT_TEXT_END,
            {"text": text, "reasoning": reasoning},
        )

    def _assistant_turn_from_response(
        self,
        response: Response,
        *,
        error: BaseException | None = None,
    ) -> AssistantTurn:
        response_text = getattr(response, "text", "")
        if response_text is None:
            response_text = ""
        elif not isinstance(response_text, str):
            response_text = str(response_text)

        return AssistantTurn(
            content=response_text,
            tool_calls=list(getattr(response, "tool_calls", []) or []),
            reasoning=getattr(response, "reasoning", None),
            usage=getattr(response, "usage", None),
            response_id=getattr(response, "id", None),
            finish_reason=getattr(response, "finish_reason", None),
            raw=getattr(response, "raw", None),
            warnings=list(getattr(response, "warnings", []) or []),
            error=error,
        )

    def _default_text_completion_is_question(self, response_text: str) -> bool:
        stripped_text = response_text.rstrip()
        return bool(stripped_text) and stripped_text.endswith("?")

    def _profile_text_completion_is_question(
        self,
        response_text: str,
        assistant_turn: AssistantTurn,
    ) -> bool:
        classifier = getattr(self.provider_profile, "classify_text_completion", None)
        if not callable(classifier):
            return False

        try:
            signature = inspect.signature(classifier)
        except (TypeError, ValueError):
            result = classifier(response_text, assistant_turn)
        else:
            parameters = list(signature.parameters.values())
            positional_parameter_count = sum(
                parameter.kind
                in (
                    inspect.Parameter.POSITIONAL_ONLY,
                    inspect.Parameter.POSITIONAL_OR_KEYWORD,
                )
                for parameter in parameters
            )
            if any(
                parameter.kind == inspect.Parameter.VAR_POSITIONAL
                for parameter in parameters
            ) or positional_parameter_count >= 2:
                result = classifier(response_text, assistant_turn)
            elif positional_parameter_count == 1:
                result = classifier(response_text)
            else:
                result = classifier()

        return result is True

    def _assistant_response_is_open_question(self, assistant_turn: AssistantTurn) -> bool:
        response_text = assistant_turn.text
        if self._profile_text_completion_is_question(response_text, assistant_turn):
            return True
        return self._default_text_completion_is_question(response_text)

    def _next_follow_up(self) -> str | None:
        try:
            return self.follow_up_queue.get_nowait()
        except asyncio.QueueEmpty:
            return None

    async def _guard_before_model_request(self, *, round_count: int) -> bool:
        if self.abort_signaled:
            await self.close()
            raise SessionAbortedError("session is aborted")

        if self.state == SessionState.CLOSED:
            raise SessionClosedError("session is closed")

        if self.config.max_tool_rounds_per_input > 0 and (
            round_count >= self.config.max_tool_rounds_per_input
        ):
            self.mark_turn_limit(
                round_count=round_count,
                total_turns=len(self.history),
            )
            return False

        if self.config.max_turns > 0 and len(self.history) >= self.config.max_turns:
            self.mark_turn_limit(
                round_count=round_count,
                total_turns=len(self.history),
            )
            return False

        check_context_usage(self)
        return True

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

    async def _complete_response(self, request: Request) -> tuple[Response, BaseException | None]:
        response = self.client.complete(request)
        if inspect.isawaitable(response):
            response = await response
        self._emit_assistant_text_events(
            response_text=getattr(response, "text", ""),
            reasoning=getattr(response, "reasoning", None),
            response_id=getattr(response, "id", None),
        )
        return response, None

    async def _stream_response(self, request: Request) -> tuple[Response, BaseException | None]:
        accumulator = StreamAccumulator(
            model=request.model,
            provider=request.provider or "",
        )
        assistant_text_started = False
        response_id: str | None = None
        async for stream_event in self.client.stream(request):
            accumulator.add(stream_event)
            current_response_id = accumulator.response.id or response_id
            if current_response_id not in (None, ""):
                response_id = current_response_id

            if stream_event.type == StreamEventType.TEXT_START:
                if not assistant_text_started:
                    self._emit_assistant_text_start(response_id)
                    assistant_text_started = True
                if stream_event.delta is not None:
                    self._emit_assistant_text_delta(stream_event.delta, response_id)
                continue

            if stream_event.type == StreamEventType.TEXT_DELTA:
                if not assistant_text_started:
                    self._emit_assistant_text_start(response_id)
                    assistant_text_started = True
                if stream_event.delta is not None:
                    self._emit_assistant_text_delta(stream_event.delta, response_id)
                continue

            if stream_event.type == StreamEventType.TEXT_END and not assistant_text_started:
                self._emit_assistant_text_start(response_id)
                assistant_text_started = True

        if assistant_text_started:
            self._emit_assistant_text_end(
                accumulator.response.text,
                accumulator.response.reasoning,
            )
        else:
            self._emit_assistant_text_events(
                response_text=accumulator.response.text,
                reasoning=accumulator.response.reasoning,
                response_id=response_id,
            )
        return accumulator.response, accumulator.error

    async def _model_response(self, request: Request) -> tuple[Response, BaseException | None]:
        if self.provider_profile.supports_streaming:
            return await self._stream_response(request)
        return await self._complete_response(request)

    async def process_input(self, user_input: str | Iterable[Any]) -> None:
        processing_task = asyncio.current_task()
        self._processing_task = processing_task
        try:
            if self.state == SessionState.CLOSED:
                raise SessionClosedError("session is closed")
            if self.state == SessionState.PROCESSING:
                raise SessionStateError("session is already processing input")
            if self.state not in (SessionState.IDLE, SessionState.AWAITING_INPUT):
                raise SessionStateError("session is not ready for input")

            answer_to_question: str | None = None
            if self.state == SessionState.AWAITING_INPUT:
                answer_to_question = self.pending_user_question
                self.mark_user_answer()
            else:
                self.pending_user_question = None
                self.state = SessionState.PROCESSING

            system_prompt = getattr(self, "_system_prompt", None)
            if system_prompt is None:
                system_prompt = _build_session_system_prompt(
                    self.provider_profile,
                    self.execution_environment,
                )
                self._system_prompt = system_prompt

            current_input: str | Iterable[Any] = user_input
            while True:
                user_turn = UserTurn(content=current_input)
                self.history.append(user_turn)
                event_data: dict[str, Any] = {"content": user_turn.content}
                if answer_to_question is not None:
                    event_data["answer_to"] = answer_to_question
                    answer_to_question = None
                self._emit_event(EventKind.USER_INPUT, event_data)
                self._drain_steering_queue()
                check_context_usage(self)

                round_count = 0
                while True:
                    if not await self._guard_before_model_request(round_count=round_count):
                        return

                    request = self.build_request(system_prompt)

                    try:
                        response, stream_error = await self._model_response(request)
                    except Exception as exc:
                        await self._handle_model_error(exc)
                        raise

                    assistant_turn = self._assistant_turn_from_response(
                        response,
                        error=stream_error,
                    )
                    self.history.append(assistant_turn)

                    if stream_error is not None:
                        await self._handle_model_error(stream_error)
                        raise stream_error

                    if not assistant_turn.tool_calls:
                        if self._assistant_response_is_open_question(assistant_turn):
                            self.mark_awaiting_input(assistant_turn.text)
                            return
                        break

                    round_count += 1

                    try:
                        tool_results = await execute_tool_calls(self, assistant_turn.tool_calls)
                    except Exception as exc:
                        logger.exception("Unexpected error executing tool calls")
                        await self.mark_unrecoverable_error(exc)
                        raise

                    self.history.append(ToolResultsTurn(result_list=tool_results))
                    self._drain_steering_queue()
                    check_context_usage(self)
                    self._maybe_emit_loop_detection_warning()

                next_follow_up = self._next_follow_up()
                if next_follow_up is None:
                    self.mark_natural_completion()
                    return
                current_input = next_follow_up
        except asyncio.CancelledError as exc:
            if self.abort_signaled:
                await self.close()
                raise SessionAbortedError("session is aborted") from exc
            raise
        finally:
            if self._processing_task is processing_task:
                self._processing_task = None

    async def submit(self, user_input: str | Iterable[Any]) -> None:
        await self.process_input(user_input)

    def build_request(self, system_prompt: str) -> Request:
        if not isinstance(system_prompt, str):
            raise TypeError("system_prompt must be a string")

        messages = [Message.system(system_prompt), *history_to_messages(self.history)]
        provider = self.provider_profile.id or None
        tools = self._provider_tools()
        return Request(
            model=self.provider_profile.model,
            provider=provider,
            messages=messages,
            tools=tools,
            tool_choice=SDKToolChoice(mode="auto", tool_name=None) if tools else None,
            reasoning_effort=self.config.reasoning_effort,
            provider_options=self._provider_options(),
        )

    async def close(self) -> SessionState:
        if self._closed_event_emitted:
            self.state = SessionState.CLOSED
            return self.state
        if self._closing:
            return self.state

        self._closing = True
        self.state = SessionState.CLOSED
        try:
            await self._cancel_processing_task()
            await close_active_subagents(self)
            await self._cleanup_execution_environment()
            if not self._closed_event_emitted:
                self._emit_session_end()
                self._closed_event_emitted = True
        finally:
            self._closing = False
        return self.state

    async def abort(self) -> SessionState:
        self.abort_signaled = True
        processing_task = self._processing_task
        current_task = asyncio.current_task()
        if (
            processing_task is not None
            and processing_task is not current_task
            and not processing_task.done()
        ):
            processing_task.cancel()
        await self.close()
        return self.state

    def _emit_session_end(self) -> None:
        self._emit_event(EventKind.SESSION_END, {"state": self.state.value})


__all__ = ["Session"]
