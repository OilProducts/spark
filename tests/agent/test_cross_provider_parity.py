from __future__ import annotations

import asyncio
import fnmatch
import re
from collections.abc import Callable
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

import pytest

import agent
import unified_llm
from agent.history import history_to_messages


async def _next_event(stream) -> agent.SessionEvent:
    return await asyncio.wait_for(anext(stream), timeout=1)


@dataclass(frozen=True)
class ProviderCase:
    name: str
    model: str
    factory: Callable[..., agent.ProviderProfile]
    write_file_path_key: str
    expected_tool_names: tuple[str, ...]
    expected_provider_options: dict[str, Any]


PROVIDER_CASES = (
    ProviderCase(
        name="openai",
        model="gpt-5.2",
        factory=agent.create_openai_profile,
        write_file_path_key="path",
        expected_tool_names=(
            "read_file",
            "apply_patch",
            "write_file",
            "shell",
            "grep",
            "glob",
            "spawn_agent",
            "send_input",
            "wait",
            "close_agent",
        ),
        expected_provider_options={
            "reasoning": {"effort": "medium"},
        },
    ),
    ProviderCase(
        name="anthropic",
        model="claude-sonnet-4-5",
        factory=agent.create_anthropic_profile,
        write_file_path_key="file_path",
        expected_tool_names=(
            "read_file",
            "write_file",
            "edit_file",
            "shell",
            "grep",
            "glob",
            "spawn_agent",
            "send_input",
            "wait",
            "close_agent",
        ),
        expected_provider_options={},
    ),
    ProviderCase(
        name="gemini",
        model="gemini-3.1-pro-preview",
        factory=agent.create_gemini_profile,
        write_file_path_key="file_path",
        expected_tool_names=(
            "read_file",
            "read_many_files",
            "write_file",
            "edit_file",
            "shell",
            "grep",
            "glob",
            "list_dir",
            "spawn_agent",
            "send_input",
            "wait",
            "close_agent",
        ),
        expected_provider_options={},
    ),
)

REASONING_EFFORT = "medium"


class _CompleteClient:
    def __init__(
        self,
        responses: list[unified_llm.Response],
        *,
        request_hook: Callable[[int, unified_llm.Request], None] | None = None,
    ) -> None:
        self.requests: list[unified_llm.Request] = []
        self._responses = list(responses)
        self._request_hook = request_hook

    async def complete(self, request: unified_llm.Request) -> unified_llm.Response:
        request_index = len(self.requests)
        self.requests.append(request)
        if self._request_hook is not None:
            self._request_hook(request_index, request)
        if not self._responses:
            raise AssertionError("unexpected complete call")
        return self._responses.pop(0)

    def stream(self, request: unified_llm.Request):
        raise AssertionError("complete-mode sessions must not call stream()")


class _StreamingClient:
    def __init__(
        self,
        stream_groups: list[list[unified_llm.StreamEvent]],
        *,
        request_hook: Callable[[int, unified_llm.Request], None] | None = None,
    ) -> None:
        self.requests: list[unified_llm.Request] = []
        self._stream_groups = [list(group) for group in stream_groups]
        self._request_hook = request_hook

    def stream(self, request: unified_llm.Request):
        request_index = len(self.requests)
        self.requests.append(request)
        if self._request_hook is not None:
            self._request_hook(request_index, request)
        if not self._stream_groups:
            raise AssertionError("unexpected stream call")
        events = self._stream_groups.pop(0)

        async def _events():
            for event in events:
                yield event

        return _events()

    async def complete(self, request: unified_llm.Request) -> unified_llm.Response:
        raise AssertionError("stream-mode sessions must not call complete()")


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


class _RecordingExecutionEnvironment(agent.LocalExecutionEnvironment):
    def __init__(
        self,
        working_dir: Path,
        *,
        exec_handler: Callable[..., agent.ExecResult] | None = None,
    ) -> None:
        super().__init__(working_dir=working_dir)
        self.exec_calls: list[dict[str, object | None]] = []
        self.grep_calls: list[dict[str, object]] = []
        self.glob_calls: list[dict[str, object]] = []
        self._exec_handler = exec_handler

    def _resolve_search_path(self, path: str | Path) -> Path:
        candidate = Path(path)
        if not candidate.is_absolute():
            candidate = self._resolved_working_directory / candidate
        return candidate.expanduser().resolve(strict=False)

    def _display_path(self, path: Path) -> str:
        try:
            return str(path.relative_to(self._resolved_working_directory))
        except ValueError:
            return str(path)

    def exec_command(
        self,
        command: str,
        timeout_ms: int | None = None,
        working_dir: str | Path | None = None,
        env_vars: dict[str, str] | None = None,
    ) -> agent.ExecResult:
        self.exec_calls.append(
            {
                "command": command,
                "timeout_ms": timeout_ms,
                "working_dir": working_dir,
                "env_vars": None if env_vars is None else dict(env_vars),
            }
        )
        if self._exec_handler is None:
            raise AssertionError("unexpected exec_command call")
        result = self._exec_handler(
            command,
            timeout_ms=timeout_ms,
            working_dir=working_dir,
            env_vars=env_vars,
        )
        if not isinstance(result, agent.ExecResult):
            raise TypeError("exec handler must return an ExecResult")
        return result

    def grep(self, pattern: str, path: str | Path, options: agent.GrepOptions) -> str:
        self.grep_calls.append(
            {
                "pattern": pattern,
                "path": path,
                "options": options,
            }
        )
        base = self._resolve_search_path(path)
        if not base.exists():
            raise FileNotFoundError(base)
        if base.is_file():
            candidates = [base]
        elif base.is_dir():
            candidates = sorted(
                (candidate for candidate in base.rglob("*") if candidate.is_file()),
                key=lambda candidate: self._display_path(candidate),
            )
        else:
            raise NotADirectoryError(base)

        flags = re.IGNORECASE if options.case_insensitive else 0
        try:
            compiled = re.compile(pattern, flags)
        except re.error as exc:
            raise ValueError(str(exc)) from exc

        matches: list[str] = []
        for file_path in candidates:
            display_path = self._display_path(file_path)
            if options.glob_filter and not fnmatch.fnmatch(display_path, options.glob_filter):
                continue
            text = file_path.read_text(encoding="utf-8", errors="surrogateescape")
            for line_number, line in enumerate(text.splitlines(), start=1):
                if compiled.search(line):
                    matches.append(f"{display_path}:{line_number}:{line}")
                    if len(matches) >= options.max_results:
                        return "\n".join(matches)
        return "\n".join(matches)

    def glob(self, pattern: str, path: str | Path) -> list[str]:
        self.glob_calls.append(
            {
                "pattern": pattern,
                "path": path,
            }
        )
        base = self._resolve_search_path(path)
        if not base.exists():
            raise FileNotFoundError(base)
        if not base.is_dir():
            raise NotADirectoryError(base)
        try:
            matches = [candidate for candidate in base.glob(pattern) if candidate.is_file()]
        except (NotImplementedError, ValueError) as exc:
            raise ValueError(str(exc)) from exc
        matches.sort(key=lambda candidate: self._display_path(candidate))
        return [self._display_path(candidate) for candidate in matches]


@dataclass(frozen=True)
class ToolCallSpec:
    id: str
    name: str
    arguments: dict[str, Any]
    model_result_content: str | dict[str, Any] | list[Any]
    event_result_content: str | dict[str, Any] | list[Any] | None = None
    is_error: bool = False

    def __post_init__(self) -> None:
        if self.event_result_content is None:
            object.__setattr__(self, "event_result_content", self.model_result_content)


@dataclass(frozen=True)
class AssistantTurnSpec:
    response_id: str
    text: str
    tool_calls: tuple[ToolCallSpec, ...] = field(default_factory=tuple)


def _noop_post_run(
    provider_case: ProviderCase,
    session: agent.Session,
    environment: _RecordingExecutionEnvironment,
    client: object,
    events: list[agent.SessionEvent],
) -> None:
    return None


@dataclass
class ScenarioSpec:
    name: str
    user_input: str
    assistant_turns: tuple[AssistantTurnSpec, ...]
    config: agent.SessionConfig
    initial_files: tuple[tuple[str, str], ...] = field(default_factory=tuple)
    expected_final_files: tuple[tuple[str, str], ...] = field(default_factory=tuple)
    exec_handler: Callable[..., agent.ExecResult] | None = None
    post_run: Callable[
        [
            ProviderCase,
            agent.Session,
            _RecordingExecutionEnvironment,
            object,
            list[agent.SessionEvent],
        ],
        None,
    ] = _noop_post_run


