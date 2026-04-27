from __future__ import annotations

import itertools
import json

import pytest

from spark_common.codex_app_client import CodexAppServerClient
import spark_common.codex_app_protocol as codex_app_protocol
from spark_common.turn_stream import TurnStreamEvent


def test_turn_stream_content_events_require_channel() -> None:
    with pytest.raises(ValueError, match="must set channel"):
        TurnStreamEvent(kind="content_delta", content_delta="Ack")


def test_shared_client_ensure_process_initializes_with_experimental_api_opt_in() -> None:
    client = CodexAppServerClient("/tmp/project")
    requests: list[tuple[str, dict[str, object] | None]] = []
    notifications: list[dict[str, object]] = []

    class DummyStdout:
        def readline(self) -> str:
            return ""

    class DummyProc:
        stdout = DummyStdout()
        stdin = object()

        def poll(self) -> int | None:
            return None

    def fake_send_request(method: str, params: dict[str, object] | None, **kwargs) -> dict[str, object]:
        requests.append((method, params))
        if method == "initialize":
            return {"result": {}}
        raise AssertionError(f"unexpected request: {method}")

    client.send_json = lambda payload: notifications.append(payload)  # type: ignore[method-assign]
    client.send_request = fake_send_request  # type: ignore[method-assign]

    client.ensure_process(
        popen_factory=lambda *args, **kwargs: DummyProc(),
        runtime_environment_builder=lambda: {},
    )

    assert requests == [
        (
            "initialize",
            {
                "clientInfo": {"name": "spark", "version": "0.1"},
                "capabilities": {"experimentalApi": True},
            },
        )
    ]
    assert notifications == [{"jsonrpc": "2.0", "method": "initialized", "params": {}}]


def test_shared_client_wait_for_response_auto_approves_requests_and_queues_normalized_message() -> None:
    client = CodexAppServerClient("/tmp/project")
    outgoing_lines: list[str] = []
    rpc_log: list[tuple[str, str]] = []
    lines = iter(
        [
            '{"jsonrpc":"2.0","id":2,"method":"item/commandExecution/requestApproval","params":{"itemId":"cmd-1","command":["git","status"]}}',
            '{"jsonrpc":"2.0","id":7,"result":{}}',
        ]
    )

    class DummyStdin:
        def write(self, text: str) -> None:
            outgoing_lines.append(text)

        def flush(self) -> None:
            return None

    class DummyProc:
        stdin = DummyStdin()

        def poll(self) -> int | None:
            return None

    client.proc = DummyProc()  # type: ignore[assignment]
    client.set_raw_rpc_logger(lambda direction, line: rpc_log.append((direction, line)))

    response = client.wait_for_response(7, read_line=lambda wait: next(lines, None))

    assert response == {"jsonrpc": "2.0", "id": 7, "result": {}}
    assert json.loads(outgoing_lines[0]) == {
        "jsonrpc": "2.0",
        "id": 2,
        "result": {"decision": "acceptForSession"},
    }
    queued_message = client.pending_messages.popleft()
    events = codex_app_protocol.process_turn_message(
        queued_message,
        codex_app_protocol.CodexAppServerTurnState(),
    )
    assert len(events) == 1
    assert events[0].kind == "tool_call_started"
    assert events[0].source.raw_kind == "command_approval_requested"
    assert events[0].content_delta == "git status"
    assert rpc_log == [
        ("incoming", '{"jsonrpc":"2.0","id":2,"method":"item/commandExecution/requestApproval","params":{"itemId":"cmd-1","command":["git","status"]}}'),
        ("outgoing", '{"jsonrpc": "2.0", "id": 2, "result": {"decision": "acceptForSession"}}'),
        ("incoming", '{"jsonrpc":"2.0","id":7,"result":{}}'),
    ]


