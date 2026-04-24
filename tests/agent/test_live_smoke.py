from __future__ import annotations

import asyncio
import json
import os
import shlex
import subprocess
import sys
from collections.abc import Callable, Iterable, Mapping
from dataclasses import dataclass
from pathlib import Path
from typing import Any

import httpx
import pytest

import agent
import unified_llm

pytestmark = pytest.mark.live_smoke


@dataclass(frozen=True)
class ProviderCase:
    name: str
    family: str
    model: str
    factory: Callable[..., agent.ProviderProfile]


PROVIDER_CASES = (
    ProviderCase(
        name="openai",
        family="openai",
        model="gpt-5.2",
        factory=agent.create_openai_profile,
    ),
    ProviderCase(
        name="anthropic",
        family="anthropic",
        model="claude-sonnet-4-5",
        factory=agent.create_anthropic_profile,
    ),
    ProviderCase(
        name="gemini",
        family="gemini",
        model="gemini-3.1-pro-preview",
        factory=agent.create_gemini_profile,
    ),
)


class _QueuedCompleteClient:
    def __init__(self, responses: list[unified_llm.Response]) -> None:
        self.requests: list[unified_llm.Request] = []
        self._responses = list(responses)

    async def complete(self, request: unified_llm.Request) -> unified_llm.Response:
        self.requests.append(request)
        if not self._responses:
            raise AssertionError("unexpected complete call")
        return self._responses.pop(0)

    def stream(self, request: unified_llm.Request):
        raise AssertionError("live smoke clients must not stream")


class _BlockingCompleteClient:
    def __init__(
        self,
        responses: list[unified_llm.Response],
        *,
        errors: list[BaseException | None] | None = None,
    ) -> None:
        self.requests: list[unified_llm.Request] = []
        self.responses = list(responses)
        self.errors = list(errors or [None] * len(responses))
        self.started: list[asyncio.Event] = [asyncio.Event() for _ in responses]
        self.released: list[asyncio.Event] = [asyncio.Event() for _ in responses]

    async def complete(self, request: unified_llm.Request) -> unified_llm.Response:
        index = len(self.requests)
        if index >= len(self.responses):
            raise AssertionError("unexpected complete call")
        self.requests.append(request)
        self.started[index].set()
        await self.released[index].wait()
        error = self.errors[index]
        if error is not None:
            raise error
        return self.responses[index]

    def stream(self, request: unified_llm.Request):
        raise AssertionError("live smoke clients must not stream")


def _event_kind_names(events: Iterable[agent.SessionEvent]) -> list[agent.EventKind | str]:
    return [event.kind for event in events]


async def _next_event(stream) -> agent.SessionEvent:
    return await asyncio.wait_for(anext(stream), timeout=2)


async def _collect_events_until(
    stream,
    final_kind: agent.EventKind,
) -> list[agent.SessionEvent]:
    events: list[agent.SessionEvent] = []
    while True:
        event = await _next_event(stream)
        events.append(event)
        if event.kind == final_kind:
            return events


async def _collect_events(stream, count: int) -> list[agent.SessionEvent]:
    return [await _next_event(stream) for _ in range(count)]


def _env_value(*names: str) -> str | None:
    for name in names:
        value = os.environ.get(name)
        if value:
            return value
    return None


def _base_url_for_case(case: ProviderCase) -> str:
    if case.family == "openai":
        return _env_value("OPENAI_BASE_URL") or "https://api.openai.com/v1"
    if case.family == "anthropic":
        return _env_value("ANTHROPIC_BASE_URL") or "https://api.anthropic.com/v1"
    if case.family == "gemini":
        return _env_value("GEMINI_BASE_URL") or "https://generativelanguage.googleapis.com/v1beta/openai/"
    raise AssertionError(f"unsupported provider family: {case.family}")


def _api_key_for_case(case: ProviderCase) -> str:
    if case.family == "openai":
        key = _env_value("OPENAI_API_KEY")
    elif case.family == "anthropic":
        key = _env_value("ANTHROPIC_API_KEY")
    elif case.family == "gemini":
        key = _env_value("GEMINI_API_KEY", "GOOGLE_API_KEY")
    else:
        raise AssertionError(f"unsupported provider family: {case.family}")
    if key is None:
        raise AssertionError(f"missing API key for {case.name}")
    return key