def _file_path_key(provider_case: ProviderCase) -> str:
    return provider_case.write_file_path_key


def _text_line(text: str) -> str:
    return f"{text}\n"


def _byte_count(content: str) -> int:
    return len(content.encode("utf-8", errors="surrogateescape"))


def _read_output(content: str) -> str:
    lines = content.splitlines()
    return "\n".join(
        f"{line_number:03d} | {line}"
        for line_number, line in enumerate(lines, start=1)
    )


def _write_file_result(path: str, content: str) -> dict[str, Any]:
    return {"path": path, "bytes_written": _byte_count(content)}


def _edit_file_result(path: str, content: str, *, replace_all: bool = False) -> dict[str, Any]:
    return {
        "path": path,
        "replacements": 1,
        "bytes_written": _byte_count(content),
        "replace_all": replace_all,
    }


def _shell_result(exec_result: agent.ExecResult) -> dict[str, Any]:
    return {
        "stdout": exec_result.stdout,
        "stderr": exec_result.stderr,
        "exit_code": exec_result.exit_code,
        "timed_out": exec_result.timed_out,
        "duration_ms": exec_result.duration_ms,
    }


def _apply_patch_update(file_path: str, old_line: str, new_line: str) -> str:
    return "\n".join(
        [
            "*** Begin Patch",
            f"*** Update File: {file_path}",
            "@@",
            f"-{old_line}",
            f"+{new_line}",
            "*** End Patch",
        ]
    )


def _apply_patch_result(file_path: str) -> list[dict[str, Any]]:
    return [{"operation": "update", "path": file_path, "hunks": 1}]


def _glob_result(paths: list[str]) -> list[str]:
    return list(paths)


def _grep_result(matches: list[tuple[str, int, str]]) -> dict[str, Any]:
    return {
        "matches": [
            {"path": path, "line_number": line_number, "line": line}
            for path, line_number, line in matches
        ]
    }


def _is_exec_probe(call: dict[str, object | None]) -> bool:
    command = call.get("command")
    return isinstance(command, str) and command.startswith("git rev-parse ")


def _relevant_exec_calls(
    environment: _RecordingExecutionEnvironment,
) -> list[dict[str, object | None]]:
    return [call for call in environment.exec_calls if not _is_exec_probe(call)]


def _expected_grep_result(
    provider_case: ProviderCase,
    matches: list[tuple[str, int, str]],
) -> str | dict[str, Any] | list[Any]:
    if provider_case.name == "anthropic":
        paths: list[str] = []
        seen: set[str] = set()
        for path, _, _ in matches:
            if path in seen:
                continue
            seen.add(path)
            paths.append(path)
        return paths
    return _grep_result(matches)


def _expected_grep_options(provider_case: ProviderCase) -> agent.GrepOptions:
    return agent.GrepOptions(
        glob_filter=None,
        case_insensitive=False,
        max_results=10000 if provider_case.name == "anthropic" else 100,
    )


def _turn(response_id: str, text: str, *tool_calls: ToolCallSpec) -> AssistantTurnSpec:
    return AssistantTurnSpec(response_id=response_id, text=text, tool_calls=tuple(tool_calls))


def _write_file_call(
    provider_case: ProviderCase,
    *,
    call_id: str,
    path: str,
    content: str,
) -> ToolCallSpec:
    return ToolCallSpec(
        id=call_id,
        name="write_file",
        arguments={
            _file_path_key(provider_case): path,
            "content": content,
        },
        model_result_content=_write_file_result(path, content),
    )


def _read_file_call(
    provider_case: ProviderCase,
    *,
    call_id: str,
    path: str,
    content: str,
) -> ToolCallSpec:
    return ToolCallSpec(
        id=call_id,
        name="read_file",
        arguments={
            _file_path_key(provider_case): path,
        },
        model_result_content=_read_output(content),
    )


def _edit_file_call(
    provider_case: ProviderCase,
    *,
    call_id: str,
    path: str,
    old_line: str,
    new_line: str,
    instruction: str,
) -> ToolCallSpec:
    if provider_case.name == "openai":
        return ToolCallSpec(
            id=call_id,
            name="apply_patch",
            arguments={
                "patch": _apply_patch_update(path, old_line, new_line),
            },
            model_result_content=_apply_patch_result(path),
        )

    arguments: dict[str, Any] = {
        _file_path_key(provider_case): path,
        "old_string": old_line,
        "new_string": new_line,
    }
    if provider_case.name == "gemini":
        arguments["instruction"] = instruction
    return ToolCallSpec(
        id=call_id,
        name="edit_file",
        arguments=arguments,
        model_result_content=_edit_file_result(path, _text_line(new_line)),
    )


def _shell_exec_handler(
    *,
    expected_command: str,
    expected_timeout_ms: int,
    result: agent.ExecResult,
) -> Callable[..., agent.ExecResult]:
    def _handler(
        command: str,
        timeout_ms: int | None = None,
        working_dir: str | Path | None = None,
        env_vars: dict[str, str] | None = None,
    ) -> agent.ExecResult:
        assert command == expected_command
        assert timeout_ms == expected_timeout_ms
        assert working_dir is None
        assert env_vars is None
        return result

    return _handler


def _shell_call(
    *,
    call_id: str,
    command: str,
    timeout_ms: int,
    exec_result: agent.ExecResult,
) -> ToolCallSpec:
    return ToolCallSpec(
        id=call_id,
        name="shell",
        arguments={
            "command": command,
            "timeout_ms": timeout_ms,
        },
        model_result_content=_shell_result(exec_result),
        is_error=bool(exec_result.exit_code != 0 or exec_result.timed_out),
    )


def _glob_call(
    *,
    call_id: str,
    pattern: str,
    path: str | None,
    result_paths: list[str],
) -> ToolCallSpec:
    arguments: dict[str, Any] = {
        "pattern": pattern,
    }
    if path is not None:
        arguments["path"] = path
    return ToolCallSpec(
        id=call_id,
        name="glob",
        arguments=arguments,
        model_result_content=_glob_result(result_paths),
    )


def _grep_call(
    *,
    call_id: str,
    pattern: str,
    path: str | None,
    matches: list[tuple[str, int, str]],
) -> ToolCallSpec:
    arguments: dict[str, Any] = {
        "pattern": pattern,
    }
    if path is not None:
        arguments["path"] = path
    return ToolCallSpec(
        id=call_id,
        name="grep",
        arguments=arguments,
        model_result_content=_grep_result(matches),
    )


def _assistant_message_parts(turn: AssistantTurnSpec) -> list[unified_llm.ContentPart]:
    parts: list[unified_llm.ContentPart] = []
    if turn.text:
        parts.append(
            unified_llm.ContentPart(
                kind=unified_llm.ContentKind.TEXT,
                text=turn.text,
            )
        )
    for tool_call in turn.tool_calls:
        parts.append(
            unified_llm.ContentPart(
                kind=unified_llm.ContentKind.TOOL_CALL,
                tool_call=unified_llm.ToolCallData(
                    id=tool_call.id,
                    name=tool_call.name,
                    arguments=tool_call.arguments,
                ),
            )
        )
    return parts


def _turn_finish_reason(turn: AssistantTurnSpec) -> str:
    return "tool_calls" if turn.tool_calls else "stop"


def _build_complete_response(
    provider_case: ProviderCase,
    turn: AssistantTurnSpec,
) -> unified_llm.Response:
    return unified_llm.Response(
        id=turn.response_id,
        model=provider_case.model,
        provider=provider_case.name,
        message=unified_llm.Message.assistant(_assistant_message_parts(turn)),
        finish_reason=_turn_finish_reason(turn),
    )


