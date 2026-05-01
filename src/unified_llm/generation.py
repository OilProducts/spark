from __future__ import annotations

import asyncio
import inspect
import logging
from collections.abc import AsyncIterator, Callable, Iterable, Mapping, Sequence
from contextlib import suppress
from dataclasses import dataclass, replace
from typing import Any

from .client import Client
from .defaults import get_default_client
from .errors import AbortError, ConfigurationError
from .models import get_latest_model
from .retry import RetryPolicy
from .retry import retry as retry_operation
from .streaming import StreamAccumulator
from .timeouts import (
    AbortSignal,
    TimeoutConfig,
    await_with_timeout,
    check_abort,
    coerce_timeout_config,
    deadline_after,
    remaining_timeout,
)
from .tools import Tool, ToolCall, ToolChoice, ToolResult, execute_tool_call
from .types import (
    FinishReason,
    Message,
    Request,
    Response,
    ResponseFormat,
    StreamEvent,
    StreamEventType,
    Usage,
    Warning,
    _PlaceholderRecord,
)

logger = logging.getLogger(__name__)

_TOOL_CALLS_REASON = FinishReason.TOOL_CALLS.value
_sleep = asyncio.sleep


def _is_int_like(value: Any) -> bool:
    return isinstance(value, int) and not isinstance(value, bool)


def _coerce_optional_str(value: Any, field_name: str) -> str | None:
    if value is None:
        return None
    if not isinstance(value, str):
        raise TypeError(f"{field_name} must be a string or None")
    return value


def _coerce_message_sequence(messages: Iterable[Message] | None) -> list[Message] | None:
    if messages is None:
        return None
    if isinstance(messages, (str, bytes, bytearray, memoryview)):
        raise TypeError("messages must be an iterable of Message instances or None")

    try:
        normalized = list(messages)
    except TypeError as exc:
        raise TypeError("messages must be an iterable of Message instances or None") from exc

    for message in normalized:
        if not isinstance(message, Message):
            raise TypeError("messages must contain only Message instances")
    return normalized


def _coerce_tool_sequence(tools: Iterable[Tool] | None) -> list[Tool] | None:
    if tools is None:
        return None
    if isinstance(tools, (str, bytes, bytearray, memoryview)):
        raise TypeError("tools must be an iterable of Tool instances or None")

    try:
        normalized = list(tools)
    except TypeError as exc:
        raise TypeError("tools must be an iterable of Tool instances or None") from exc

    for tool in normalized:
        if not isinstance(tool, Tool):
            raise TypeError("tools must contain only Tool instances")
    return normalized


def _coerce_str_sequence(values: Iterable[str] | None, field_name: str) -> list[str] | None:
    if values is None:
        return None
    if isinstance(values, (str, bytes, bytearray, memoryview)):
        raise TypeError(f"{field_name} must be an iterable of strings or None")

    try:
        normalized = list(values)
    except TypeError as exc:
        raise TypeError(f"{field_name} must be an iterable of strings or None") from exc

    for value in normalized:
        if not isinstance(value, str):
            raise TypeError(f"{field_name} must contain only strings")
    return normalized


def _coerce_provider_options(
    provider_options: Mapping[str, Any] | None,
) -> dict[str, Any] | None:
    if provider_options is None:
        return None
    if not isinstance(provider_options, Mapping):
        raise TypeError("provider_options must be a mapping or None")
    return dict(provider_options)


def _sum_usage(values: Iterable[Usage]) -> Usage:
    total = Usage()
    for usage in values:
        total = total + usage
    return total


def _resolve_generation_model(
    model: str | None,
    *,
    provider: str | None,
    client: Client,
) -> str:
    if model is not None:
        if not isinstance(model, str):
            raise TypeError("model must be a string or None")
        return model

    if provider is not None and not isinstance(provider, str):
        raise TypeError("provider must be a string or None")

    resolved_provider = provider if provider is not None else client.default_provider
    if resolved_provider is None:
        raise ConfigurationError(
            "No provider configured for omitted model; pass provider, "
            "set Client.default_provider, or pass model explicitly"
        )

    latest_model = get_latest_model(resolved_provider)
    if latest_model is None:
        raise ConfigurationError(
            f"No latest model configured for provider {resolved_provider!r}; "
            "pass model explicitly"
        )
    return latest_model.id