def _make_workspace(tmp_path: Path, name: str) -> Path:
    workspace = tmp_path / name
    workspace.mkdir(parents=True, exist_ok=True)
    return workspace


def _make_profile(case: ProviderCase) -> agent.ProviderProfile:
    return case.factory(model=case.model)


def _tool_name_set(tools: Iterable[Any] | None) -> set[str]:
    names: set[str] = set()
    for tool in tools or []:
        name = getattr(tool, "name", None)
        if isinstance(name, str):
            names.add(name)
    return names


def _tool_choice_for_openai(tool_name: str | None) -> str | dict[str, Any]:
    if tool_name is None:
        return "none"
    return {"type": "function", "function": {"name": tool_name}}


def _tool_choice_for_anthropic(tool_name: str | None) -> dict[str, Any]:
    if tool_name is None:
        return {"type": "none"}
    return {"type": "tool", "name": tool_name}


def _json_content(value: str | dict[str, Any] | list[Any]) -> str:
    if isinstance(value, str):
        return value
    return json.dumps(value, ensure_ascii=False, separators=(",", ":"))


def _message_role(message: Any) -> str:
    role = getattr(message, "role", None)
    return getattr(role, "value", str(role))


def _message_parts(message: Any) -> list[Any]:
    parts = getattr(message, "content", [])
    return list(parts or [])


def _message_text(message: Any) -> str:
    text = getattr(message, "text", "")
    if isinstance(text, str):
        return text
    return str(text)


def _kind_name(part: Any) -> str:
    kind = getattr(part, "kind", None)
    return getattr(kind, "value", str(kind))


def _assistant_tool_calls(message: Any) -> list[Any]:
    tool_calls: list[Any] = []
    for part in _message_parts(message):
        if _kind_name(part) == "tool_call":
            tool_call = getattr(part, "tool_call", None)
            if tool_call is not None:
                tool_calls.append(tool_call)
    return tool_calls


def _last_assistant_tool_call_name(messages: Iterable[Any]) -> str | None:
    for message in reversed(list(messages)):
        if _message_role(message) != "assistant":
            continue
        tool_calls = _assistant_tool_calls(message)
        if tool_calls:
            return getattr(tool_calls[-1], "name", None)
        return None
    return None


def _tail_user_messages(messages: Iterable[Any]) -> list[Any]:
    tail: list[Any] = []
    for message in reversed(list(messages)):
        if _message_role(message) != "user":
            break
        tail.append(message)
    tail.reverse()
    return tail


def _preferred_edit_tool(tools: Iterable[Any] | None) -> str:
    names = _tool_name_set(tools)
    if "apply_patch" in names:
        return "apply_patch"
    if "edit_file" in names:
        return "edit_file"
    raise AssertionError("no edit tool is available")


def _planned_tool_name(request: unified_llm.Request) -> str | None:
    if not request.messages:
        return None

    last_role = _message_role(request.messages[-1])
    if last_role == "tool":
        previous_tool_name = _last_assistant_tool_call_name(request.messages)
        if previous_tool_name == "read_file":
            return _preferred_edit_tool(request.tools)
        return None

    if last_role != "user":
        return None

    current_user_messages = _tail_user_messages(request.messages)
    if not current_user_messages:
        return None

    prompt = _message_text(current_user_messages[0]).casefold()
    if "hello.py" in prompt and any(
        keyword in prompt for keyword in ("create", "write")
    ):
        return "write_file"
    if "hello.py" in prompt and any(
        keyword in prompt for keyword in ("read", "edit", "update", "modify", "goodbye")
    ):
        return "read_file"
    return None


def _tool_call_to_openai(tool_call: Any) -> dict[str, Any]:
    arguments = getattr(tool_call, "arguments", {})
    if isinstance(arguments, str):
        arguments_text = arguments
    else:
        arguments_text = json.dumps(arguments, ensure_ascii=False, separators=(",", ":"))
    return {
        "id": getattr(tool_call, "id"),
        "type": "function",
        "function": {
            "name": getattr(tool_call, "name"),
            "arguments": arguments_text,
        },
    }