def _build_stream_turn(
    provider_case: ProviderCase,
    turn: AssistantTurnSpec,
) -> list[unified_llm.StreamEvent]:
    response = unified_llm.Response(
        id=turn.response_id,
        model=provider_case.model,
        provider=provider_case.name,
    )
    events: list[unified_llm.StreamEvent] = [
        unified_llm.StreamEvent(
            type=unified_llm.StreamEventType.STREAM_START,
            response=response,
        )
    ]
    if turn.text:
        events.append(
            unified_llm.StreamEvent(
                type=unified_llm.StreamEventType.TEXT_START,
                delta=turn.text,
            )
        )
        events.append(
            unified_llm.StreamEvent(
                type=unified_llm.StreamEventType.TEXT_END,
            )
        )
    for tool_call in turn.tool_calls:
        tool_call_data = unified_llm.ToolCallData(
            id=tool_call.id,
            name=tool_call.name,
            arguments=tool_call.arguments,
        )
        events.append(
            unified_llm.StreamEvent(
                type=unified_llm.StreamEventType.TOOL_CALL_START,
                tool_call=tool_call_data,
            )
        )
        events.append(
            unified_llm.StreamEvent(
                type=unified_llm.StreamEventType.TOOL_CALL_END,
                tool_call=tool_call_data,
            )
        )
    events.append(
        unified_llm.StreamEvent(
            type=unified_llm.StreamEventType.FINISH,
            finish_reason=_turn_finish_reason(turn),
            response=unified_llm.Response(
                id=turn.response_id,
                model=provider_case.model,
                provider=provider_case.name,
            ),
        )
    )
    return events


def _assert_provider_request_shape(
    request: unified_llm.Request,
    *,
    provider_case: ProviderCase,
    reasoning_effort: str | None = REASONING_EFFORT,
    provider_options: dict[str, Any] | None = None,
) -> None:
    assert request.provider == provider_case.name
    assert request.model == provider_case.model
    assert request.reasoning_effort == reasoning_effort
    expected_provider_options = (
        provider_case.expected_provider_options
        if provider_options is None
        else provider_options
    )
    assert request.provider_options == {provider_case.name: expected_provider_options}
    tool_names = [tool.name for tool in request.tools or []]
    assert tool_names == list(provider_case.expected_tool_names)
    assert request.tool_choice is not None
    assert request.tool_choice.mode == "auto"
    assert request.tool_choice.tool_name is None

    if provider_case.name == "openai":
        assert "apply_patch" in tool_names
        assert "edit_file" not in tool_names
        apply_patch_tool = next(tool for tool in request.tools or [] if tool.name == "apply_patch")
        assert apply_patch_tool.parameters["properties"]["patch"]["type"] == "string"
        assert apply_patch_tool.parameters["required"] == ["patch"]
    elif provider_case.name == "anthropic":
        assert "apply_patch" not in tool_names
        edit_file_tool = next(tool for tool in request.tools or [] if tool.name == "edit_file")
        properties = edit_file_tool.parameters["properties"]
        assert properties["file_path"]["type"] == "string"
        assert properties["old_string"]["type"] == "string"
        assert properties["new_string"]["type"] == "string"
        assert properties["replace_all"]["type"] == "boolean"
        assert edit_file_tool.parameters["required"] == [
            "file_path",
            "old_string",
            "new_string",
        ]
    elif provider_case.name == "gemini":
        assert "apply_patch" not in tool_names
        edit_file_tool = next(tool for tool in request.tools or [] if tool.name == "edit_file")
        properties = edit_file_tool.parameters["properties"]
        assert properties["file_path"]["type"] == "string"
        assert properties["instruction"]["type"] == "string"
        assert properties["old_string"]["type"] == "string"
        assert properties["new_string"]["type"] == "string"
        assert properties["allow_multiple"]["default"] is False
        assert edit_file_tool.parameters["required"] == [
            "file_path",
            "instruction",
            "old_string",
            "new_string",
        ]


def _assert_request_history_prefixes(
    requests: list[unified_llm.Request],
    history: list[object],
    prefix_lengths: tuple[int, ...],
) -> None:
    assert len(requests) == len(prefix_lengths)
    for request, prefix_length in zip(requests, prefix_lengths):
        expected_messages = history_to_messages(history[:prefix_length])
        actual_messages = request.messages[1:]
        assert len(actual_messages) == len(expected_messages)
        for actual_message, expected_message in zip(actual_messages, expected_messages):
            assert actual_message.role == expected_message.role
            assert actual_message.name == expected_message.name
            assert actual_message.tool_call_id == expected_message.tool_call_id
            assert actual_message.text == expected_message.text
            assert [part.kind for part in actual_message.content] == [
                part.kind for part in expected_message.content
            ]
            for actual_part, expected_part in zip(
                actual_message.content,
                expected_message.content,
            ):
                assert actual_part.kind == expected_part.kind
                assert actual_part.text == expected_part.text
                if expected_part.tool_call is not None:
                    assert actual_part.tool_call is not None
                    _assert_tool_call(actual_part.tool_call, expected_part.tool_call)
                else:
                    assert actual_part.tool_call is None
                if expected_part.tool_result is not None:
                    assert actual_part.tool_result is not None
                    assert actual_part.tool_result.tool_call_id == (
                        expected_part.tool_result.tool_call_id
                    )
                    assert actual_part.tool_result.content == expected_part.tool_result.content
                    assert actual_part.tool_result.is_error == expected_part.tool_result.is_error
                    assert (
                        actual_part.tool_result.image_data
                        == expected_part.tool_result.image_data
                    )
                    assert (
                        actual_part.tool_result.image_media_type
                        == expected_part.tool_result.image_media_type
                    )
                else:
                    assert actual_part.tool_result is None


def _assert_tool_call(
    tool_call: Any,
    spec: ToolCallSpec,
) -> None:
    assert tool_call.id == spec.id
    assert tool_call.name == spec.name
    assert tool_call.type == "function"
    assert tool_call.arguments == spec.arguments


def _assert_model_tool_result(
    tool_result: unified_llm.ToolResultData,
    spec: ToolCallSpec,
) -> None:
    assert tool_result.tool_call_id == spec.id
    assert tool_result.content == spec.model_result_content
    assert tool_result.is_error == spec.is_error


def _assert_event_tool_result(
    tool_result: Any,
    spec: ToolCallSpec,
) -> None:
    assert isinstance(tool_result, agent.ToolOutput)
    assert tool_result.content == spec.event_result_content
    assert tool_result.is_error == spec.is_error


def _assert_request_history(
    request: unified_llm.Request,
    *,
    user_input: str,
    processed_turns: tuple[AssistantTurnSpec, ...],
) -> None:
    assert request.messages[0].role == unified_llm.Role.SYSTEM
    assert request.messages[1].role == unified_llm.Role.USER
    assert request.messages[1].text == user_input
    assert len(request.messages[1].content) == 1
    assert request.messages[1].content[0].kind == unified_llm.ContentKind.TEXT
    assert request.messages[1].content[0].text == user_input

    index = 2
    for turn in processed_turns:
        assistant_message = request.messages[index]
        assert assistant_message.role == unified_llm.Role.ASSISTANT
        assert assistant_message.text == turn.text
        assert len(assistant_message.content) == 1 + len(turn.tool_calls)
        assert assistant_message.content[0].kind == unified_llm.ContentKind.TEXT
        assert assistant_message.content[0].text == turn.text
        for part, tool_call_spec in zip(assistant_message.content[1:], turn.tool_calls):
            assert part.kind == unified_llm.ContentKind.TOOL_CALL
            assert part.tool_call is not None
            _assert_tool_call(part.tool_call, tool_call_spec)
        index += 1

        for tool_call_spec in turn.tool_calls:
            tool_message = request.messages[index]
            assert tool_message.role == unified_llm.Role.TOOL
            assert tool_message.tool_call_id == tool_call_spec.id
            assert len(tool_message.content) == 1
            tool_part = tool_message.content[0]
            assert tool_part.kind == unified_llm.ContentKind.TOOL_RESULT
            assert tool_part.tool_result is not None
            _assert_model_tool_result(tool_part.tool_result, tool_call_spec)
            index += 1

    assert index == len(request.messages)