@dataclass(slots=True)
class _GenerationConfig:
    client: Client
    base_request: Request
    conversation: list[Message]
    tools: list[Tool] | None
    stop_when: Callable[[Sequence[StepResult]], bool] | None
    max_tool_rounds: int
    timeout_config: TimeoutConfig | None
    total_deadline: float | None
    max_retries: int
    abort_signal: AbortSignal | None
    repair_tool_call: Callable[..., Any] | None


def _prepare_generation_config(
    model: str | None = None,
    *,
    prompt: str | None = None,
    messages: Iterable[Message] | None = None,
    system: str | None = None,
    tools: Iterable[Tool] | None = None,
    tool_choice: ToolChoice | None = None,
    max_tool_rounds: int = 1,
    stop_when: Callable[[Sequence[StepResult]], bool] | None = None,
    response_format: ResponseFormat | None = None,
    temperature: float | None = None,
    top_p: float | None = None,
    max_tokens: int | None = None,
    stop_sequences: Iterable[str] | None = None,
    reasoning_effort: str | None = None,
    provider: str | None = None,
    provider_options: Mapping[str, Any] | None = None,
    max_retries: int = 2,
    timeout: float | TimeoutConfig | None = None,
    abort_signal: AbortSignal | None = None,
    repair_tool_call: Callable[..., Any] | None = None,
    client: Client | None = None,
) -> _GenerationConfig:
    if prompt is not None and not isinstance(prompt, str):
        raise TypeError("prompt must be a string or None")
    if system is not None and not isinstance(system, str):
        raise TypeError("system must be a string or None")
    if prompt is not None and messages is not None:
        raise ValueError("prompt and messages are mutually exclusive")
    if prompt is None and messages is None:
        raise ValueError("either prompt or messages must be provided")
    if tool_choice is not None and not isinstance(tool_choice, ToolChoice):
        raise TypeError("tool_choice must be a ToolChoice or None")
    if stop_when is not None and not callable(stop_when):
        raise TypeError("stop_when must be callable or None")
    if not _is_int_like(max_tool_rounds):
        raise TypeError("max_tool_rounds must be an integer")
    if max_tool_rounds < 0:
        raise ValueError("max_tool_rounds must be non-negative")
    if not _is_int_like(max_retries):
        raise TypeError("max_retries must be an integer")
    if max_retries < 0:
        raise ValueError("max_retries must be non-negative")
    if abort_signal is not None and not isinstance(abort_signal, AbortSignal):
        raise TypeError("abort_signal must be an AbortSignal or None")
    if repair_tool_call is not None and not callable(repair_tool_call):
        raise TypeError("repair_tool_call must be callable or None")
    if client is not None and not isinstance(client, Client):
        raise TypeError("client must be a Client or None")

    timeout_config = coerce_timeout_config(timeout)
    tools_list = _coerce_tool_sequence(tools)
    provider_options_dict = _coerce_provider_options(provider_options)
    message_list = _coerce_message_sequence(messages)
    stop_sequences_list = _coerce_str_sequence(stop_sequences, "stop_sequences")

    if prompt is not None:
        conversation = [Message.user(prompt)]
    else:
        conversation = list(message_list or [])
    if system is not None:
        conversation = [Message.system(system), *conversation]

    resolved_tool_choice = tool_choice
    if resolved_tool_choice is None and tools is not None:
        resolved_tool_choice = ToolChoice.auto()

    check_abort(abort_signal)
    resolved_client = client if client is not None else get_default_client()
    resolved_model = _resolve_generation_model(
        model,
        provider=provider,
        client=resolved_client,
    )
    total_deadline = deadline_after(timeout_config.total) if timeout_config is not None else None

    base_request = Request(
        model=resolved_model,
        messages=list(conversation),
        provider=provider,
        tools=tools_list,
        tool_choice=resolved_tool_choice,
        response_format=response_format,
        temperature=temperature,
        top_p=top_p,
        max_tokens=max_tokens,
        stop_sequences=stop_sequences_list,
        reasoning_effort=reasoning_effort,
        provider_options=provider_options_dict,
    )

    return _GenerationConfig(
        client=resolved_client,
        base_request=base_request,
        conversation=list(conversation),
        tools=tools_list,
        stop_when=stop_when,
        max_tool_rounds=max_tool_rounds,
        timeout_config=timeout_config,
        total_deadline=total_deadline,
        max_retries=max_retries,
        abort_signal=abort_signal,
        repair_tool_call=repair_tool_call,
    )