def _tool_call_to_anthropic(tool_call: Any) -> dict[str, Any]:
    arguments = getattr(tool_call, "arguments", {})
    if isinstance(arguments, str):
        try:
            arguments = json.loads(arguments)
        except json.JSONDecodeError:
            pass
    return {
        "type": "tool_use",
        "id": getattr(tool_call, "id"),
        "name": getattr(tool_call, "name"),
        "input": arguments,
    }


def _serialize_openai_message(message: Any) -> dict[str, Any]:
    role = _message_role(message)
    if role == "tool":
        tool_result = next(
            (getattr(part, "tool_result", None) for part in _message_parts(message)),
            None,
        )
        if tool_result is None:
            raise AssertionError("tool message is missing a tool_result part")
        return {
            "role": "tool",
            "tool_call_id": getattr(message, "tool_call_id", None),
            "content": _json_content(getattr(tool_result, "content", "")),
        }

    if role == "assistant":
        payload: dict[str, Any] = {"role": "assistant", "content": _message_text(message) or None}
        tool_calls = _assistant_tool_calls(message)
        if tool_calls:
            payload["tool_calls"] = [_tool_call_to_openai(tool_call) for tool_call in tool_calls]
        return payload

    return {"role": role, "content": _message_text(message)}


def _serialize_anthropic_messages(messages: Iterable[Any]) -> tuple[str, list[dict[str, Any]]]:
    system_parts: list[str] = []
    serialized_messages: list[dict[str, Any]] = []
    pending_tool_results: list[dict[str, Any]] = []

    def _flush_tool_results() -> None:
        nonlocal pending_tool_results
        if pending_tool_results:
            serialized_messages.append(
                {
                    "role": "user",
                    "content": list(pending_tool_results),
                }
            )
            pending_tool_results = []

    for message in messages:
        role = _message_role(message)
        if role == "system":
            text = _message_text(message)
            if text:
                system_parts.append(text)
            continue

        if role == "tool":
            tool_result = next(
                (getattr(part, "tool_result", None) for part in _message_parts(message)),
                None,
            )
            if tool_result is None:
                raise AssertionError("tool message is missing a tool_result part")
            pending_tool_results.append(
                {
                    "type": "tool_result",
                    "tool_use_id": getattr(message, "tool_call_id", None),
                    "content": _json_content(getattr(tool_result, "content", "")),
                    "is_error": bool(getattr(tool_result, "is_error", False)),
                }
            )
            continue

        _flush_tool_results()
        if role == "assistant":
            content: list[dict[str, Any]] = []
            for part in _message_parts(message):
                kind = _kind_name(part)
                if kind == "text" and getattr(part, "text", None) is not None:
                    content.append({"type": "text", "text": getattr(part, "text")})
                elif kind == "tool_call":
                    tool_call = getattr(part, "tool_call", None)
                    if tool_call is not None:
                        content.append(_tool_call_to_anthropic(tool_call))
            serialized_messages.append({"role": "assistant", "content": content})
            continue

        serialized_messages.append({"role": role, "content": _message_text(message)})

    _flush_tool_results()
    return "\n".join(system_parts), serialized_messages


def _response_usage_from_openai(data: Mapping[str, Any]) -> unified_llm.Usage:
    usage = data.get("usage")
    if not isinstance(usage, Mapping):
        return unified_llm.Usage()
    prompt_tokens = usage.get("prompt_tokens")
    completion_tokens = usage.get("completion_tokens")
    total_tokens = usage.get("total_tokens")
    return unified_llm.Usage(
        input_tokens=int(prompt_tokens or 0),
        output_tokens=int(completion_tokens or 0),
        total_tokens=int(total_tokens or (prompt_tokens or 0) + (completion_tokens or 0)),
        raw=dict(usage),
    )


def _response_usage_from_anthropic(data: Mapping[str, Any]) -> unified_llm.Usage:
    usage = data.get("usage")
    if not isinstance(usage, Mapping):
        return unified_llm.Usage()
    input_tokens = usage.get("input_tokens")
    output_tokens = usage.get("output_tokens")
    return unified_llm.Usage(
        input_tokens=int(input_tokens or 0),
        output_tokens=int(output_tokens or 0),
        total_tokens=int((input_tokens or 0) + (output_tokens or 0)),
        raw=dict(usage),
    )