def _assert_session_history(
    session: agent.Session,
    *,
    user_input: str,
    turn_specs: tuple[AssistantTurnSpec, ...],
) -> None:
    expected_types = ["UserTurn"]
    for turn in turn_specs:
        expected_types.append("AssistantTurn")
        if turn.tool_calls:
            expected_types.append("ToolResultsTurn")
    assert [type(turn).__name__ for turn in session.history] == expected_types

    index = 0
    user_turn = session.history[index]
    assert isinstance(user_turn, agent.UserTurn)
    assert user_turn.text == user_input
    index += 1

    for turn in turn_specs:
        assistant_turn = session.history[index]
        assert isinstance(assistant_turn, agent.AssistantTurn)
        assert assistant_turn.text == turn.text
        assert assistant_turn.response_id == turn.response_id
        assert assistant_turn.finish_reason is not None
        assert assistant_turn.finish_reason.reason == _turn_finish_reason(turn)
        assert len(assistant_turn.tool_calls) == len(turn.tool_calls)
        for actual_tool_call, expected_tool_call in zip(
            assistant_turn.tool_calls,
            turn.tool_calls,
        ):
            _assert_tool_call(actual_tool_call, expected_tool_call)
        index += 1

        if not turn.tool_calls:
            continue

        tool_results_turn = session.history[index]
        assert isinstance(tool_results_turn, agent.ToolResultsTurn)
        assert len(tool_results_turn.result_list) == len(turn.tool_calls)
        for actual_result, expected_result in zip(
            tool_results_turn.result_list,
            turn.tool_calls,
        ):
            _assert_model_tool_result(actual_result, expected_result)
        index += 1

    assert index == len(session.history)


def _assert_events(
    events: list[agent.SessionEvent],
    *,
    user_input: str,
    turn_specs: tuple[AssistantTurnSpec, ...],
) -> None:
    index = 0
    assert events[index].kind == agent.EventKind.SESSION_START
    assert events[index].data == {"state": "idle"}
    index += 1

    assert events[index].kind == agent.EventKind.USER_INPUT
    assert events[index].data == {"content": user_input}
    index += 1

    for turn in turn_specs:
        assert events[index].kind == agent.EventKind.ASSISTANT_TEXT_START
        assert events[index].data == {"response_id": turn.response_id}
        index += 1

        assert events[index].kind == agent.EventKind.ASSISTANT_TEXT_DELTA
        assert events[index].data == {
            "response_id": turn.response_id,
            "delta": turn.text,
        }
        index += 1

        assert events[index].kind == agent.EventKind.ASSISTANT_TEXT_END
        assert events[index].data == {
            "text": turn.text,
            "reasoning": None,
        }
        index += 1

        for tool_call_spec in turn.tool_calls:
            assert events[index].kind == agent.EventKind.TOOL_CALL_START
            assert events[index].data == {
                "tool_call_id": tool_call_spec.id,
                "tool_name": tool_call_spec.name,
            }
            index += 1

            assert events[index].kind == agent.EventKind.TOOL_CALL_END
            payload = events[index].data
            assert payload["tool_call_id"] == tool_call_spec.id
            assert payload["tool_name"] == tool_call_spec.name
            value = payload["error"] if tool_call_spec.is_error else payload["output"]
            _assert_event_tool_result(value, tool_call_spec)
            index += 1

    assert events[index].kind == agent.EventKind.PROCESSING_END
    assert events[index].data == {"state": "idle"}
    index += 1
    assert index == len(events)


def _assert_file_tree(
    tmp_path: Path,
    environment: _RecordingExecutionEnvironment,
    expected_files: tuple[tuple[str, str], ...],
) -> None:
    actual_paths = sorted(
        str(path.relative_to(tmp_path))
        for path in tmp_path.rglob("*")
        if path.is_file()
    )
    expected_paths = sorted(path for path, _ in expected_files)
    assert actual_paths == expected_paths
    for path, expected_content in expected_files:
        assert environment.read_file(path) == expected_content


def _assert_single_exec_call(
    environment: _RecordingExecutionEnvironment,
    *,
    command: str,
    timeout_ms: int,
) -> None:
    assert _relevant_exec_calls(environment) == [
        {
            "command": command,
            "timeout_ms": timeout_ms,
            "working_dir": None,
            "env_vars": None,
        }
    ]


def _assert_search_calls(
    provider_case: ProviderCase,
    environment: _RecordingExecutionEnvironment,
) -> None:
    assert _relevant_exec_calls(environment) == []
    assert environment.glob_calls == [
        {
            "pattern": "*.txt",
            "path": "search",
        }
    ]
    assert environment.grep_calls == [
        {
            "pattern": "needle",
            "path": "search",
            "options": _expected_grep_options(provider_case),
        }
    ]


def _build_simple_file_creation_scenario(provider_case: ProviderCase) -> ScenarioSpec:
    content = _text_line("alpha")
    write_call = _write_file_call(
        provider_case,
        call_id="call-1",
        path="hello.txt",
        content=content,
    )
    return ScenarioSpec(
        name="simple-file-creation",
        user_input="Create hello.txt and confirm it was written.",
        assistant_turns=(
            _turn("resp-1", "Writing hello.txt.", write_call),
            _turn("resp-2", "Created hello.txt."),
        ),
        config=agent.SessionConfig(reasoning_effort=REASONING_EFFORT),
        expected_final_files=(("hello.txt", content),),
    )


def _build_read_then_edit_scenario(provider_case: ProviderCase) -> ScenarioSpec:
    initial_content = _text_line("alpha")
    updated_content = _text_line("beta")
    read_call = _read_file_call(
        provider_case,
        call_id="call-1",
        path="note.txt",
        content=initial_content,
    )
    edit_call = _edit_file_call(
        provider_case,
        call_id="call-2",
        path="note.txt",
        old_line="alpha",
        new_line="beta",
        instruction="Replace alpha with beta.",
    )
    return ScenarioSpec(
        name="read-then-edit",
        user_input="Read note.txt, then update it.",
        assistant_turns=(
            _turn("resp-1", "Reading note.txt.", read_call),
            _turn("resp-2", "Replacing alpha with beta.", edit_call),
            _turn("resp-3", "Updated note.txt."),
        ),
        config=agent.SessionConfig(reasoning_effort=REASONING_EFFORT),
        initial_files=(("note.txt", initial_content),),
        expected_final_files=(("note.txt", updated_content),),
    )


def _build_multi_file_edit_scenario(provider_case: ProviderCase) -> ScenarioSpec:
    first_initial = _text_line("apple")
    second_initial = _text_line("banana")
    first_updated = _text_line("APPLE")
    second_updated = _text_line("BANANA")
    first_edit = _edit_file_call(
        provider_case,
        call_id="call-1a",
        path="first.txt",
        old_line="apple",
        new_line="APPLE",
        instruction="Uppercase first.txt.",
    )
    second_edit = _edit_file_call(
        provider_case,
        call_id="call-1b",
        path="second.txt",
        old_line="banana",
        new_line="BANANA",
        instruction="Uppercase second.txt.",
    )
    return ScenarioSpec(
        name="multi-file-edit",
        user_input="Update both files.",
        assistant_turns=(
            _turn("resp-1", "Updating both files.", first_edit, second_edit),
            _turn("resp-2", "Both files are updated."),
        ),
        config=agent.SessionConfig(reasoning_effort=REASONING_EFFORT),
        initial_files=(
            ("first.txt", first_initial),
            ("second.txt", second_initial),
        ),
        expected_final_files=(
            ("first.txt", first_updated),
            ("second.txt", second_updated),
        ),
    )


def _build_shell_execution_scenario(provider_case: ProviderCase) -> ScenarioSpec:
    exec_result = agent.ExecResult(
        stdout="shell ok\n",
        stderr="",
        exit_code=0,
        timed_out=False,
        duration_ms=7,
    )
    command = "echo shell-ok"
    timeout_ms = 2500
    shell_call = _shell_call(
        call_id="call-1",
        command=command,
        timeout_ms=timeout_ms,
        exec_result=exec_result,
    )

    def _post_run(
        provider_case: ProviderCase,
        session: agent.Session,
        environment: _RecordingExecutionEnvironment,
        client: object,
        events: list[agent.SessionEvent],
    ) -> None:
        _assert_single_exec_call(
            environment,
            command=command,
            timeout_ms=timeout_ms,
        )

    return ScenarioSpec(
        name="shell-execution",
        user_input="Run the shell command and report the result.",
        assistant_turns=(
            _turn("resp-1", "Running the shell command.", shell_call),
            _turn("resp-2", "Shell command succeeded."),
        ),
        config=agent.SessionConfig(reasoning_effort=REASONING_EFFORT),
        exec_handler=_shell_exec_handler(
            expected_command=command,
            expected_timeout_ms=timeout_ms,
            result=exec_result,
        ),
        post_run=_post_run,
    )