def test_shared_client_run_turn_drains_notifications_queued_during_turn_start_response() -> None:
    client = CodexAppServerClient("/tmp/project")
    lines = iter(
        [
            '{"jsonrpc":"2.0","method":"item/agentMessage/delta","params":{"delta":"Ack"}}',
            '{"jsonrpc":"2.0","method":"item/completed","params":{"item":{"type":"AgentMessage","id":"msg-1","content":[{"type":"Text","text":"Ack"}],"phase":"final_answer"}}}',
            '{"jsonrpc":"2.0","id":1,"result":{"turn":{"id":"turn-123","status":"inProgress","items":[]}}}',
            '{"jsonrpc":"2.0","method":"turn/completed","params":{"turn":{"id":"turn-123","status":"completed"}}}',
        ]
    )
    emitted_events: list[TurnStreamEvent] = []

    class DummyStdin:
        def write(self, text: str) -> None:
            return None

        def flush(self) -> None:
            return None

    class DummyProc:
        stdin = DummyStdin()

        def poll(self) -> int | None:
            return None

    client.proc = DummyProc()  # type: ignore[assignment]
    client.read_line = lambda wait: next(lines, None)  # type: ignore[method-assign]

    result = client.run_turn(
        thread_id="thread-123",
        prompt="hello",
        model="gpt-test",
        on_event=emitted_events.append,
    )

    assert result.assistant_message == "Ack"
    assert [event.kind for event in emitted_events] == [
        "content_delta",
        "content_completed",
        "turn_completed",
    ]
    assert not client.pending_messages


def test_shared_client_run_turn_requires_turn_completed_after_final_answer() -> None:
    client = CodexAppServerClient("/tmp/project")
    lines = iter(
        [
            '{"jsonrpc":"2.0","id":1,"result":{"turn":{"id":"turn-123","status":"inProgress","items":[]}}}',
            '{"jsonrpc":"2.0","method":"item/agentMessage/delta","params":{"delta":"Ack"}}',
            '{"jsonrpc":"2.0","method":"item/completed","params":{"item":{"type":"AgentMessage","id":"msg-1","content":[{"type":"Text","text":"Ack"}],"phase":"final_answer"}}}',
            None,
        ]
    )
    monotonic_values = itertools.count(0.0, 0.01)

    class DummyStdin:
        def write(self, text: str) -> None:
            return None

        def flush(self) -> None:
            return None

    class DummyProc:
        stdin = DummyStdin()

        def poll(self) -> int | None:
            return None

    client.proc = DummyProc()  # type: ignore[assignment]
    client.read_line = lambda wait: next(lines, None)  # type: ignore[method-assign]

    with pytest.raises(RuntimeError, match="timed out waiting for activity"):
        client.run_turn(
            thread_id="thread-123",
            prompt="hello",
            model="gpt-test",
            idle_timeout_seconds=1.0,
            now=lambda: next(monotonic_values),
        )


@pytest.mark.parametrize(
    ("chat_mode", "expected_collaboration_mode"),
    [
        (
            "chat",
            {
                "mode": "default",
                "settings": {
                    "model": "gpt-test",
                },
            },
        ),
        (
            "plan",
            {
                "mode": "plan",
                "settings": {
                    "model": "gpt-test",
                },
            },
        ),
    ],
)
def test_shared_client_run_turn_includes_collaboration_mode(
    chat_mode: str,
    expected_collaboration_mode: dict[str, object],
) -> None:
    client = CodexAppServerClient("/tmp/project")
    sent_requests: list[tuple[str, dict[str, object] | None]] = []
    messages = iter(
        [
            {
                "jsonrpc": "2.0",
                "method": "item/completed",
                "params": {
                    "turnId": "turn-123",
                    "item": {
                        "type": "AgentMessage",
                        "id": "msg-1",
                        "content": [{"type": "Text", "text": "Ack"}],
                        "phase": "final_answer",
                    },
                },
            },
            {
                "jsonrpc": "2.0",
                "method": "turn/completed",
                "params": {"turn": {"id": "turn-123", "status": "completed"}},
            },
        ]
    )

    result = client.run_turn(
        thread_id="thread-123",
        prompt="hello",
        model="gpt-test",
        chat_mode=chat_mode,
        send_request=lambda method, params: sent_requests.append((method, params))
        or {"result": {"turn": {"id": "turn-123", "status": "inProgress"}}},
        next_message=lambda wait: next(messages, None),
    )

    assert result.assistant_message == "Ack"
    assert sent_requests == [
        (
            "turn/start",
            {
                "threadId": "thread-123",
                "input": [{"type": "text", "text": "hello"}],
                "collaborationMode": expected_collaboration_mode,
                "approvalPolicy": "never",
                "sandboxPolicy": {"type": "dangerFullAccess"},
                "cwd": "/tmp/project",
                "model": "gpt-test",
            },
        )
    ]