def _select_stream_timeout(
    timeout_config: TimeoutConfig | None,
    *,
    total_deadline: float | None,
    step_deadline: float | None,
) -> tuple[float | None, str]:
    candidates: list[tuple[float, str]] = []

    if total_deadline is not None:
        total_remaining = remaining_timeout(total_deadline)
        if total_remaining is not None:
            candidates.append((total_remaining, "stream"))

    if step_deadline is not None:
        step_remaining = remaining_timeout(step_deadline)
        if step_remaining is not None:
            candidates.append((step_remaining, "stream step"))

    if timeout_config is not None and timeout_config.stream_read is not None:
        candidates.append((timeout_config.stream_read, "stream_read"))

    if not candidates:
        return None, "stream_read"

    timeout, scope = min(candidates, key=lambda item: item[0])
    return timeout, scope


async def _read_stream_event(
    iterator: Any,
    *,
    timeout_config: TimeoutConfig | None,
    total_deadline: float | None,
    step_deadline: float | None,
    abort_signal: AbortSignal | None,
) -> StreamEvent:
    timeout, scope = _select_stream_timeout(
        timeout_config,
        total_deadline=total_deadline,
        step_deadline=step_deadline,
    )
    event_task = asyncio.create_task(iterator.__anext__())
    try:
        return await await_with_timeout(
            event_task,
            timeout,
            scope=scope,
            abort_signal=abort_signal,
        )
    finally:
        if not event_task.done():
            event_task.cancel()
            with suppress(asyncio.CancelledError):
                await event_task


async def _close_async_iterator(iterator: Any) -> None:
    if iterator is None:
        return

    close = getattr(iterator, "aclose", None)
    if close is None:
        close = getattr(iterator, "close", None)
    if close is None:
        return

    try:
        result = close()
        if inspect.isawaitable(result):
            await result
    except Exception:
        logger.exception("Unexpected error closing stream iterator")


def _best_effort_close_awaitable(awaitable: Any) -> None:
    if not inspect.isawaitable(awaitable):
        return

    if inspect.iscoroutine(awaitable):
        coroutine = awaitable
        try:
            coroutine.send(None)
        except StopIteration:
            return
        except Exception:
            logger.exception("Unexpected error closing stream iterator")
            coroutine.close()
            return

        try:
            loop = asyncio.get_running_loop()
        except RuntimeError:
            coroutine.close()
            return

        loop.create_task(coroutine)
        return

    try:
        loop = asyncio.get_running_loop()
    except RuntimeError:
        return

    async def _await_close() -> None:
        await awaitable

    loop.create_task(_await_close())


@dataclass(slots=True, eq=True)
class GenerateResult:
    text: str
    reasoning: str | None
    tool_calls: list[ToolCall]
    tool_results: list[ToolResult]
    finish_reason: FinishReason
    usage: Usage
    total_usage: Usage
    steps: list[StepResult]
    response: Response
    output: Any | None = None