def _build_shell_timeout_scenario(provider_case: ProviderCase) -> ScenarioSpec:
    exec_result = agent.ExecResult(
        stdout="partial\n",
        stderr="timed out\n",
        exit_code=124,
        timed_out=True,
        duration_ms=2000,
    )
    command = "sleep 60"
    timeout_ms = 5
    shell_call = _shell_call(
        call_id="call-1",
        command=command,
        timeout_ms=timeout_ms,
        exec_result=exec_result,
    )

    def _post_run(
        provider_case: ProviderCase,
        session: agent.Session,
        environment: _RecordingExecutionEnvironment,
        client: object,
        events: list[agent.SessionEvent],
    ) -> None:
        _assert_single_exec_call(
            environment,
            command=command,
            timeout_ms=timeout_ms,
        )

    return ScenarioSpec(
        name="shell-timeout",
        user_input="Run the command with a short timeout.",
        assistant_turns=(
            _turn("resp-1", "Running a command that times out.", shell_call),
            _turn("resp-2", "The timeout was handled."),
        ),
        config=agent.SessionConfig(reasoning_effort=REASONING_EFFORT),
        exec_handler=_shell_exec_handler(
            expected_command=command,
            expected_timeout_ms=timeout_ms,
            result=exec_result,
        ),
        post_run=_post_run,
    )


def _build_grep_plus_glob_scenario(provider_case: ProviderCase) -> ScenarioSpec:
    alpha_content = _text_line("needle alpha")
    beta_content = _text_line("needle beta")
    ignored_content = _text_line("skip me")
    matches = [
        ("search/alpha.txt", 1, "needle alpha"),
        ("search/beta.txt", 1, "needle beta"),
    ]
    initial_files = (
        ("search/alpha.txt", alpha_content),
        ("search/beta.txt", beta_content),
        ("search/ignore.md", ignored_content),
    )
    glob_call = _glob_call(
        call_id="call-1",
        pattern="*.txt",
        path="search",
        result_paths=[
            "search/alpha.txt",
            "search/beta.txt",
        ],
    )
    grep_call = _grep_call(
        call_id="call-2",
        pattern="needle",
        path="search",
        matches=matches,
    )
    grep_result = _expected_grep_result(provider_case, matches)
    grep_call = ToolCallSpec(
        id=grep_call.id,
        name=grep_call.name,
        arguments=grep_call.arguments,
        model_result_content=grep_result,
    )

    def _post_run(
        provider_case: ProviderCase,
        session: agent.Session,
        environment: _RecordingExecutionEnvironment,
        client: object,
        events: list[agent.SessionEvent],
    ) -> None:
        _assert_search_calls(provider_case, environment)

    return ScenarioSpec(
        name="grep-plus-glob",
        user_input="Find the txt files and matches.",
        assistant_turns=(
            _turn("resp-1", "Finding the relevant files.", glob_call, grep_call),
            _turn("resp-2", "The search is narrowed down."),
        ),
        config=agent.SessionConfig(reasoning_effort=REASONING_EFFORT),
        initial_files=initial_files,
        expected_final_files=initial_files,
        post_run=_post_run,
    )


def _build_multi_step_read_analyze_edit_scenario(
    provider_case: ProviderCase,
) -> ScenarioSpec:
    plan_content = _text_line("replace pending with done")
    draft_initial = _text_line("status: pending")
    draft_updated = _text_line("status: done")
    read_plan_call = _read_file_call(
        provider_case,
        call_id="call-1",
        path="plan.txt",
        content=plan_content,
    )
    edit_call = _edit_file_call(
        provider_case,
        call_id="call-2",
        path="draft.txt",
        old_line="status: pending",
        new_line="status: done",
        instruction="Update draft.txt based on the plan.",
    )
    read_draft_call = _read_file_call(
        provider_case,
        call_id="call-3",
        path="draft.txt",
        content=draft_updated,
    )
    return ScenarioSpec(
        name="multi-step-read-analyze-edit",
        user_input="Read the plan and apply it to the draft.",
        assistant_turns=(
            _turn("resp-1", "Reading the plan.", read_plan_call),
            _turn("resp-2", "Applying the plan to draft.txt.", edit_call),
            _turn("resp-3", "Checking the updated draft.", read_draft_call),
            _turn("resp-4", "Draft is updated."),
        ),
        config=agent.SessionConfig(reasoning_effort=REASONING_EFFORT),
        initial_files=(
            ("plan.txt", plan_content),
            ("draft.txt", draft_initial),
        ),
        expected_final_files=(
            ("plan.txt", plan_content),
            ("draft.txt", draft_updated),
        ),
    )


def _build_large_output_truncation_scenario(
    provider_case: ProviderCase,
) -> ScenarioSpec:
    long_line = "0123456789" * 30
    file_content = _text_line(long_line)
    full_output = _read_output(file_content)
    read_call = ToolCallSpec(
        id="call-1",
        name="read_file",
        arguments={
            _file_path_key(provider_case): "large.txt",
        },
        model_result_content=agent.truncate_output(full_output, 40, "head_tail"),
        event_result_content=full_output,
    )
    return ScenarioSpec(
        name="large-output-truncation",
        user_input="Read the large file.",
        assistant_turns=(
            _turn("resp-1", "Reading the large file.", read_call),
            _turn("resp-2", "The large output was truncated for the model."),
        ),
        config=agent.SessionConfig(
            reasoning_effort=REASONING_EFFORT,
            tool_output_limits={"read_file": 40},
        ),
        initial_files=(("large.txt", file_content),),
        expected_final_files=(("large.txt", file_content),),
    )


def _build_tool_error_recovery_scenario(provider_case: ProviderCase) -> ScenarioSpec:
    repaired_content = _text_line("recovered")
    missing_read_call = ToolCallSpec(
        id="call-1",
        name="read_file",
        arguments={
            _file_path_key(provider_case): "missing.txt",
        },
        model_result_content="File not found: missing.txt",
        is_error=True,
    )
    write_call = _write_file_call(
        provider_case,
        call_id="call-2",
        path="missing.txt",
        content=repaired_content,
    )
    return ScenarioSpec(
        name="tool-error-recovery",
        user_input="Read missing.txt and repair it.",
        assistant_turns=(
            _turn("resp-1", "Reading missing.txt.", missing_read_call),
            _turn("resp-2", "Creating missing.txt.", write_call),
            _turn("resp-3", "missing.txt has been repaired."),
        ),
        config=agent.SessionConfig(reasoning_effort=REASONING_EFFORT),
        expected_final_files=(("missing.txt", repaired_content),),
    )


SCENARIO_BUILDERS: tuple[tuple[str, Callable[[ProviderCase], ScenarioSpec]], ...] = (
    ("simple-file-creation", _build_simple_file_creation_scenario),
    ("read-then-edit", _build_read_then_edit_scenario),
    ("multi-file-edit", _build_multi_file_edit_scenario),
    ("shell-execution", _build_shell_execution_scenario),
    ("shell-timeout", _build_shell_timeout_scenario),
    ("grep-plus-glob", _build_grep_plus_glob_scenario),
    ("multi-step-read-analyze-edit", _build_multi_step_read_analyze_edit_scenario),
    ("large-output-truncation", _build_large_output_truncation_scenario),
    ("tool-error-recovery", _build_tool_error_recovery_scenario),
)