def test_shared_client_default_model_reads_default_entry_from_model_list() -> None:
    client = CodexAppServerClient("/tmp/project")

    client.send_request = lambda method, params, **kwargs: {  # type: ignore[method-assign]
        "result": {
            "data": [
                {
                    "model": "gpt-alt",
                    "isDefault": False,
                },
                {
                    "model": "gpt-default",
                    "isDefault": True,
                },
            ]
        }
    }

    assert client.default_model() == "gpt-default"


def test_shared_client_model_metadata_parses_reasoning_efforts() -> None:
    client = CodexAppServerClient("/tmp/project")

    client.send_request = lambda method, params, **kwargs: {  # type: ignore[method-assign]
        "result": {
            "data": [
                {
                    "model": "gpt-5.4",
                    "displayName": "GPT-5.4",
                    "isDefault": True,
                    "supportedReasoningEfforts": [
                        {"reasoningEffort": "low", "description": "Fast responses with lighter reasoning"},
                        {"reasoningEffort": "medium", "description": "Balances speed and reasoning depth"},
                        {"reasoningEffort": "high", "description": "Greater reasoning depth"},
                        {"reasoningEffort": "xhigh", "description": "Extra high reasoning"},
                        {"reasoningEffort": "invalid", "description": "Unsupported effort"},
                    ],
                    "defaultReasoningEffort": "medium",
                },
            ]
        }
    }

    assert [model.to_dict() for model in client.list_models()] == [
        {
            "id": "gpt-5.4",
            "display": "GPT-5.4",
            "is_default": True,
            "supported_reasoning_efforts": ["low", "medium", "high", "xhigh"],
            "default_reasoning_effort": "medium",
        }
    ]


def test_shared_client_run_turn_includes_non_empty_reasoning_effort() -> None:
    client = CodexAppServerClient("/tmp/project")
    sent_requests: list[tuple[str, dict[str, object] | None]] = []
    messages = iter(
        [
            {
                "jsonrpc": "2.0",
                "method": "item/completed",
                "params": {
                    "turnId": "turn-123",
                    "item": {
                        "type": "AgentMessage",
                        "id": "msg-1",
                        "content": [{"type": "Text", "text": "Ack"}],
                        "phase": "final_answer",
                    },
                },
            },
            {
                "jsonrpc": "2.0",
                "method": "turn/completed",
                "params": {"turn": {"id": "turn-123", "status": "completed"}},
            },
        ]
    )

    client.run_turn(
        thread_id="thread-123",
        prompt="hello",
        model="gpt-test",
        reasoning_effort="high",
        send_request=lambda method, params: sent_requests.append((method, params))
        or {"result": {"turn": {"id": "turn-123", "status": "inProgress"}}},
        next_message=lambda wait: next(messages, None),
    )

    assert sent_requests[0][1] is not None
    assert sent_requests[0][1]["reasoningEffort"] == "high"


def test_shared_client_run_turn_omits_blank_reasoning_effort() -> None:
    client = CodexAppServerClient("/tmp/project")
    sent_requests: list[tuple[str, dict[str, object] | None]] = []
    messages = iter(
        [
            {
                "jsonrpc": "2.0",
                "method": "item/completed",
                "params": {
                    "turnId": "turn-123",
                    "item": {
                        "type": "AgentMessage",
                        "id": "msg-1",
                        "content": [{"type": "Text", "text": "Ack"}],
                        "phase": "final_answer",
                    },
                },
            },
            {
                "jsonrpc": "2.0",
                "method": "turn/completed",
                "params": {"turn": {"id": "turn-123", "status": "completed"}},
            },
        ]
    )

    client.run_turn(
        thread_id="thread-123",
        prompt="hello",
        model="gpt-test",
        reasoning_effort="",
        send_request=lambda method, params: sent_requests.append((method, params))
        or {"result": {"turn": {"id": "turn-123", "status": "inProgress"}}},
        next_message=lambda wait: next(messages, None),
    )

    assert sent_requests[0][1] is not None
    assert "reasoningEffort" not in sent_requests[0][1]