@dataclass(slots=True, eq=True)
class StepResult:
    text: str
    reasoning: str | None
    tool_calls: list[ToolCall]
    tool_results: list[ToolResult]
    finish_reason: FinishReason
    usage: Usage
    response: Response
    warnings: list[Warning]


class StreamResult(_PlaceholderRecord):
    def __init__(
        self,
        config: _GenerationConfig | None = None,
        **placeholder_fields: Any,
    ) -> None:
        super().__init__(**placeholder_fields)
        self._config = config
        self._iterator: AsyncIterator[StreamEvent] | None = None
        self._closed = False
        self._finished = False
        self._terminal_error: BaseException | None = None
        self._final_response: Response | None = None
        if config is not None:
            self._partial_response = Response()

    def __aiter__(self) -> AsyncIterator[StreamEvent]:
        return _StreamResultIterator(self)

    async def __anext__(self) -> StreamEvent:
        if self._config is None:
            raise TypeError("StreamResult requires a stream config")

        if self._closed or self._finished:
            raise StopAsyncIteration

        if self._iterator is None:
            self._iterator = self._stream_events()

        try:
            return await self._iterator.__anext__()
        except StopAsyncIteration:
            self._finished = True
            self._closed = True
            self._iterator = None
            raise
        except Exception as error:
            self._terminal_error = error
            self._finished = True
            await self.close()
            raise

    @property
    def text_stream(self) -> AsyncIterator[str] | StreamResult:
        if self._config is None:
            return self
        return _StreamTextIterator(self)

    @property
    def partial_response(self) -> Response | Any:
        if self._config is None:
            return self.__dict__.get("partial_response")
        return self._partial_response

    async def response(self) -> Response | Any:
        if self._config is None:
            raise TypeError("StreamResult requires a stream config")

        if self._terminal_error is not None:
            raise self._terminal_error

        if not self._finished:
            async for _ in self:
                pass

        if self._terminal_error is not None:
            raise self._terminal_error

        if self._final_response is not None:
            return self._final_response
        return self._partial_response

    async def close(self) -> None:
        self._closed = True
        self._finished = True

        iterator = self._iterator
        self._iterator = None
        await _close_async_iterator(iterator)

    async def aclose(self) -> None:
        await self.close()

    async def _stream_events(self) -> AsyncIterator[StreamEvent]:
        config = self._config
        if config is None:
            raise TypeError("StreamResult requires a stream config")

        conversation = list(config.conversation)
        steps: list[StepResult] = []
        tool_rounds_executed = 0
        retry_policy = RetryPolicy(max_retries=config.max_retries)
        stream_yielded_any = False
        try:
            while True:
                check_abort(config.abort_signal)
                current_request = replace(config.base_request, messages=list(conversation))
                current_stream: Any | None = None
                step_accumulator = StreamAccumulator(
                    response=Response(
                        model=current_request.model,
                        provider=current_request.provider or "",
                    )
                )
                step_attempt = 0
                terminal_response: Response | None = None

                while True:
                    try:
                        current_stream = config.client.stream(current_request)
                        iterator = current_stream.__aiter__()
                        step_deadline = (
                            deadline_after(config.timeout_config.per_step)
                            if config.timeout_config is not None
                            else None
                        )
                        while True:
                            event = await _read_stream_event(
                                iterator,
                                timeout_config=config.timeout_config,
                                total_deadline=config.total_deadline,
                                step_deadline=step_deadline,
                                abort_signal=config.abort_signal,
                            )
                            stream_yielded_any = True
                            self._partial_response = step_accumulator.add(event)
                            yield event
                            if event.type in (
                                StreamEventType.FINISH,
                                StreamEventType.ERROR,
                            ):
                                break
                        break
                    except StopAsyncIteration:
                        break
                    except AbortError:
                        raise
                    except Exception as error:
                        if not stream_yielded_any and retry_policy.is_retryable_error(error):
                            if step_attempt >= retry_policy.max_retries:
                                raise

                            delay = retry_policy.calculate_delay(
                                step_attempt,
                                error=error,
                            )
                            if delay is None:
                                raise

                            if retry_policy.on_retry is not None:
                                callback_result = retry_policy.on_retry(
                                    error,
                                    step_attempt,
                                    delay,
                                )
                                if inspect.isawaitable(callback_result):
                                    await callback_result

                            logger.debug(
                                "Retrying %s after %.3fs (attempt %d of %d)",
                                error.__class__.__name__,
                                delay,
                                step_attempt + 1,
                                retry_policy.max_retries,
                            )
                            await await_with_timeout(
                                _sleep(delay),
                                remaining_timeout(config.total_deadline),
                                scope="stream",
                                abort_signal=config.abort_signal,
                            )
                            step_attempt += 1
                            continue
                        if stream_yielded_any:
                            terminal_response = replace(
                                self._partial_response,
                                finish_reason=FinishReason.ERROR,
                            )
                            self._partial_response = terminal_response
                            self._final_response = terminal_response
                            yield StreamEvent(
                                type=StreamEventType.ERROR,
                                error=error,
                                response=terminal_response,
                                finish_reason=terminal_response.finish_reason,
                                usage=terminal_response.usage,
                            )
                            break
                        raise
                    finally:
                        await _close_async_iterator(current_stream)

                step_response = terminal_response or step_accumulator.response
                self._partial_response = step_response

                step_result = StepResult(
                    text=step_response.text,
                    reasoning=step_response.reasoning,
                    tool_calls=list(step_response.tool_calls),
                    tool_results=[],
                    finish_reason=step_response.finish_reason,
                    usage=step_response.usage,
                    response=step_response,
                    warnings=list(step_response.warnings),
                )
                steps.append(step_result)
                self._final_response = step_response

                has_passive_tool_call = _has_passive_tool_call(
                    config.tools,
                    step_result.tool_calls,
                )

                if (
                    not step_result.tool_calls
                    or step_result.finish_reason.reason != _TOOL_CALLS_REASON
                    or has_passive_tool_call
                    or tool_rounds_executed >= config.max_tool_rounds
                ):
                    break

                if config.stop_when is not None and config.stop_when(steps):
                    break

                check_abort(config.abort_signal)
                tool_results = await _execute_tool_calls(
                    config.tools,
                    step_result.tool_calls,
                    messages=conversation,
                    abort_signal=config.abort_signal,
                    total_deadline=config.total_deadline,
                    repair_tool_call=config.repair_tool_call,
                )
                step_result.tool_results = list(tool_results)
                tool_rounds_executed += 1

                tool_result_messages = [
                    tool_result.to_message() for tool_result in tool_results
                ]
                conversation = [
                    *conversation,
                    step_result.response.message,
                    *tool_result_messages,
                ]

                yield StreamEvent(
                    type="step_finish",
                    response=step_result.response,
                    finish_reason=step_result.finish_reason,
                    usage=step_result.usage,
                    raw={"step": len(steps)},
                )
                stream_yielded_any = True
        finally:
            self._closed = True
            self._finished = True
            if self._final_response is None:
                self._final_response = self._partial_response