def _response_from_openai(data: Mapping[str, Any], *, provider: str) -> unified_llm.Response:
    choices = data.get("choices")
    if not isinstance(choices, list) or not choices:
        raise AssertionError("openai-compatible response is missing choices")
    choice = choices[0]
    if not isinstance(choice, Mapping):
        raise AssertionError("openai-compatible response choice is invalid")
    message = choice.get("message")
    if not isinstance(message, Mapping):
        raise AssertionError("openai-compatible response message is invalid")

    content = message.get("content")
    if isinstance(content, list):
        text = "".join(
            str(part.get("text", ""))
            for part in content
            if isinstance(part, Mapping) and part.get("type") == "text"
        )
    elif content is None:
        text = ""
    else:
        text = str(content)

    parts: list[unified_llm.ContentPart] = []
    if text:
        parts.append(unified_llm.ContentPart(kind="text", text=text))
    for tool_call in message.get("tool_calls") or []:
        if not isinstance(tool_call, Mapping):
            continue
        function = tool_call.get("function")
        if not isinstance(function, Mapping):
            continue
        arguments = function.get("arguments", {})
        if isinstance(arguments, str):
            try:
                arguments = json.loads(arguments)
            except json.JSONDecodeError:
                pass
        parts.append(
            unified_llm.ContentPart(
                kind="tool_call",
                tool_call=unified_llm.ToolCallData(
                    id=str(tool_call.get("id", "")),
                    name=str(function.get("name", "")),
                    arguments=arguments if isinstance(arguments, (dict, str)) else {},
                ),
            )
        )

    response_message = unified_llm.Message.assistant(parts or "")
    return unified_llm.Response(
        id=str(data.get("id") or ""),
        model=str(data.get("model") or ""),
        provider=provider,
        message=response_message,
        finish_reason=str(choice.get("finish_reason") or "stop"),
        usage=_response_usage_from_openai(data),
        raw=dict(data),
    )


def _response_from_anthropic(data: Mapping[str, Any], *, provider: str) -> unified_llm.Response:
    content = data.get("content")
    if not isinstance(content, list):
        raise AssertionError("anthropic response is missing content blocks")

    parts: list[unified_llm.ContentPart] = []
    for block in content:
        if not isinstance(block, Mapping):
            continue
        block_type = block.get("type")
        if block_type == "text":
            parts.append(
                unified_llm.ContentPart(kind="text", text=str(block.get("text", "")))
            )
        elif block_type == "tool_use":
            arguments = block.get("input", {})
            if not isinstance(arguments, (dict, str)):
                arguments = {}
            parts.append(
                unified_llm.ContentPart(
                    kind="tool_call",
                    tool_call=unified_llm.ToolCallData(
                        id=str(block.get("id", "")),
                        name=str(block.get("name", "")),
                        arguments=arguments,
                    ),
                )
            )

    response_message = unified_llm.Message.assistant(parts or "")
    return unified_llm.Response(
        id=str(data.get("id") or ""),
        model=str(data.get("model") or ""),
        provider=provider,
        message=response_message,
        finish_reason=str(data.get("stop_reason") or "end_turn"),
        usage=_response_usage_from_anthropic(data),
        raw=dict(data),
    )


class _OpenAICompatibleLiveClient:
    def __init__(self, *, api_key: str, base_url: str, provider: str) -> None:
        self.provider = provider
        self.requests: list[unified_llm.Request] = []
        self._client = httpx.AsyncClient(
            base_url=base_url,
            headers={"Authorization": f"Bearer {api_key}"},
            timeout=httpx.Timeout(60.0),
        )

    async def aclose(self) -> None:
        await self._client.aclose()

    def stream(self, request: unified_llm.Request):
        raise AssertionError("live smoke clients must not stream")

    async def complete(self, request: unified_llm.Request) -> unified_llm.Response:
        self.requests.append(request)
        tool_name = _planned_tool_name(request)
        body: dict[str, Any] = {
            "model": request.model,
            "messages": [_serialize_openai_message(message) for message in request.messages],
            "tools": [
                {
                    "type": "function",
                    "function": {
                        "name": getattr(tool, "name", ""),
                        "description": getattr(tool, "description", ""),
                        "parameters": dict(getattr(tool, "parameters", {}) or {}),
                    },
                }
                for tool in request.tools or []
            ],
            "tool_choice": _tool_choice_for_openai(tool_name),
            "temperature": 0,
            "max_completion_tokens": 256 if tool_name is not None else 32,
        }
        response = await self._client.post("chat/completions", json=body)
        if response.status_code >= 400:
            raise RuntimeError(
                f"{self.provider} API request failed: {response.status_code} {response.text}"
            )
        payload = response.json()
        if not isinstance(payload, Mapping):
            raise RuntimeError(f"{self.provider} API returned a non-object payload")
        return _response_from_openai(payload, provider=self.provider)