def test_shared_client_run_turn_exposes_structured_token_usage_payload() -> None:
    client = CodexAppServerClient("/tmp/project")
    lines = iter(
        [
            '{"jsonrpc":"2.0","id":1,"result":{"turn":{"id":"turn-123","status":"inProgress","items":[]}}}',
            '{"jsonrpc":"2.0","method":"thread/tokenUsage/updated","params":{"turnId":"turn-123","tokenUsage":{"last":{"inputTokens":12,"cachedInputTokens":2,"outputTokens":4,"reasoningOutputTokens":1,"totalTokens":16},"total":{"inputTokens":12,"cachedInputTokens":2,"outputTokens":4,"reasoningOutputTokens":1,"totalTokens":16}}}}',
            '{"jsonrpc":"2.0","method":"turn/completed","params":{"turn":{"id":"turn-123","status":"completed"}}}',
        ]
    )

    class DummyStdin:
        def write(self, text: str) -> None:
            return None

        def flush(self) -> None:
            return None

    class DummyProc:
        stdin = DummyStdin()

        def poll(self) -> int | None:
            return None

    client.proc = DummyProc()  # type: ignore[assignment]
    client.read_line = lambda wait: next(lines, None)  # type: ignore[method-assign]

    result = client.run_turn(
        thread_id="thread-123",
        prompt="hello",
        model="gpt-test",
    )

    assert result.token_total == 16
    assert result.token_usage_payload == {
        "last": {
            "inputTokens": 12,
            "cachedInputTokens": 2,
            "outputTokens": 4,
            "reasoningOutputTokens": 1,
            "totalTokens": 16,
        },
        "total": {
            "inputTokens": 12,
            "cachedInputTokens": 2,
            "outputTokens": 4,
            "reasoningOutputTokens": 1,
            "totalTokens": 16,
        },
    }


def test_shared_client_run_turn_counts_unparsed_lines_as_activity() -> None:
    client = CodexAppServerClient("/tmp/project")
    lines = iter(
        [
            "Reading specs/unified-llm/source.md",
            None,
            "Reading README.md",
            '{"jsonrpc":"2.0","method":"item/agentMessage/delta","params":{"delta":"Ack"}}',
            '{"jsonrpc":"2.0","method":"item/completed","params":{"item":{"type":"AgentMessage","id":"msg-1","content":[{"type":"Text","text":"Ack"}],"phase":"final_answer"}}}',
            '{"jsonrpc":"2.0","method":"turn/completed","params":{"turn":{"id":"turn-123","status":"completed"}}}',
        ]
    )
    monotonic_values = itertools.count(0.0, 0.01)

    class DummyStdin:
        def write(self, text: str) -> None:
            return None

        def flush(self) -> None:
            return None

    class DummyProc:
        stdin = DummyStdin()

        def poll(self) -> int | None:
            return None

    client.proc = DummyProc()  # type: ignore[assignment]
    client.read_line = lambda wait: next(lines, None)  # type: ignore[method-assign]

    result = client.run_turn(
        thread_id="thread-123",
        prompt="hello",
        model="gpt-test",
        idle_timeout_seconds=0.08,
        send_request=lambda method, params: {"result": {"turn": {"id": "turn-123", "status": "inProgress"}}},
        now=lambda: next(monotonic_values),
    )

    assert result.assistant_message == "Ack"