class _StreamResultIterator(AsyncIterator[StreamEvent]):
    def __init__(self, stream: StreamResult) -> None:
        self._stream = stream

    def __aiter__(self) -> _StreamResultIterator:
        return self

    def __del__(self) -> None:
        _best_effort_close_awaitable(self._stream.close())

    async def __anext__(self) -> StreamEvent:
        return await self._stream.__anext__()

    async def aclose(self) -> None:
        await self._stream.close()

    async def close(self) -> None:
        await self.aclose()


class _StreamTextIterator(AsyncIterator[str]):
    def __init__(self, stream: StreamResult) -> None:
        self._stream = stream

    def __aiter__(self) -> _StreamTextIterator:
        return self

    def __del__(self) -> None:
        _best_effort_close_awaitable(self._stream.close())

    async def __anext__(self) -> str:
        while True:
            event = await self._stream.__anext__()
            if event.type == StreamEventType.TEXT_DELTA and event.delta is not None:
                return event.delta

    async def aclose(self) -> None:
        await self._stream.close()

    async def close(self) -> None:
        await self.aclose()


async def _complete_request_with_retries(
    client: Client,
    request: Request,
    *,
    timeout_config: TimeoutConfig | None,
    total_deadline: float | None,
    abort_signal: AbortSignal | None,
    max_retries: int,
) -> Response:
    retry_policy = RetryPolicy(max_retries=max_retries)

    async def attempt() -> Response:
        per_step_timeout = timeout_config.per_step if timeout_config is not None else None
        return await await_with_timeout(
            client.complete(request),
            per_step_timeout,
            scope="generation step",
        )

    return await await_with_timeout(
        retry_operation(attempt, policy=retry_policy),
        remaining_timeout(total_deadline),
        scope="generation",
        abort_signal=abort_signal,
    )