class _AnthropicLiveClient:
    def __init__(self, *, api_key: str, base_url: str) -> None:
        self.provider = "anthropic"
        self.requests: list[unified_llm.Request] = []
        self._client = httpx.AsyncClient(
            base_url=base_url,
            headers={
                "x-api-key": api_key,
                "anthropic-version": "2023-06-01",
            },
            timeout=httpx.Timeout(60.0),
        )

    async def aclose(self) -> None:
        await self._client.aclose()

    def stream(self, request: unified_llm.Request):
        raise AssertionError("live smoke clients must not stream")

    async def complete(self, request: unified_llm.Request) -> unified_llm.Response:
        self.requests.append(request)
        tool_name = _planned_tool_name(request)
        system_prompt, messages = _serialize_anthropic_messages(request.messages)
        body: dict[str, Any] = {
            "model": request.model,
            "system": system_prompt,
            "messages": messages,
            "tools": [
                {
                    "name": getattr(tool, "name", ""),
                    "description": getattr(tool, "description", ""),
                    "input_schema": dict(getattr(tool, "parameters", {}) or {}),
                }
                for tool in request.tools or []
            ],
            "tool_choice": _tool_choice_for_anthropic(tool_name),
            "temperature": 0,
            "max_tokens": 256 if tool_name is not None else 32,
        }
        response = await self._client.post("messages", json=body)
        if response.status_code >= 400:
            raise RuntimeError(
                f"anthropic API request failed: {response.status_code} {response.text}"
            )
        payload = response.json()
        if not isinstance(payload, Mapping):
            raise RuntimeError("anthropic API returned a non-object payload")
        return _response_from_anthropic(payload, provider=self.provider)


def _make_live_client(case: ProviderCase) -> _OpenAICompatibleLiveClient | _AnthropicLiveClient:
    api_key = _api_key_for_case(case)
    base_url = _base_url_for_case(case)
    if case.family == "anthropic":
        return _AnthropicLiveClient(api_key=api_key, base_url=base_url)
    return _OpenAICompatibleLiveClient(api_key=api_key, base_url=base_url, provider=case.name)


def _make_live_session(
    case: ProviderCase,
    workspace: Path,
) -> tuple[agent.Session, _OpenAICompatibleLiveClient | _AnthropicLiveClient]:
    environment = agent.LocalExecutionEnvironment(working_dir=workspace)
    environment.initialize()
    profile = _make_profile(case)
    client = _make_live_client(case)
    session = agent.Session(profile=profile, execution_env=environment, llm_client=client)
    return session, client


def _make_tool_session(case: ProviderCase, workspace: Path) -> agent.Session:
    environment = agent.LocalExecutionEnvironment(working_dir=workspace)
    environment.initialize()
    profile = _make_profile(case)
    return agent.Session(profile=profile, execution_env=environment)


def _make_subagent_session(
    case: ProviderCase,
    workspace: Path,
    client: _BlockingCompleteClient,
) -> agent.Session:
    environment = agent.LocalExecutionEnvironment(working_dir=workspace)
    environment.initialize()
    profile = _make_profile(case)
    return agent.Session(profile=profile, execution_env=environment, llm_client=client)


def _assistant_response(
    text: str,
    response_id: str,
    *,
    model: str,
    provider: str,
) -> unified_llm.Response:
    return unified_llm.Response(
        id=response_id,
        model=model,
        provider=provider,
        message=unified_llm.Message.assistant(text),
        finish_reason=unified_llm.FinishReason.STOP,
    )