async def _run_parity_scenario(
    tmp_path: Path,
    provider_case: ProviderCase,
    client_mode: str,
    scenario: ScenarioSpec,
) -> None:
    environment = _RecordingExecutionEnvironment(
        working_dir=tmp_path,
        exec_handler=scenario.exec_handler,
    )
    for path, content in scenario.initial_files:
        environment.write_file(path, content)

    profile = provider_case.factory(
        model=provider_case.model,
        supports_streaming=client_mode == "stream",
    )
    if client_mode == "complete":
        client: _CompleteClient | _StreamingClient = _CompleteClient(
            [
                _build_complete_response(provider_case, turn)
                for turn in scenario.assistant_turns
            ]
        )
    else:
        client = _StreamingClient(
            [
                _build_stream_turn(provider_case, turn)
                for turn in scenario.assistant_turns
            ]
        )

    session = agent.Session(
        profile=profile,
        execution_env=environment,
        llm_client=client,
        config=scenario.config,
    )
    stream = session.events()

    start_event = await _next_event(stream)
    assert start_event.kind == agent.EventKind.SESSION_START
    assert start_event.data == {"state": "idle"}

    await session.process_input(scenario.user_input)

    events: list[agent.SessionEvent] = [start_event]
    while True:
        event = await _next_event(stream)
        events.append(event)
        if event.kind == agent.EventKind.PROCESSING_END:
            break

    assert session.state == agent.SessionState.IDLE
    if client_mode == "complete":
        assert isinstance(session.client, _CompleteClient)
    else:
        assert isinstance(session.client, _StreamingClient)

    assert len(client.requests) == len(scenario.assistant_turns)
    for index, request in enumerate(client.requests):
        _assert_provider_request_shape(request, provider_case=provider_case)
        _assert_request_history(
            request,
            user_input=scenario.user_input,
            processed_turns=scenario.assistant_turns[:index],
        )

    _assert_events(
        events,
        user_input=scenario.user_input,
        turn_specs=scenario.assistant_turns,
    )
    _assert_session_history(
        session,
        user_input=scenario.user_input,
        turn_specs=scenario.assistant_turns,
    )
    _assert_file_tree(tmp_path, environment, scenario.expected_final_files)
    scenario.post_run(provider_case, session, environment, client, events)


async def _run_parallel_tool_call_parity_case(
    tmp_path: Path,
    provider_case: ProviderCase,
    client_mode: str,
) -> None:
    environment = _RecordingExecutionEnvironment(working_dir=tmp_path)
    first_path = "parallel-first.txt"
    second_path = "parallel-second.txt"
    first_content = _text_line("first")
    second_content = _text_line("second")
    first_result = _write_file_result(first_path, first_content)
    second_result = _write_file_result(second_path, second_content)
    first_started = asyncio.Event()
    second_started = asyncio.Event()
    release_first = asyncio.Event()
    release_second = asyncio.Event()

    async def _parallel_write_file_executor(
        arguments: dict[str, object],
        execution_environment: object,
    ) -> dict[str, Any]:
        assert isinstance(execution_environment, agent.LocalExecutionEnvironment)
        path_key = _file_path_key(provider_case)
        assert path_key in arguments
        path = arguments[path_key]
        assert isinstance(path, str)
        content = arguments["content"]
        assert isinstance(content, str)

        if path == first_path:
            first_started.set()
            await release_first.wait()
            execution_environment.write_file(path, content)
            return first_result
        if path == second_path:
            second_started.set()
            await release_second.wait()
            execution_environment.write_file(path, content)
            return second_result
        raise AssertionError(f"unexpected parallel write_file path: {path}")

    profile = provider_case.factory(
        model=provider_case.model,
        supports_streaming=client_mode == "stream",
        supports_parallel_tool_calls=True,
    )
    write_file_tool = profile.tool_registry.get("write_file")
    assert write_file_tool is not None
    profile.tool_registry.register(write_file_tool, executor=_parallel_write_file_executor)

    first_turn = _turn(
        "resp-1",
        "Writing both files in parallel.",
        _write_file_call(
            provider_case,
            call_id="call-1",
            path=first_path,
            content=first_content,
        ),
        _write_file_call(
            provider_case,
            call_id="call-2",
            path=second_path,
            content=second_content,
        ),
    )
    second_turn = _turn("resp-2", "Both files were written.")
    if client_mode == "complete":
        client: _CompleteClient | _StreamingClient = _CompleteClient(
            [
                _build_complete_response(provider_case, first_turn),
                _build_complete_response(provider_case, second_turn),
            ]
        )
    else:
        client = _StreamingClient(
            [
                _build_stream_turn(provider_case, first_turn),
                _build_stream_turn(provider_case, second_turn),
            ]
        )

    session = agent.Session(
        profile=profile,
        execution_env=environment,
        llm_client=client,
        config=agent.SessionConfig(reasoning_effort=REASONING_EFFORT),
    )
    stream = session.events()

    start_event = await _next_event(stream)
    assert start_event.kind == agent.EventKind.SESSION_START
    assert start_event.data == {"state": "idle"}

    processing_task = asyncio.create_task(
        session.process_input("Write two files in parallel."),
    )

    events: list[agent.SessionEvent] = [start_event]
    tool_start_count = 0
    while tool_start_count < 2:
        event = await _next_event(stream)
        events.append(event)
        if event.kind == agent.EventKind.TOOL_CALL_START:
            tool_start_count += 1

    await asyncio.wait_for(
        asyncio.gather(first_started.wait(), second_started.wait()),
        timeout=1,
    )
    assert [event.kind for event in events] == [
        agent.EventKind.SESSION_START,
        agent.EventKind.USER_INPUT,
        agent.EventKind.ASSISTANT_TEXT_START,
        agent.EventKind.ASSISTANT_TEXT_DELTA,
        agent.EventKind.ASSISTANT_TEXT_END,
        agent.EventKind.TOOL_CALL_START,
        agent.EventKind.TOOL_CALL_START,
    ]
    assert [event.data["tool_call_id"] for event in events[-2:]] == [
        "call-1",
        "call-2",
    ]
    assert events[2].data == {"response_id": "resp-1"}
    assert events[3].data == {
        "response_id": "resp-1",
        "delta": "Writing both files in parallel.",
    }
    assert events[4].data == {
        "text": "Writing both files in parallel.",
        "reasoning": None,
    }

    release_second.set()
    second_end_event = await _next_event(stream)
    events.append(second_end_event)
    assert second_end_event.kind == agent.EventKind.TOOL_CALL_END
    assert second_end_event.data == {
        "tool_call_id": "call-2",
        "tool_name": "write_file",
        "output": second_result,
    }

    release_first.set()
    first_end_event = await _next_event(stream)
    events.append(first_end_event)
    assert first_end_event.kind == agent.EventKind.TOOL_CALL_END
    assert first_end_event.data == {
        "tool_call_id": "call-1",
        "tool_name": "write_file",
        "output": first_result,
    }

    while True:
        event = await _next_event(stream)
        events.append(event)
        if event.kind == agent.EventKind.PROCESSING_END:
            break

    await asyncio.wait_for(processing_task, timeout=1)

    assert session.state == agent.SessionState.IDLE
    assert [event.kind for event in events] == [
        agent.EventKind.SESSION_START,
        agent.EventKind.USER_INPUT,
        agent.EventKind.ASSISTANT_TEXT_START,
        agent.EventKind.ASSISTANT_TEXT_DELTA,
        agent.EventKind.ASSISTANT_TEXT_END,
        agent.EventKind.TOOL_CALL_START,
        agent.EventKind.TOOL_CALL_START,
        agent.EventKind.TOOL_CALL_END,
        agent.EventKind.TOOL_CALL_END,
        agent.EventKind.ASSISTANT_TEXT_START,
        agent.EventKind.ASSISTANT_TEXT_DELTA,
        agent.EventKind.ASSISTANT_TEXT_END,
        agent.EventKind.PROCESSING_END,
    ]
    assert events[9].data == {"response_id": "resp-2"}
    assert events[10].data == {
        "response_id": "resp-2",
        "delta": "Both files were written.",
    }
    assert events[11].data == {
        "text": "Both files were written.",
        "reasoning": None,
    }
    assert events[12].data == {"state": "idle"}
    assert len(client.requests) == 2
    for request in client.requests:
        _assert_provider_request_shape(request, provider_case=provider_case)
    _assert_request_history_prefixes(client.requests, session.history, (1, 3))
    _assert_session_history(
        session,
        user_input="Write two files in parallel.",
        turn_specs=(first_turn, second_turn),
    )
    _assert_file_tree(
        tmp_path,
        environment,
        (
            (first_path, first_content),
            (second_path, second_content),
        ),
    )


@pytest.mark.parametrize(
    "provider_case",
    PROVIDER_CASES,
    ids=[case.name for case in PROVIDER_CASES],
)
@pytest.mark.parametrize("client_mode", ["complete", "stream"], ids=["complete", "stream"])
@pytest.mark.parametrize(
    "scenario_name,scenario_builder",
    SCENARIO_BUILDERS,
    ids=[name for name, _ in SCENARIO_BUILDERS],
)
@pytest.mark.asyncio
async def test_cross_provider_parity_scenarios(
    tmp_path: Path,
    provider_case: ProviderCase,
    client_mode: str,
    scenario_name: str,
    scenario_builder: Callable[[ProviderCase], ScenarioSpec],
) -> None:
    scenario = scenario_builder(provider_case)
    assert scenario.name == scenario_name
    await _run_parity_scenario(tmp_path, provider_case, client_mode, scenario)