async def _execute_tool_calls(
    tools: list[Tool] | None,
    tool_calls: Sequence[ToolCall],
    *,
    messages: Sequence[Message],
    abort_signal: AbortSignal | None,
    total_deadline: float | None,
    repair_tool_call: Callable[..., Any] | None,
) -> list[ToolResult]:
    if not tool_calls:
        return []

    tool_map = {tool.name: tool for tool in tools or ()}
    message_snapshot = list(messages)
    tasks = [
        execute_tool_call(
            tool_map.get(tool_call.name),
            tool_call,
            messages=message_snapshot,
            abort_signal=abort_signal,
            repair_tool_call=repair_tool_call,
        )
        for tool_call in tool_calls
    ]
    return list(
        await await_with_timeout(
            asyncio.gather(*tasks),
            remaining_timeout(total_deadline),
            scope="tool execution",
            abort_signal=abort_signal,
        )
    )


def _has_passive_tool_call(
    tools: list[Tool] | None,
    tool_calls: Sequence[ToolCall],
) -> bool:
    if not tools or not tool_calls:
        return False

    tool_map = {tool.name: tool for tool in tools}
    for tool_call in tool_calls:
        tool = tool_map.get(tool_call.name)
        if tool is not None and tool.is_passive:
            return True
    return False