def _python_sleep_command(seconds: int) -> str:
    code = f"import time; time.sleep({seconds})"
    if os.name == "nt":
        return subprocess.list2cmdline([sys.executable, "-c", code])
    return shlex.join([sys.executable, "-c", code])


def _python_run_file_command(path: str) -> str:
    if os.name == "nt":
        return subprocess.list2cmdline([sys.executable, path])
    return shlex.join([sys.executable, path])


@pytest.mark.parametrize(
    "provider_case",
    PROVIDER_CASES,
    ids=[case.name for case in PROVIDER_CASES],
)
@pytest.mark.asyncio
async def test_live_smoke_provider_creates_edits_and_respects_steering(
    tmp_path: Path,
    provider_case: ProviderCase,
) -> None:
    workspace = _make_workspace(tmp_path, f"{provider_case.name}-live")
    (workspace / "scope_sentinel.txt").write_text("stay put", encoding="utf-8")

    session, client = _make_live_session(provider_case, workspace)
    stream = session.events()
    start_event = await _next_event(stream)
    assert start_event.kind == agent.EventKind.SESSION_START

    create_prompt = (
        'Create hello.py with exactly one line: print("Hello"). '
        'Use the write_file tool. After the file exists, reply with exactly done.'
    )
    await session.process_input(create_prompt)
    create_events = await _collect_events_until(stream, agent.EventKind.PROCESSING_END)

    assert _event_kind_names(create_events) == [
        agent.EventKind.USER_INPUT,
        agent.EventKind.ASSISTANT_TEXT_START,
        agent.EventKind.ASSISTANT_TEXT_DELTA,
        agent.EventKind.ASSISTANT_TEXT_END,
        agent.EventKind.TOOL_CALL_START,
        agent.EventKind.TOOL_CALL_END,
        agent.EventKind.ASSISTANT_TEXT_START,
        agent.EventKind.ASSISTANT_TEXT_DELTA,
        agent.EventKind.ASSISTANT_TEXT_END,
        agent.EventKind.PROCESSING_END,
    ]
    assert create_events[0].data == {"content": create_prompt}
    assert create_events[4].data["tool_name"] == "write_file"
    assert create_events[5].data["tool_name"] == "write_file"
    assert create_events[8].data["text"] == "done"
    assert session.state == agent.SessionState.IDLE
    assert isinstance(session.history[0], agent.UserTurn)
    assert isinstance(session.history[1], agent.AssistantTurn)
    assert isinstance(session.history[2], agent.ToolResultsTurn)
    assert isinstance(session.history[3], agent.AssistantTurn)
    assert session.history[2].result_list[0].content["path"] == "hello.py"
    assert (workspace / "hello.py").read_text(encoding="utf-8").splitlines() == [
        'print("Hello")',
    ]

    steering_message = "Only modify hello.py; leave scope_sentinel.txt unchanged."
    session.steer(steering_message)
    edit_prompt = (
        'Read hello.py and update it so it contains exactly two lines: '
        'print("Hello") and print("Goodbye"). '
        'Use the file tools as needed. After the update, reply with exactly done.'
    )
    await session.process_input(edit_prompt)
    edit_events = await _collect_events_until(stream, agent.EventKind.PROCESSING_END)

    expected_edit_tool_name = (
        "apply_patch"
        if "apply_patch" in session.profile.tool_registry.names()
        else "edit_file"
    )
    assert _event_kind_names(edit_events) == [
        agent.EventKind.USER_INPUT,
        agent.EventKind.STEERING_INJECTED,
        agent.EventKind.ASSISTANT_TEXT_START,
        agent.EventKind.ASSISTANT_TEXT_DELTA,
        agent.EventKind.ASSISTANT_TEXT_END,
        agent.EventKind.TOOL_CALL_START,
        agent.EventKind.TOOL_CALL_END,
        agent.EventKind.ASSISTANT_TEXT_START,
        agent.EventKind.ASSISTANT_TEXT_DELTA,
        agent.EventKind.ASSISTANT_TEXT_END,
        agent.EventKind.TOOL_CALL_START,
        agent.EventKind.TOOL_CALL_END,
        agent.EventKind.ASSISTANT_TEXT_START,
        agent.EventKind.ASSISTANT_TEXT_DELTA,
        agent.EventKind.ASSISTANT_TEXT_END,
        agent.EventKind.PROCESSING_END,
    ]
    assert edit_events[0].data == {"content": edit_prompt}
    assert edit_events[1].data == {"content": steering_message}
    assert edit_events[5].data["tool_name"] == "read_file"
    assert edit_events[6].data["tool_name"] == "read_file"
    assert edit_events[10].data["tool_name"] == expected_edit_tool_name
    assert edit_events[11].data["tool_name"] == expected_edit_tool_name
    assert edit_events[14].data["text"] == "done"
    assert session.state == agent.SessionState.IDLE
    assert isinstance(session.history[4], agent.UserTurn)
    assert isinstance(session.history[5], agent.SteeringTurn)
    assert session.history[5].text == steering_message
    assert isinstance(session.history[6], agent.AssistantTurn)
    assert isinstance(session.history[7], agent.ToolResultsTurn)
    assert isinstance(session.history[8], agent.AssistantTurn)
    assert session.history[7].result_list[0].content["path"] == "hello.py"
    assert (workspace / "hello.py").read_text(encoding="utf-8").splitlines() == [
        'print("Hello")',
        'print("Goodbye")',
    ]
    assert (workspace / "scope_sentinel.txt").read_text(encoding="utf-8") == "stay put"

    await session.close()
    await client.aclose()