@pytest.mark.parametrize(
    "provider_case",
    PROVIDER_CASES,
    ids=[case.name for case in PROVIDER_CASES],
)
@pytest.mark.parametrize("client_mode", ["complete", "stream"], ids=["complete", "stream"])
@pytest.mark.asyncio
async def test_cross_provider_parity_parallel_tool_calls(
    tmp_path: Path,
    provider_case: ProviderCase,
    client_mode: str,
) -> None:
    await _run_parallel_tool_call_parity_case(tmp_path, provider_case, client_mode)


async def _collect_events_until_processing_end(
    stream,
) -> list[agent.SessionEvent]:
    events = [await _next_event(stream)]
    while True:
        event = await _next_event(stream)
        events.append(event)
        if event.kind == agent.EventKind.PROCESSING_END:
            return events


@pytest.mark.parametrize(
    "provider_case",
    PROVIDER_CASES,
    ids=[case.name for case in PROVIDER_CASES],
)
@pytest.mark.parametrize("client_mode", ["complete", "stream"], ids=["complete", "stream"])
@pytest.mark.asyncio
async def test_cross_provider_parity_steering_mid_task(
    tmp_path: Path,
    provider_case: ProviderCase,
    client_mode: str,
) -> None:
    environment = _RecordingExecutionEnvironment(working_dir=tmp_path)
    file_content = _text_line("alpha")
    steering_message = "Add one sentence about the change."
    write_call = _write_file_call(
        provider_case,
        call_id="call-1",
        path="hello.txt",
        content=file_content,
    )
    response_turns = (
        _turn("resp-1", "Writing hello.txt.", write_call),
        _turn("resp-2", "The steering note was applied."),
    )
    session_box: dict[str, agent.Session] = {}

    def _request_hook(request_index: int, request: unified_llm.Request) -> None:
        if request_index == 0:
            session_box["session"].steer(steering_message)

    profile = provider_case.factory(
        model=provider_case.model,
        supports_streaming=client_mode == "stream",
    )
    if client_mode == "complete":
        client: _CompleteClient | _StreamingClient = _CompleteClient(
            [_build_complete_response(provider_case, turn) for turn in response_turns],
            request_hook=_request_hook,
        )
    else:
        client = _StreamingClient(
            [_build_stream_turn(provider_case, turn) for turn in response_turns],
            request_hook=_request_hook,
        )

    session = agent.Session(
        profile=profile,
        execution_env=environment,
        llm_client=client,
        config=agent.SessionConfig(reasoning_effort=REASONING_EFFORT),
    )
    session_box["session"] = session
    stream = session.events()

    start_event = await _next_event(stream)
    assert start_event.kind == agent.EventKind.SESSION_START

    await session.process_input("Create hello.txt and steer mid-task.")

    events = await _collect_events_until_processing_end(stream)
    assert session.state == agent.SessionState.IDLE
    assert len(client.requests) == 2
    _assert_provider_request_shape(client.requests[0], provider_case=provider_case)
    _assert_provider_request_shape(client.requests[1], provider_case=provider_case)
    _assert_request_history_prefixes(client.requests, session.history, (1, 4))
    assert [event.kind for event in events] == [
        agent.EventKind.USER_INPUT,
        agent.EventKind.ASSISTANT_TEXT_START,
        agent.EventKind.ASSISTANT_TEXT_DELTA,
        agent.EventKind.ASSISTANT_TEXT_END,
        agent.EventKind.TOOL_CALL_START,
        agent.EventKind.TOOL_CALL_END,
        agent.EventKind.STEERING_INJECTED,
        agent.EventKind.ASSISTANT_TEXT_START,
        agent.EventKind.ASSISTANT_TEXT_DELTA,
        agent.EventKind.ASSISTANT_TEXT_END,
        agent.EventKind.PROCESSING_END,
    ]
    assert events[0].data == {"content": "Create hello.txt and steer mid-task."}
    assert events[6].data == {"content": steering_message}
    assert events[10].data == {"state": "idle"}
    assert isinstance(session.history[0], agent.UserTurn)
    assert session.history[0].text == "Create hello.txt and steer mid-task."
    assert isinstance(session.history[1], agent.AssistantTurn)
    assert session.history[1].text == "Writing hello.txt."
    assert isinstance(session.history[2], agent.ToolResultsTurn)
    assert session.history[2].result_list[0].content == _write_file_result(
        "hello.txt",
        file_content,
    )
    assert isinstance(session.history[3], agent.SteeringTurn)
    assert session.history[3].text == steering_message
    assert isinstance(session.history[4], agent.AssistantTurn)
    assert session.history[4].text == "The steering note was applied."
    _assert_event_tool_result(events[5].data["output"], write_call)
    _assert_file_tree(tmp_path, environment, (("hello.txt", file_content),))


@pytest.mark.parametrize(
    "provider_case",
    PROVIDER_CASES,
    ids=[case.name for case in PROVIDER_CASES],
)
@pytest.mark.parametrize("client_mode", ["complete", "stream"], ids=["complete", "stream"])
@pytest.mark.asyncio
async def test_cross_provider_parity_reasoning_effort_changes_mid_run(
    tmp_path: Path,
    provider_case: ProviderCase,
    client_mode: str,
) -> None:
    environment = _RecordingExecutionEnvironment(working_dir=tmp_path)
    file_content = _text_line("alpha")
    write_call = _write_file_call(
        provider_case,
        call_id="call-1",
        path="hello.txt",
        content=file_content,
    )
    session_box: dict[str, agent.Session] = {}

    def _request_hook(request_index: int, request: unified_llm.Request) -> None:
        if request_index == 0:
            session_box["session"].config.reasoning_effort = None

    profile = provider_case.factory(
        model=provider_case.model,
        supports_streaming=client_mode == "stream",
    )
    if client_mode == "complete":
        client: _CompleteClient | _StreamingClient = _CompleteClient(
            [
                _build_complete_response(
                    provider_case,
                    _turn("resp-1", "Writing hello.txt.", write_call),
                ),
                _build_complete_response(provider_case, _turn("resp-2", "Created hello.txt.")),
            ],
            request_hook=_request_hook,
        )
    else:
        client = _StreamingClient(
            [
                _build_stream_turn(
                    provider_case,
                    _turn("resp-1", "Writing hello.txt.", write_call),
                ),
                _build_stream_turn(provider_case, _turn("resp-2", "Created hello.txt.")),
            ],
            request_hook=_request_hook,
        )

    session = agent.Session(
        profile=profile,
        execution_env=environment,
        llm_client=client,
        config=agent.SessionConfig(reasoning_effort=REASONING_EFFORT),
    )
    session_box["session"] = session
    stream = session.events()

    start_event = await _next_event(stream)
    assert start_event.kind == agent.EventKind.SESSION_START

    await session.process_input("Create hello.txt and update reasoning effort.")

    events = await _collect_events_until_processing_end(stream)
    assert session.state == agent.SessionState.IDLE
    assert len(client.requests) == 2
    _assert_provider_request_shape(client.requests[0], provider_case=provider_case)
    _assert_provider_request_shape(
        client.requests[1],
        provider_case=provider_case,
        reasoning_effort=None,
        provider_options={},
    )
    _assert_request_history_prefixes(client.requests, session.history, (1, 3))
    _assert_events(
        [start_event, *events],
        user_input="Create hello.txt and update reasoning effort.",
        turn_specs=(
            _turn("resp-1", "Writing hello.txt.", write_call),
            _turn("resp-2", "Created hello.txt."),
        ),
    )
    _assert_session_history(
        session,
        user_input="Create hello.txt and update reasoning effort.",
        turn_specs=(
            _turn("resp-1", "Writing hello.txt.", write_call),
            _turn("resp-2", "Created hello.txt."),
        ),
    )
    _assert_file_tree(tmp_path, environment, (("hello.txt", file_content),))