async def generate(
    model: str | None = None,
    *,
    prompt: str | None = None,
    messages: Iterable[Message] | None = None,
    system: str | None = None,
    tools: Iterable[Tool] | None = None,
    tool_choice: ToolChoice | None = None,
    max_tool_rounds: int = 1,
    stop_when: Callable[[Sequence[StepResult]], bool] | None = None,
    response_format: ResponseFormat | None = None,
    temperature: float | None = None,
    top_p: float | None = None,
    max_tokens: int | None = None,
    stop_sequences: Iterable[str] | None = None,
    reasoning_effort: str | None = None,
    provider: str | None = None,
    provider_options: Mapping[str, Any] | None = None,
    max_retries: int = 2,
    timeout: float | TimeoutConfig | None = None,
    abort_signal: AbortSignal | None = None,
    repair_tool_call: Callable[..., Any] | None = None,
    client: Client | None = None,
) -> GenerateResult:
    config = _prepare_generation_config(
        model,
        prompt=prompt,
        messages=messages,
        system=system,
        tools=tools,
        tool_choice=tool_choice,
        max_tool_rounds=max_tool_rounds,
        stop_when=stop_when,
        response_format=response_format,
        temperature=temperature,
        top_p=top_p,
        max_tokens=max_tokens,
        stop_sequences=stop_sequences,
        reasoning_effort=reasoning_effort,
        provider=provider,
        provider_options=provider_options,
        max_retries=max_retries,
        timeout=timeout,
        abort_signal=abort_signal,
        repair_tool_call=repair_tool_call,
        client=client,
    )

    steps: list[StepResult] = []
    tool_rounds_executed = 0
    conversation = list(config.conversation)

    while True:
        current_request = replace(config.base_request, messages=list(conversation))
        response = await _complete_request_with_retries(
            config.client,
            current_request,
            timeout_config=config.timeout_config,
            total_deadline=config.total_deadline,
            abort_signal=config.abort_signal,
            max_retries=config.max_retries,
        )
        tool_calls = list(response.tool_calls)
        has_passive_tool_call = _has_passive_tool_call(config.tools, tool_calls)
        should_execute_tools = (
            bool(tool_calls)
            and response.finish_reason.reason == _TOOL_CALLS_REASON
            and tool_rounds_executed < config.max_tool_rounds
            and not has_passive_tool_call
        )

        tool_results: list[ToolResult] = []
        if should_execute_tools:
            check_abort(config.abort_signal)
            tool_results = await _execute_tool_calls(
                config.tools,
                tool_calls,
                messages=conversation,
                abort_signal=config.abort_signal,
                total_deadline=config.total_deadline,
                repair_tool_call=config.repair_tool_call,
            )

        step = StepResult(
            text=response.text,
            reasoning=response.reasoning,
            tool_calls=tool_calls,
            tool_results=tool_results,
            finish_reason=response.finish_reason,
            usage=response.usage,
            response=response,
            warnings=list(response.warnings),
        )
        steps.append(step)

        if (
            not tool_calls
            or response.finish_reason.reason != _TOOL_CALLS_REASON
            or has_passive_tool_call
            or tool_rounds_executed >= config.max_tool_rounds
        ):
            break
        if config.stop_when is not None and config.stop_when(steps):
            break

        tool_result_messages = [tool_result.to_message() for tool_result in tool_results]
        conversation = [*conversation, response.message, *tool_result_messages]
        tool_rounds_executed += 1

    final_step = steps[-1]
    return GenerateResult(
        text=final_step.text,
        reasoning=final_step.reasoning,
        tool_calls=list(final_step.tool_calls),
        tool_results=list(final_step.tool_results),
        finish_reason=final_step.finish_reason,
        usage=final_step.usage,
        total_usage=_sum_usage(step.usage for step in steps),
        steps=list(steps),
        response=final_step.response,
        output=None,
    )


def stream(
    model: str | None = None,
    *,
    prompt: str | None = None,
    messages: Iterable[Message] | None = None,
    system: str | None = None,
    tools: Iterable[Tool] | None = None,
    tool_choice: ToolChoice | None = None,
    max_tool_rounds: int = 1,
    stop_when: Callable[[Sequence[StepResult]], bool] | None = None,
    response_format: ResponseFormat | None = None,
    temperature: float | None = None,
    top_p: float | None = None,
    max_tokens: int | None = None,
    stop_sequences: Iterable[str] | None = None,
    reasoning_effort: str | None = None,
    provider: str | None = None,
    provider_options: Mapping[str, Any] | None = None,
    max_retries: int = 2,
    timeout: float | TimeoutConfig | None = None,
    abort_signal: AbortSignal | None = None,
    repair_tool_call: Callable[..., Any] | None = None,
    client: Client | None = None,
) -> StreamResult:
    config = _prepare_generation_config(
        model,
        prompt=prompt,
        messages=messages,
        system=system,
        tools=tools,
        tool_choice=tool_choice,
        max_tool_rounds=max_tool_rounds,
        stop_when=stop_when,
        response_format=response_format,
        temperature=temperature,
        top_p=top_p,
        max_tokens=max_tokens,
        stop_sequences=stop_sequences,
        reasoning_effort=reasoning_effort,
        provider=provider,
        provider_options=provider_options,
        max_retries=max_retries,
        timeout=timeout,
        abort_signal=abort_signal,
        repair_tool_call=repair_tool_call,
        client=client,
    )
    return StreamResult(config=config)


__all__ = ["GenerateResult", "StepResult", "StreamResult", "generate", "stream"]