@pytest.mark.parametrize(
    "provider_case",
    PROVIDER_CASES,
    ids=[case.name for case in PROVIDER_CASES],
)
@pytest.mark.asyncio
async def test_live_smoke_provider_shell_truncation_and_timeout(
    tmp_path: Path,
    provider_case: ProviderCase,
) -> None:
    workspace = _make_workspace(tmp_path, f"{provider_case.name}-tools")
    environment = agent.LocalExecutionEnvironment(working_dir=workspace)
    environment.initialize()
    profile = _make_profile(provider_case)
    session = agent.Session(profile=profile, execution_env=environment)
    stream = session.events()
    start_event = await _next_event(stream)
    assert start_event.kind == agent.EventKind.SESSION_START

    big_text = "x" * 100000
    (workspace / "big.txt").write_text(big_text, encoding="utf-8")

    read_result = await agent.execute_tool_call(
        session,
        unified_llm.ToolCallData(
            id="read-big",
            name="read_file",
            arguments={"path": "big.txt"},
        ),
    )
    read_events = await _collect_events(stream, 2)
    assert _event_kind_names(read_events) == [
        agent.EventKind.TOOL_CALL_START,
        agent.EventKind.TOOL_CALL_END,
    ]
    read_end_event = read_events[1]
    raw_read_output = read_end_event.data["output"]
    assert isinstance(raw_read_output, unified_llm.ToolResult)
    assert read_result.is_error is False
    assert read_result.content == agent.truncate_tool_output(
        raw_read_output.content,
        "read_file",
        session.config,
    )
    assert len(raw_read_output.content) > len(read_result.content)
    assert "[WARNING: Tool output was truncated." in read_result.content

    (workspace / "hello.py").write_text(
        'print("Hello")\nprint("Goodbye")\n',
        encoding="utf-8",
    )
    shell_result = await agent.execute_tool_call(
        session,
        unified_llm.ToolCallData(
            id="shell-run",
            name="shell",
            arguments={"command": _python_run_file_command("hello.py")},
        ),
    )
    shell_events = await _collect_events(stream, 2)
    assert _event_kind_names(shell_events) == [
        agent.EventKind.TOOL_CALL_START,
        agent.EventKind.TOOL_CALL_END,
    ]
    shell_end_event = shell_events[1]
    assert shell_result.is_error is False
    assert shell_result.content["stdout"].splitlines() == ["Hello", "Goodbye"]
    assert isinstance(shell_end_event.data["output"], agent.ExecResult)
    assert shell_end_event.data["output"].stdout.splitlines() == ["Hello", "Goodbye"]

    session.config.max_command_timeout_ms = 500
    timeout_result = await agent.execute_tool_call(
        session,
        unified_llm.ToolCallData(
            id="shell-timeout",
            name="shell",
            arguments={"command": _python_sleep_command(30)},
        ),
    )
    timeout_events = await _collect_events(stream, 2)
    assert _event_kind_names(timeout_events) == [
        agent.EventKind.TOOL_CALL_START,
        agent.EventKind.TOOL_CALL_END,
    ]
    timeout_end_event = timeout_events[1]
    assert timeout_result.is_error is True
    assert timeout_result.content["timed_out"] is True
    assert isinstance(timeout_end_event.data["error"], agent.ExecResult)
    assert timeout_end_event.data["error"].timed_out is True

    await session.close()