@pytest.mark.parametrize(
    "provider_case",
    PROVIDER_CASES,
    ids=[case.name for case in PROVIDER_CASES],
)
@pytest.mark.parametrize("client_mode", ["complete", "stream"], ids=["complete", "stream"])
@pytest.mark.asyncio
async def test_cross_provider_parity_loop_detection(
    tmp_path: Path,
    provider_case: ProviderCase,
    client_mode: str,
) -> None:
    loop_command = "echo loop"
    exec_result = agent.ExecResult(
        stdout="loop\n",
        stderr="",
        exit_code=0,
        timed_out=False,
        duration_ms=5,
    )
    first_shell_call = _shell_call(
        call_id="call-1",
        command=loop_command,
        timeout_ms=1000,
        exec_result=exec_result,
    )
    second_shell_call = _shell_call(
        call_id="call-2",
        command=loop_command,
        timeout_ms=1000,
        exec_result=exec_result,
    )
    environment = _RecordingExecutionEnvironment(
        working_dir=tmp_path,
        exec_handler=_shell_exec_handler(
            expected_command=loop_command,
            expected_timeout_ms=1000,
            result=exec_result,
        ),
    )
    profile = provider_case.factory(
        model=provider_case.model,
        supports_streaming=client_mode == "stream",
    )
    if client_mode == "complete":
        client = _CompleteClient(
            [
                _build_complete_response(
                    provider_case,
                    _turn("resp-1", "Running the command.", first_shell_call),
                ),
                _build_complete_response(
                    provider_case,
                    _turn("resp-2", "Running the command again.", second_shell_call),
                ),
                _build_complete_response(
                    provider_case,
                    _turn("resp-3", "The repeated pattern was detected."),
                ),
            ]
        )
    else:
        client = _StreamingClient(
            [
                _build_stream_turn(
                    provider_case,
                    _turn("resp-1", "Running the command.", first_shell_call),
                ),
                _build_stream_turn(
                    provider_case,
                    _turn("resp-2", "Running the command again.", second_shell_call),
                ),
                _build_stream_turn(
                    provider_case,
                    _turn("resp-3", "The repeated pattern was detected."),
                ),
            ]
        )

    session = agent.Session(
        profile=profile,
        execution_env=environment,
        llm_client=client,
        config=agent.SessionConfig(
            reasoning_effort=REASONING_EFFORT,
            loop_detection_window=2,
        ),
    )
    stream = session.events()

    start_event = await _next_event(stream)
    assert start_event.kind == agent.EventKind.SESSION_START

    await session.process_input("Detect the loop.")

    events = await _collect_events_until_processing_end(stream)
    assert session.state == agent.SessionState.IDLE
    assert len(client.requests) == 3
    for request in client.requests:
        _assert_provider_request_shape(request, provider_case=provider_case)
    _assert_request_history_prefixes(client.requests, session.history, (1, 3, 6))
    assert [event.kind for event in events] == [
        agent.EventKind.USER_INPUT,
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
        agent.EventKind.LOOP_DETECTION,
        agent.EventKind.ASSISTANT_TEXT_START,
        agent.EventKind.ASSISTANT_TEXT_DELTA,
        agent.EventKind.ASSISTANT_TEXT_END,
        agent.EventKind.PROCESSING_END,
    ]
    assert events[11].data == {"message": agent.LOOP_DETECTION_WARNING}
    assert isinstance(session.history[5], agent.SteeringTurn)
    assert session.history[5].text == agent.LOOP_DETECTION_WARNING
    assert isinstance(session.history[6], agent.AssistantTurn)
    assert session.history[6].text == "The repeated pattern was detected."
    _assert_file_tree(tmp_path, environment, ())


@pytest.mark.parametrize(
    "provider_case",
    PROVIDER_CASES,
    ids=[case.name for case in PROVIDER_CASES],
)
@pytest.mark.asyncio
async def test_cross_provider_parity_subagent_spawn_and_wait(
    tmp_path: Path,
    provider_case: ProviderCase,
) -> None:
    client = _BlockingCompleteClient(
        [
            _build_complete_response(
                provider_case,
                _turn("resp-1", "child response 1"),
            ),
            _build_complete_response(
                provider_case,
                _turn("resp-2", "child response 2"),
            ),
        ]
    )
    profile = provider_case.factory(model=provider_case.model, supports_streaming=False)
    session = agent.Session(
        profile=profile,
        execution_env=agent.LocalExecutionEnvironment(working_dir=tmp_path),
        llm_client=client,
        config=agent.SessionConfig(reasoning_effort=REASONING_EFFORT),
    )
    stream = session.events()

    start_event = await _next_event(stream)
    assert start_event.kind == agent.EventKind.SESSION_START

    spawn_result = await agent.execute_tool_call(
        session,
        unified_llm.ToolCallData(
            id="spawn-1",
            name="spawn_agent",
            arguments={"task": "Investigate the repository"},
        ),
    )
    assert spawn_result.is_error is False
    assert spawn_result.content["status"] == "running"
    agent_id = spawn_result.content["agent_id"]
    handle = next(iter(session.active_subagents.values()))
    child_session = handle.session
    assert child_session is not None
    assert handle.status == agent.SubAgentStatus.RUNNING

    spawn_start_event = await _next_event(stream)
    assert spawn_start_event.kind == agent.EventKind.TOOL_CALL_START
    assert spawn_start_event.data == {
        "tool_call_id": "spawn-1",
        "tool_name": "spawn_agent",
    }
    spawn_end_event = await _next_event(stream)
    assert spawn_end_event.kind == agent.EventKind.TOOL_CALL_END
    assert spawn_end_event.data["tool_name"] == "spawn_agent"
    assert spawn_end_event.data["output"].content["status"] == "running"

    await asyncio.wait_for(client.started[0].wait(), timeout=1)

    send_result = await agent.execute_tool_call(
        session,
        unified_llm.ToolCallData(
            id="send-1",
            name="send_input",
            arguments={
                "agent_id": agent_id,
                "message": "Please continue",
            },
        ),
    )
    assert send_result.is_error is False
    assert send_result.content["status"] == "running"

    send_start_event = await _next_event(stream)
    assert send_start_event.kind == agent.EventKind.TOOL_CALL_START
    assert send_start_event.data == {
        "tool_call_id": "send-1",
        "tool_name": "send_input",
    }
    send_end_event = await _next_event(stream)
    assert send_end_event.kind == agent.EventKind.TOOL_CALL_END
    assert send_end_event.data["output"].content["status"] == "running"

    client.released[0].set()
    await asyncio.wait_for(client.started[1].wait(), timeout=1)

    wait_task = asyncio.create_task(
        agent.execute_tool_call(
            session,
            unified_llm.ToolCallData(
                id="wait-1",
                name="wait",
                arguments={"agent_id": agent_id},
            ),
        )
    )
    wait_start_event = await _next_event(stream)
    assert wait_start_event.kind == agent.EventKind.TOOL_CALL_START
    assert wait_start_event.data == {
        "tool_call_id": "wait-1",
        "tool_name": "wait",
    }

    client.released[1].set()
    wait_result = await asyncio.wait_for(wait_task, timeout=1)
    assert wait_result.is_error is False
    assert wait_result.content["status"] == "completed"
    assert wait_result.content["success"] is True
    assert wait_result.content["output"] == "child response 2"
    assert wait_result.content["turns_used"] == 4
    assert handle.result is not None
    assert handle.result.status == agent.SubAgentStatus.COMPLETED

    wait_end_event = await _next_event(stream)
    assert wait_end_event.kind == agent.EventKind.TOOL_CALL_END
    assert wait_end_event.data["output"].content["status"] == "completed"

    _assert_provider_request_shape(
        client.requests[0],
        provider_case=provider_case,
        reasoning_effort=REASONING_EFFORT,
    )
    _assert_provider_request_shape(
        client.requests[1],
        provider_case=provider_case,
        reasoning_effort=REASONING_EFFORT,
    )
    _assert_request_history_prefixes(client.requests, child_session.history, (1, 3))
    assert child_session.state == agent.SessionState.IDLE
    assert [type(turn).__name__ for turn in child_session.history] == [
        "UserTurn",
        "AssistantTurn",
        "UserTurn",
        "AssistantTurn",
    ]
    assert [turn.text for turn in child_session.history] == [
        "Investigate the repository",
        "child response 1",
        "Please continue",
        "child response 2",
    ]
    assert session.state == agent.SessionState.IDLE
    await session.close()
    assert session.state == agent.SessionState.CLOSED
    assert session.active_subagents == {}