def test_shared_client_run_turn_tracks_plan_items_without_assistant_message() -> None:
    client = CodexAppServerClient("/tmp/project")
    lines = iter(
        [
            '{"jsonrpc":"2.0","id":1,"result":{"turn":{"id":"turn-123","status":"inProgress","items":[]}}}',
            '{"jsonrpc":"2.0","method":"item/plan/delta","params":{"itemId":"plan-1","delta":"1. Patch the real path.\\n"}}',
            '{"jsonrpc":"2.0","method":"item/completed","params":{"item":{"type":"Plan","id":"plan-1","text":"1. Patch the real path.\\n2. Add the regression."}}}',
            '{"jsonrpc":"2.0","method":"turn/completed","params":{"turn":{"id":"turn-123","status":"completed"}}}',
        ]
    )
    emitted_events: list[TurnStreamEvent] = []

    class DummyStdin:
        def write(self, text: str) -> None:
            return None

        def flush(self) -> None:
            return None

    class DummyProc:
        stdin = DummyStdin()

        def poll(self) -> int | None:
            return None

    client.proc = DummyProc()  # type: ignore[assignment]
    client.read_line = lambda wait: next(lines, None)  # type: ignore[method-assign]

    result = client.run_turn(
        thread_id="thread-123",
        prompt="hello",
        model="gpt-test",
        on_event=emitted_events.append,
    )

    assert result.assistant_message == ""
    assert result.plan_message == "1. Patch the real path.\n2. Add the regression."
    assert [event.kind for event in emitted_events] == [
        "content_delta",
        "content_completed",
        "turn_completed",
    ]


def test_shared_client_run_turn_ignores_non_matching_turn_completed() -> None:
    client = CodexAppServerClient("/tmp/project")
    lines = iter(
        [
            '{"jsonrpc":"2.0","id":1,"result":{"turn":{"id":"turn-123","status":"inProgress","items":[]}}}',
            '{"jsonrpc":"2.0","method":"item/agentMessage/delta","params":{"delta":"Ack","itemId":"msg-1"}}',
            '{"jsonrpc":"2.0","method":"item/completed","params":{"item":{"type":"AgentMessage","id":"msg-1","content":[{"type":"Text","text":"Ack"}],"phase":"final_answer"}}}',
            '{"jsonrpc":"2.0","method":"turn/completed","params":{"turn":{"id":"turn-stale","status":"completed"}}}',
            '{"jsonrpc":"2.0","method":"turn/completed","params":{"turn":{"id":"turn-123","status":"completed"}}}',
        ]
    )
    emitted_events: list[TurnStreamEvent] = []

    class DummyStdin:
        def write(self, text: str) -> None:
            return None

        def flush(self) -> None:
            return None

    class DummyProc:
        stdin = DummyStdin()

        def poll(self) -> int | None:
            return None

    client.proc = DummyProc()  # type: ignore[assignment]
    client.read_line = lambda wait: next(lines, None)  # type: ignore[method-assign]

    result = client.run_turn(
        thread_id="thread-123",
        prompt="hello",
        model="gpt-test",
        on_event=emitted_events.append,
    )

    assert result.assistant_message == "Ack"
    assert [event.kind for event in emitted_events] == [
        "content_delta",
        "content_completed",
        "turn_completed",
    ]


def test_shared_client_run_turn_raises_when_process_exits_before_completion() -> None:
    client = CodexAppServerClient("/tmp/project")
    lines = iter(
        [
            '{"jsonrpc":"2.0","id":1,"result":{"turn":{"id":"turn-123","status":"inProgress","items":[]}}}',
            None,
        ]
    )

    class DummyStdin:
        def write(self, text: str) -> None:
            return None

        def flush(self) -> None:
            return None

    class DummyProc:
        stdin = DummyStdin()

        def __init__(self) -> None:
            self._poll_count = 0

        def poll(self) -> int | None:
            self._poll_count += 1
            if self._poll_count == 1:
                return None
            return 0

    client.proc = DummyProc()  # type: ignore[assignment]
    client.read_line = lambda wait: next(lines, None)  # type: ignore[method-assign]

    with pytest.raises(RuntimeError, match="exited before turn completion"):
        client.run_turn(
            thread_id="thread-123",
            prompt="hello",
            model="gpt-test",
        )


def test_shared_client_run_turn_surfaces_turn_start_error_message() -> None:
    client = CodexAppServerClient("/tmp/project")

    with pytest.raises(
        RuntimeError,
        match="turn/start failed: turn/start.collaborationMode requires experimentalApi capability",
    ):
        client.run_turn(
            thread_id="thread-123",
            prompt="hello",
            model="gpt-test",
            chat_mode="plan",
            send_request=lambda method, params: {
                "error": {
                    "code": -32600,
                    "message": "turn/start.collaborationMode requires experimentalApi capability",
                }
            },
            next_message=lambda wait: None,
        )