@pytest.mark.parametrize(
    "provider_case",
    PROVIDER_CASES,
    ids=[case.name for case in PROVIDER_CASES],
)
@pytest.mark.asyncio
async def test_live_smoke_provider_subagent_events_are_emitted(
    tmp_path: Path,
    provider_case: ProviderCase,
) -> None:
    workspace = _make_workspace(tmp_path, f"{provider_case.name}-subagent")
    child_client = _BlockingCompleteClient(
        [
            _assistant_response(
                "child complete",
                "child-1",
                model=provider_case.model,
                provider=provider_case.name,
            ),
        ]
    )
    session = _make_subagent_session(provider_case, workspace, child_client)
    stream = session.events()
    start_event = await _next_event(stream)
    assert start_event.kind == agent.EventKind.SESSION_START

    spawn_result = await agent.execute_tool_call(
        session,
        unified_llm.ToolCallData(
            id="spawn-1",
            name="spawn_agent",
            arguments={"task": "Reply with the word done and stop."},
        ),
    )
    spawn_events = await _collect_events(stream, 2)
    assert _event_kind_names(spawn_events) == [
        agent.EventKind.TOOL_CALL_START,
        agent.EventKind.TOOL_CALL_END,
    ]
    assert spawn_result.is_error is False
    assert spawn_result.content["status"] == "running"
    agent_id = spawn_result.content["agent_id"]
    handle = next(iter(session.active_subagents.values()))
    assert handle.status == agent.SubAgentStatus.RUNNING
    child_session = handle.session
    assert child_session is not None
    await asyncio.wait_for(child_client.started[0].wait(), timeout=2)

    child_stream = child_session.events()
    child_start_event = await _next_event(child_stream)
    assert child_start_event.kind == agent.EventKind.SESSION_START
    assert handle.status == agent.SubAgentStatus.RUNNING

    child_client.released[0].set()

    wait_result = await agent.execute_tool_call(
        session,
        unified_llm.ToolCallData(
            id="wait-1",
            name="wait",
            arguments={"agent_id": agent_id},
        ),
    )
    wait_events = await _collect_events(stream, 2)
    assert _event_kind_names(wait_events) == [
        agent.EventKind.TOOL_CALL_START,
        agent.EventKind.TOOL_CALL_END,
    ]
    assert wait_result.is_error is False
    assert wait_result.content["status"] == "completed"
    assert wait_result.content["success"] is True
    assert wait_result.content["output"] == "child complete"
    assert handle.result is not None
    assert handle.result.status == agent.SubAgentStatus.COMPLETED

    child_events = await _collect_events_until(child_stream, agent.EventKind.PROCESSING_END)
    assert _event_kind_names(child_events) == [
        agent.EventKind.USER_INPUT,
        agent.EventKind.ASSISTANT_TEXT_START,
        agent.EventKind.ASSISTANT_TEXT_DELTA,
        agent.EventKind.ASSISTANT_TEXT_END,
        agent.EventKind.PROCESSING_END,
    ]
    close_result = await agent.execute_tool_call(
        session,
        unified_llm.ToolCallData(
            id="close-1",
            name="close_agent",
            arguments={"agent_id": agent_id},
        ),
    )
    close_events = await _collect_events(stream, 2)
    assert _event_kind_names(close_events) == [
        agent.EventKind.TOOL_CALL_START,
        agent.EventKind.TOOL_CALL_END,
    ]
    assert close_result.is_error is False
    assert close_result.content["status"] == "completed"

    child_end_event = await _next_event(child_stream)
    assert child_end_event.kind == agent.EventKind.SESSION_END
    assert handle.status == agent.SubAgentStatus.COMPLETED
    assert session.active_subagents == {}

    await session.close()


def test_selection_probe() -> None:
    assert True
