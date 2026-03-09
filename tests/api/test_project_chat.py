from __future__ import annotations

import json
from pathlib import Path

import pytest

import attractor.api.server as server
import attractor.api.project_chat as project_chat
from attractor.prompt_templates import PROMPTS_FILE_NAME
from attractor.storage import ensure_project_paths


def test_extract_command_text_handles_list_and_string_payloads() -> None:
    assert project_chat._extract_command_text({"command": ["git", "status", "--short"]}) == "git status --short"
    assert project_chat._extract_command_text({"commandLine": "npm test"}) == "npm test"


def test_parse_chat_response_payload_accepts_plain_text_and_json() -> None:
    assistant_message, payload = project_chat._parse_chat_response_payload("Plain text reply.")

    assert assistant_message == "Plain text reply."
    assert payload is None

    assistant_message, payload = project_chat._parse_chat_response_payload(
        '{"assistant_message":"Hello.","spec_proposal":null}'
    )

    assert assistant_message == "Hello."
    assert payload == {
        "assistant_message": "Hello.",
        "spec_proposal": None,
    }


def test_project_chat_service_creates_default_prompt_templates_file(tmp_path: Path) -> None:
    service = project_chat.ProjectChatService(tmp_path)

    prompts_path = tmp_path / "config" / PROMPTS_FILE_NAME

    assert prompts_path.exists()
    prompt_text = prompts_path.read_text(encoding="utf-8")
    assert "[project_chat]" in prompt_text
    assert "Help the user understand the project" in prompt_text
    assert "call the draft_spec_proposal tool" in prompt_text
    assert service._prompt_templates.chat


def test_project_chat_service_uses_custom_prompt_templates(tmp_path: Path) -> None:
    prompts_path = tmp_path / "config" / PROMPTS_FILE_NAME
    prompts_path.parent.mkdir(parents=True, exist_ok=True)
    prompts_path.write_text(
        "\n".join(
            [
                "[project_chat]",
                "chat = '''CHAT {{project_path}} :: {{latest_user_message}} :: {{recent_conversation}}'''",
                "execution_planning = '''PLAN {{approved_spec_edit_proposal}} :: {{review_feedback}}'''",
                "",
            ]
        ),
        encoding="utf-8",
    )
    service = project_chat.ProjectChatService(tmp_path)
    state = project_chat.ConversationState(
        conversation_id="conversation-test",
        project_path="/tmp/project",
        turns=[
            project_chat.ConversationTurn(
                id="turn-user-1",
                role="user",
                content="Older message",
                timestamp="2026-03-08T12:00:00Z",
            )
        ],
    )

    chat_prompt = service._build_chat_prompt(state, "Latest message")
    execution_prompt = service._build_execution_planning_prompt(
        state,
        project_chat.SpecEditProposal(
            id="proposal-1",
            created_at="2026-03-08T12:01:00Z",
            summary="Summary",
            changes=[project_chat.SpecEditProposalChange(path="specs/ui-spec.md", before="old", after="new")],
            status="approved",
        ),
        "Needs refinement",
    )

    assert chat_prompt == "CHAT /tmp/project :: Latest message :: USER: Older message"
    assert '"id": "proposal-1"' in execution_prompt
    assert execution_prompt.endswith(":: Needs refinement")


def test_extract_spec_proposal_payload_requires_summary_and_changes() -> None:
    payload = project_chat._extract_spec_proposal_payload(
        {
            "summary": "Tighten the top bar.",
            "changes": [
                {
                    "path": "specs/ui-spec.md",
                    "before": "Header includes runtime metadata.",
                    "after": "Header includes only navigation and active project context.",
                }
            ],
            "rationale": "Reduce chrome noise.",
        }
    )

    assert payload["summary"] == "Tighten the top bar."
    assert payload["rationale"] == "Reduce chrome noise."
    assert payload["changes"][0]["path"] == "specs/ui-spec.md"


def test_extract_file_paths_deduplicates_nested_entries() -> None:
    payload = {
        "path": "frontend/src/components/ProjectsPanel.tsx",
        "files": [
            "frontend/src/components/ProjectsPanel.tsx",
            {"path": "frontend/src/lib/apiClient.ts"},
        ],
        "changes": [
            {"filePath": "frontend/src/components/ProjectsPanel.tsx"},
            {"file_path": "frontend/src/store.ts"},
        ],
    }

    assert project_chat._extract_file_paths(payload) == [
        "frontend/src/components/ProjectsPanel.tsx",
        "frontend/src/lib/apiClient.ts",
        "frontend/src/store.ts",
    ]


def test_append_tool_output_keeps_latest_tail() -> None:
    output = project_chat._append_tool_output("abc", "def", limit=4)

    assert output == "cdef"


def test_tool_call_from_command_execution_item_uses_completed_payload() -> None:
    tool_call = project_chat._tool_call_from_item(
        {
            "type": "commandExecution",
            "id": "call_123",
            "command": "/bin/bash -lc 'ls -1 /app | head -n 5'",
            "status": "completed",
            "aggregatedOutput": "AGENTS.md\nDockerfile\n",
            "exitCode": 0,
        }
    )

    assert tool_call is not None
    assert tool_call.id == "call_123"
    assert tool_call.kind == "command_execution"
    assert tool_call.status == "completed"
    assert tool_call.command == "/bin/bash -lc 'ls -1 /app | head -n 5'"
    assert tool_call.output == "AGENTS.md\nDockerfile\n"


def test_tool_call_from_file_change_item_collects_paths() -> None:
    tool_call = project_chat._tool_call_from_item(
        {
            "type": "fileChange",
            "status": "inProgress",
            "changes": [
                {"path": "frontend/src/components/ProjectsPanel.tsx"},
                {"filePath": "frontend/src/store.ts"},
            ],
        }
    )

    assert tool_call is not None
    assert tool_call.id
    assert tool_call.kind == "file_change"
    assert tool_call.status == "running"
    assert tool_call.file_paths == [
        "frontend/src/components/ProjectsPanel.tsx",
        "frontend/src/store.ts",
    ]


def test_append_turn_event_records_tool_call_against_assistant_turn(tmp_path: Path) -> None:
    service = project_chat.ProjectChatService(tmp_path)
    state = project_chat.ConversationState(
        conversation_id="conversation-test",
        project_path="/tmp/project",
        turns=[
            project_chat.ConversationTurn(
                id="turn-user-1",
                role="user",
                content="Run ls",
                timestamp="2026-03-06T23:00:00Z",
            ),
            project_chat.ConversationTurn(
                id="turn-assistant-1",
                role="assistant",
                content="",
                timestamp="2026-03-06T23:00:01Z",
                status="streaming",
                parent_turn_id="turn-user-1",
            ),
        ],
    )

    started_event = service._append_turn_event(
        state,
        "turn-assistant-1",
        "tool_call_started",
        tool_call_id="call-1",
        tool_call=project_chat.ToolCallRecord(
            id="call-1",
            kind="command_execution",
            status="running",
            title="Run command",
            command="ls -1 /app",
            output=None,
        ),
    )
    completed_event = service._append_turn_event(
        state,
        "turn-assistant-1",
        "tool_call_completed",
        tool_call_id="call-1",
        tool_call=project_chat.ToolCallRecord(
            id="call-1",
            kind="command_execution",
            status="completed",
            title="Run command",
            command="ls -1 /app",
            output="AGENTS.md\n",
        ),
    )

    assert [event.kind for event in state.turn_events] == ["tool_call_started", "tool_call_completed"]
    assert started_event.turn_id == "turn-assistant-1"
    assert started_event.sequence == 1
    assert completed_event.sequence == 2
    assert completed_event.tool_call is not None
    assert completed_event.tool_call.output == "AGENTS.md\n"


def test_conversation_session_state_round_trips(tmp_path: Path) -> None:
    service = project_chat.ProjectChatService(tmp_path)
    project_paths = ensure_project_paths(tmp_path, "/tmp/project")
    session_state = project_chat.ConversationSessionState(
        conversation_id="conversation-test",
        updated_at="2026-03-06T23:59:00Z",
        project_path="/tmp/project",
        runtime_project_path="/runtime/project",
        thread_id="thread-123",
    )

    service._write_session_state(session_state)
    loaded = service._read_session_state("conversation-test")

    assert loaded is not None
    assert loaded.conversation_id == "conversation-test"
    assert loaded.thread_id == "thread-123"
    assert loaded.project_path == project_chat._normalize_project_path("/tmp/project")
    assert loaded.runtime_project_path == project_chat._normalize_project_path("/runtime/project")
    assert project_paths.project_file.exists()


def test_build_session_restores_persisted_thread_id(tmp_path: Path) -> None:
    service = project_chat.ProjectChatService(tmp_path)
    service._write_session_state(
        project_chat.ConversationSessionState(
            conversation_id="conversation-test",
            updated_at="2026-03-06T23:59:00Z",
            project_path="/tmp/project",
            runtime_project_path="/runtime/project",
            thread_id="thread-restored",
        )
    )

    session = service._build_session("conversation-test", "/tmp/project")

    assert session._thread_id == "thread-restored"


def test_chat_session_resumes_persisted_thread_before_starting(monkeypatch) -> None:
    updated_thread_ids: list[str] = []
    session = project_chat.CodexAppServerChatSession(
        "/tmp/project",
        persisted_thread_id="thread-existing",
        on_thread_id_updated=updated_thread_ids.append,
    )
    calls: list[tuple[str, dict[str, object] | None]] = []

    def fake_send_request(method: str, params: dict[str, object] | None) -> dict[str, object]:
        calls.append((method, params))
        if method == "thread/resume":
            return {"result": {"thread": {"id": "thread-existing"}}}
        raise AssertionError(f"unexpected method {method}")

    monkeypatch.setattr(session, "_send_request", fake_send_request)

    session._ensure_thread("gpt-test")

    assert [method for method, _ in calls] == ["thread/resume"]
    assert calls[0][1] is not None
    assert calls[0][1]["threadId"] == "thread-existing"
    assert session._thread_id == "thread-existing"
    assert updated_thread_ids == ["thread-existing"]


def test_chat_session_starts_new_durable_thread_when_resume_fails(monkeypatch) -> None:
    updated_thread_ids: list[str] = []
    session = project_chat.CodexAppServerChatSession(
        "/tmp/project",
        persisted_thread_id="thread-stale",
        on_thread_id_updated=updated_thread_ids.append,
    )
    calls: list[tuple[str, dict[str, object] | None]] = []

    def fake_send_request(method: str, params: dict[str, object] | None) -> dict[str, object]:
        calls.append((method, params))
        if method == "thread/resume":
            return {"error": {"code": -32600, "message": "no rollout found"}}
        if method == "thread/start":
            return {"result": {"thread": {"id": "thread-fresh"}}}
        raise AssertionError(f"unexpected method {method}")

    monkeypatch.setattr(session, "_send_request", fake_send_request)

    session._ensure_thread("gpt-test")

    assert [method for method, _ in calls] == ["thread/resume", "thread/start"]
    assert calls[1][1] is not None
    assert calls[1][1]["ephemeral"] is False
    assert session._thread_id == "thread-fresh"
    assert updated_thread_ids == ["thread-fresh"]


def test_chat_session_turn_uses_task_complete_message_but_waits_for_turn_completed(monkeypatch) -> None:
    session = project_chat.CodexAppServerChatSession("/tmp/project")
    lines = iter(
        [
            json.dumps(
                {
                    "method": "item/agentMessage/delta",
                    "params": {"delta": '{"assistant_message":"Ack"}'},
                }
            ),
            json.dumps(
                {
                    "method": "codex/event/task_complete",
                    "params": {
                        "msg": {
                            "type": "task_complete",
                            "last_agent_message": '{"assistant_message":"Ack"}',
                        }
                    },
                }
            ),
            json.dumps(
                {
                    "method": "turn/completed",
                    "params": {
                        "turn": {
                            "id": "turn-123",
                            "status": "completed",
                        }
                    },
                }
            ),
        ]
    )

    monkeypatch.setattr(session, "_ensure_process", lambda: None)

    def fake_ensure_thread(model: str | None) -> None:
        session._thread_id = "thread-123"
        session._thread_initialized = True

    monkeypatch.setattr(session, "_ensure_thread", fake_ensure_thread)
    monkeypatch.setattr(
        session,
        "_send_request",
        lambda method, params: {"result": {"turn": {"id": "turn-123", "status": "inProgress", "items": []}}},
    )
    monkeypatch.setattr(session, "_read_line", lambda wait: next(lines, None))

    result = session.turn("hello", None)

    assert result.assistant_message == '{"assistant_message":"Ack"}'


def test_chat_session_handles_dynamic_tool_call(monkeypatch) -> None:
    session = project_chat.CodexAppServerChatSession("/tmp/project")
    lines = iter(
        [
            json.dumps(
                {
                    "jsonrpc": "2.0",
                    "id": 99,
                    "method": "item/tool/call",
                    "params": {
                        "tool": "draft_spec_proposal",
                        "callId": "call-1",
                        "turnId": "turn-123",
                        "threadId": "thread-123",
                        "arguments": {
                            "summary": "Reduce header chrome.",
                            "changes": [
                                {
                                    "path": "specs/ui-spec.md",
                                    "before": "Header contains extra metadata.",
                                    "after": "Header contains only navigation and active project context.",
                                }
                            ],
                        },
                    },
                }
            ),
            json.dumps(
                {
                    "method": "codex/event/task_complete",
                    "params": {
                        "msg": {
                            "last_agent_message": "I drafted the spec proposal for review.",
                        }
                    },
                }
            ),
            json.dumps(
                {
                    "method": "turn/completed",
                    "params": {
                        "turn": {
                            "id": "turn-123",
                            "status": "completed",
                        }
                    },
                }
            ),
        ]
    )
    responses: list[dict[str, object]] = []
    live_events: list[project_chat.ChatTurnLiveEvent] = []

    monkeypatch.setattr(session, "_ensure_process", lambda: None)

    def fake_ensure_thread(model: str | None) -> None:
        session._thread_id = "thread-123"
        session._thread_initialized = True

    monkeypatch.setattr(session, "_ensure_thread", fake_ensure_thread)
    monkeypatch.setattr(
        session,
        "_send_request",
        lambda method, params: {"result": {"turn": {"id": "turn-123", "status": "inProgress", "items": []}}},
    )
    monkeypatch.setattr(session, "_read_line", lambda wait: next(lines, None))
    monkeypatch.setattr(session, "_send_response", lambda request_id, result=None: responses.append({"id": request_id, "result": result or {}}))

    result = session.turn(
        "hello",
        None,
        on_event=live_events.append,
        on_dynamic_tool_call=lambda tool, arguments, call_id: project_chat.DynamicToolInvocationResult(
            tool_call=project_chat.ToolCallRecord(
                id=call_id,
                kind="dynamic_tool",
                status="completed",
                title="Draft spec proposal",
                output="Reduce header chrome.",
            ),
            response={
                "success": True,
                "contentItems": [{"type": "inputText", "text": "Drafted spec proposal."}],
            },
            spec_proposal_payload=project_chat._extract_spec_proposal_payload(arguments),
        ),
    )

    assert result.assistant_message == "I drafted the spec proposal for review."
    assert result.spec_proposal_payloads[0]["summary"] == "Reduce header chrome."
    assert [event.kind for event in live_events] == ["tool_call_started", "tool_call_completed"]
    assert responses[0]["result"] == {
        "success": True,
        "contentItems": [{"type": "inputText", "text": "Drafted spec proposal."}],
    }


def test_chat_session_surfaces_reasoning_summary_progress(monkeypatch) -> None:
    session = project_chat.CodexAppServerChatSession("/tmp/project")
    lines = iter(
        [
            json.dumps(
                {
                    "method": "item/reasoning/summaryPartAdded",
                    "params": {
                        "part": {
                            "text": "Scanning the repository structure.",
                        },
                    },
                }
            ),
            json.dumps(
                {
                    "method": "item/agentMessage/delta",
                    "params": {
                        "delta": "I found the main entry points.",
                    },
                }
            ),
            json.dumps(
                {
                    "method": "codex/event/task_complete",
                    "params": {
                        "msg": {
                            "last_agent_message": "I found the main entry points.",
                        }
                    },
                }
            ),
            json.dumps(
                {
                    "method": "turn/completed",
                    "params": {
                        "turn": {
                            "id": "turn-123",
                            "status": "completed",
                        }
                    },
                }
            ),
        ]
    )
    progress_updates: list[project_chat.ChatTurnLiveEvent] = []

    monkeypatch.setattr(session, "_ensure_process", lambda: None)

    def fake_ensure_thread(model: str | None) -> None:
        session._thread_id = "thread-123"
        session._thread_initialized = True

    monkeypatch.setattr(session, "_ensure_thread", fake_ensure_thread)
    monkeypatch.setattr(
        session,
        "_send_request",
        lambda method, params: {"result": {"turn": {"id": "turn-123", "status": "inProgress", "items": []}}},
    )
    monkeypatch.setattr(session, "_read_line", lambda wait: next(lines, None))

    result = session.turn("hello", None, on_event=progress_updates.append)

    assert result.assistant_message == "I found the main entry points."
    assert any(
        update.kind == "reasoning_summary" and update.content_delta == "Scanning the repository structure."
        for update in progress_updates
    )


def test_chat_session_surfaces_reasoning_summary_text_deltas(monkeypatch) -> None:
    session = project_chat.CodexAppServerChatSession("/tmp/project")
    lines = iter(
        [
            json.dumps(
                {
                    "method": "item/reasoning/summaryTextDelta",
                    "params": {
                        "delta": "Draft draft that minimal proposal a best think",
                    },
                }
            ),
            json.dumps(
                {
                    "method": "item/agentMessage/delta",
                    "params": {
                        "delta": "I’m checking the project structure first.",
                    },
                }
            ),
            json.dumps(
                {
                    "method": "codex/event/task_complete",
                    "params": {
                        "msg": {
                            "last_agent_message": "I’m checking the project structure first.",
                        }
                    },
                }
            ),
            json.dumps(
                {
                    "method": "turn/completed",
                    "params": {
                        "turn": {
                            "id": "turn-123",
                            "status": "completed",
                        }
                    },
                }
            ),
        ]
    )
    progress_updates: list[project_chat.ChatTurnLiveEvent] = []

    monkeypatch.setattr(session, "_ensure_process", lambda: None)

    def fake_ensure_thread(model: str | None) -> None:
        session._thread_id = "thread-123"
        session._thread_initialized = True

    monkeypatch.setattr(session, "_ensure_thread", fake_ensure_thread)
    monkeypatch.setattr(
        session,
        "_send_request",
        lambda method, params: {"result": {"turn": {"id": "turn-123", "status": "inProgress", "items": []}}},
    )
    monkeypatch.setattr(session, "_read_line", lambda wait: next(lines, None))

    result = session.turn("hello", None, on_event=progress_updates.append)

    assert result.assistant_message == "I’m checking the project structure first."
    assert any(
        update.kind == "reasoning_summary"
        and update.content_delta == "Draft draft that minimal proposal a best think"
        for update in progress_updates
    )


def test_chat_session_initialize_enables_experimental_api(monkeypatch) -> None:
    session = project_chat.CodexAppServerChatSession("/tmp/project")
    requests: list[tuple[str, dict[str, object] | None]] = []

    class DummyStdout:
        pass

    class DummyProc:
        stdout = DummyStdout()

        def poll(self) -> int | None:
            return None

    class DummySelector:
        def register(self, fileobj, events) -> None:
            return None

    monkeypatch.setattr(project_chat.subprocess, "Popen", lambda *args, **kwargs: DummyProc())
    monkeypatch.setattr(project_chat.selectors, "DefaultSelector", lambda: DummySelector())

    def fake_send_request(method: str, params: dict[str, object] | None) -> dict[str, object]:
        requests.append((method, params))
        if method == "initialize":
            return {"result": {}}
        raise AssertionError(f"unexpected request: {method}")

    monkeypatch.setattr(session, "_send_request", fake_send_request)
    session._ensure_process()

    assert requests == [
        (
            "initialize",
            {
                "clientInfo": {"name": "sparkspawn", "version": "0.1"},
                "capabilities": {"experimentalApi": True},
            },
        )
    ]


def test_chat_session_turn_completes_on_turn_completed_after_final_answer(monkeypatch) -> None:
    session = project_chat.CodexAppServerChatSession("/tmp/project")
    lines = iter(
        [
            json.dumps(
                {
                    "method": "item/agentMessage/delta",
                    "params": {"delta": '{"assistant_message":"Ack"}'},
                }
            ),
            json.dumps(
                {
                    "method": "item/completed",
                    "params": {
                        "item": {
                            "type": "agentMessage",
                            "id": "msg-123",
                            "phase": "final_answer",
                            "text": '{"assistant_message":"Ack"}',
                        }
                    },
                }
            ),
            json.dumps(
                {
                    "method": "turn/completed",
                    "params": {
                        "turn": {
                            "id": "turn-123",
                            "status": "completed",
                        }
                    },
                }
            ),
        ]
    )

    monkeypatch.setattr(session, "_ensure_process", lambda: None)

    def fake_ensure_thread(model: str | None) -> None:
        session._thread_id = "thread-123"
        session._thread_initialized = True

    monkeypatch.setattr(session, "_ensure_thread", fake_ensure_thread)
    monkeypatch.setattr(
        session,
        "_send_request",
        lambda method, params: {"result": {"turn": {"id": "turn-123", "status": "inProgress", "items": []}}},
    )
    monkeypatch.setattr(session, "_read_line", lambda wait: next(lines, None))

    result = session.turn("hello", None)

    assert result.assistant_message == '{"assistant_message":"Ack"}'


def test_chat_session_turn_times_out_after_live_assistant_quiet_period_without_terminal_event(monkeypatch) -> None:
    session = project_chat.CodexAppServerChatSession("/tmp/project")
    lines = iter(
        [
            json.dumps(
                {
                    "method": "item/agentMessage/delta",
                    "params": {"delta": "{\"assistant_message\":\"Ack"},
                }
            ),
            json.dumps(
                {
                    "method": "item/agentMessage/delta",
                    "params": {"delta": "\"}"},
                }
            ),
            None,
        ]
    )
    monotonic_values = iter([0.0, 0.1, 0.2, 1.3])

    monkeypatch.setattr(session, "_ensure_process", lambda: None)

    def fake_ensure_thread(model: str | None) -> None:
        session._thread_id = "thread-123"
        session._thread_initialized = True

    monkeypatch.setattr(project_chat, "CHAT_TURN_IDLE_TIMEOUT_SECONDS", 1.0)
    monkeypatch.setattr(project_chat.time, "monotonic", lambda: next(monotonic_values))
    monkeypatch.setattr(session, "_ensure_thread", fake_ensure_thread)
    monkeypatch.setattr(
        session,
        "_send_request",
        lambda method, params: {"result": {"turn": {"id": "turn-123", "status": "inProgress", "items": []}}},
    )
    monkeypatch.setattr(session, "_read_line", lambda wait: next(lines, None))

    with pytest.raises(RuntimeError, match="timed out waiting for activity"):
        session.turn("hello", None)


def test_send_turn_retries_when_app_server_request_times_out_before_progress(tmp_path: Path, monkeypatch) -> None:
    service = project_chat.ProjectChatService(tmp_path)

    class TimedOutSession:
        def turn(
            self,
            prompt: str,
            model: str | None,
            *,
            on_event=None,
            on_dynamic_tool_call=None,
        ) -> project_chat.ChatTurnResult:
            raise RuntimeError("codex app-server request timed out waiting for response")

        def _close(self) -> None:
            return None

    class FreshSession:
        def turn(
            self,
            prompt: str,
            model: str | None,
            *,
            on_event=None,
            on_dynamic_tool_call=None,
        ) -> project_chat.ChatTurnResult:
            return project_chat.ChatTurnResult(
                assistant_message='{"assistant_message":"hi"}',
            )

        def _close(self) -> None:
            return None

    sessions = [TimedOutSession(), FreshSession()]
    monkeypatch.setattr(service, "_build_session", lambda conversation_id, project_path: sessions.pop(0))
    monkeypatch.setattr(
        service,
        "_replace_session",
        lambda conversation_id, project_path, persisted_thread_id=None: sessions.pop(0),
    )

    snapshot = service.send_turn("conversation-test", str(tmp_path), "hi", None)

    assert snapshot["turns"][-1]["role"] == "assistant"
    assert snapshot["turns"][-1]["content"] == "hi"


def test_send_turn_accepts_plain_text_final_response(tmp_path: Path, monkeypatch) -> None:
    service = project_chat.ProjectChatService(tmp_path)

    class PlainTextSession:
        def turn(
            self,
            prompt: str,
            model: str | None,
            *,
            on_event=None,
            on_dynamic_tool_call=None,
        ) -> project_chat.ChatTurnResult:
            return project_chat.ChatTurnResult(
                assistant_message="This looks like a Collatz implementation project.",
            )

        def _close(self) -> None:
            return None

    monkeypatch.setattr(service, "_build_session", lambda conversation_id, project_path: PlainTextSession())

    snapshot = service.send_turn("conversation-test", str(tmp_path), "What's this project about?", None)

    assert snapshot["turns"][-1]["role"] == "assistant"
    assert snapshot["turns"][-1]["status"] == "complete"
    assert snapshot["turns"][-1]["content"] == "This looks like a Collatz implementation project."


def test_send_turn_persists_spec_proposal_from_dynamic_tool_call(tmp_path: Path, monkeypatch) -> None:
    service = project_chat.ProjectChatService(tmp_path)

    class ToolCallingSession:
        def turn(
            self,
            prompt: str,
            model: str | None,
            *,
            on_event=None,
            on_dynamic_tool_call=None,
        ) -> project_chat.ChatTurnResult:
            assert on_dynamic_tool_call is not None
            tool_result = on_dynamic_tool_call(
                "draft_spec_proposal",
                {
                    "summary": "Reduce top-bar chrome and keep only project context.",
                    "changes": [
                        {
                            "path": "specs/ui-spec.md#home-header",
                            "before": "The home header surfaces execution controls and runtime metadata.",
                            "after": "The home header shows only navigation and active project context.",
                        }
                    ],
                },
                "call-1",
            )
            return project_chat.ChatTurnResult(
                assistant_message="I can tighten the top bar and relocate the overflow metadata.",
                spec_proposal_payloads=[tool_result.spec_proposal_payload] if tool_result.spec_proposal_payload else [],
            )

        def _close(self) -> None:
            return None

    monkeypatch.setattr(service, "_build_session", lambda conversation_id, project_path: ToolCallingSession())

    snapshot = service.send_turn(
        "conversation-test",
        str(tmp_path),
        "Let's clean up the top bar.",
        None,
    )

    assert snapshot["turns"][-1]["kind"] == "spec_edit_proposal"
    assert snapshot["turns"][-2]["role"] == "assistant"
    assert snapshot["turns"][-2]["content"] == "I can tighten the top bar and relocate the overflow metadata."
    assert snapshot["spec_edit_proposals"][0]["summary"] == "Reduce top-bar chrome and keep only project context."


def test_chat_session_ignores_duplicate_codex_agent_delta_channel(monkeypatch) -> None:
    session = project_chat.CodexAppServerChatSession("/tmp/project")
    lines = iter(
        [
            json.dumps(
                {
                    "method": "item/agentMessage/delta",
                    "params": {"delta": "{\"assistant_message\":\"Ack"},
                }
            ),
            json.dumps(
                {
                    "method": "codex/event/agent_message_delta",
                    "params": {"msg": {"delta": "{\"assistant_message\":\"Ack"}},
                }
            ),
            json.dumps(
                {
                    "method": "item/agentMessage/delta",
                    "params": {"delta": "\"}"},
                }
            ),
            json.dumps(
                {
                    "method": "codex/event/task_complete",
                    "params": {
                        "msg": {
                            "type": "task_complete",
                            "last_agent_message": '{"assistant_message":"Ack"}',
                        }
                    },
                }
            ),
            json.dumps(
                {
                    "method": "turn/completed",
                    "params": {
                        "turn": {
                            "id": "turn-123",
                            "status": "completed",
                        }
                    },
                }
            ),
        ]
    )
    progress_messages: list[str] = []

    monkeypatch.setattr(session, "_ensure_process", lambda: None)

    def fake_ensure_thread(model: str | None) -> None:
        session._thread_id = "thread-123"
        session._thread_initialized = True

    monkeypatch.setattr(session, "_ensure_thread", fake_ensure_thread)
    monkeypatch.setattr(
        session,
        "_send_request",
        lambda method, params: {"result": {"turn": {"id": "turn-123", "status": "inProgress", "items": []}}},
    )
    monkeypatch.setattr(session, "_read_line", lambda wait: next(lines, None))

    result = session.turn(
        "hello",
        None,
        on_event=lambda progress: progress_messages.append(progress.content_delta or "") if progress.kind == "assistant_delta" else None,
    )

    assert result.assistant_message == '{"assistant_message":"Ack"}'
    assert any(message == '{"assistant_message":"Ack' for message in progress_messages)
    assert all("assistantassistant" not in message for message in progress_messages)


def test_build_session_ignores_legacy_persisted_thread_without_session_version(tmp_path: Path, monkeypatch) -> None:
    service = project_chat.ProjectChatService(tmp_path)
    conversation_id = "conversation-test"
    project_path = str(tmp_path)
    conversation_root = ensure_project_paths(tmp_path, project_path).conversations_dir / conversation_id
    conversation_root.mkdir(parents=True, exist_ok=True)
    (conversation_root / "session.json").write_text(
        json.dumps(
            {
                "conversation_id": conversation_id,
                "updated_at": "2026-03-08T19:00:00Z",
                "project_path": project_path,
                "runtime_project_path": project_path,
                "thread_id": "legacy-thread",
            }
        ),
        encoding="utf-8",
    )

    captured: dict[str, object] = {}

    def fake_init(self, working_dir: str, *, persisted_thread_id=None, on_thread_id_updated=None):
        captured["working_dir"] = working_dir
        captured["persisted_thread_id"] = persisted_thread_id
        captured["on_thread_id_updated"] = on_thread_id_updated

    monkeypatch.setattr(project_chat.CodexAppServerChatSession, "__init__", fake_init)

    service._build_session(conversation_id, project_path)

    assert captured["working_dir"] == project_path
    assert captured["persisted_thread_id"] is None


def test_send_turn_retries_with_fresh_session_after_timeout(tmp_path: Path, monkeypatch) -> None:
    service = project_chat.ProjectChatService(tmp_path)

    class TimedOutSession:
        def turn(
            self,
            prompt: str,
            model: str | None,
            *,
            on_event=None,
            on_dynamic_tool_call=None,
        ) -> project_chat.ChatTurnResult:
            raise RuntimeError("codex app-server turn timed out waiting for activity")

        def _close(self) -> None:
            return None

    class FreshSession:
        def turn(
            self,
            prompt: str,
            model: str | None,
            *,
            on_event=None,
            on_dynamic_tool_call=None,
        ) -> project_chat.ChatTurnResult:
            return project_chat.ChatTurnResult(
                assistant_message='{"assistant_message":"Recovered."}',
            )

        def _close(self) -> None:
            return None

    sessions = [TimedOutSession(), FreshSession()]
    monkeypatch.setattr(service, "_build_session", lambda conversation_id, project_path: sessions.pop(0))
    monkeypatch.setattr(
        service,
        "_replace_session",
        lambda conversation_id, project_path, persisted_thread_id=None: sessions.pop(0),
    )

    snapshot = service.send_turn("conversation-test", str(tmp_path), "hello", None)

    assert snapshot["turns"][-1]["role"] == "assistant"
    assert snapshot["turns"][-1]["content"] == "Recovered."


def test_list_conversations_filters_by_project_and_sorts_latest_first(tmp_path: Path) -> None:
    service = project_chat.ProjectChatService(tmp_path)
    shared_project = str(tmp_path / "project-a")
    other_project = str(tmp_path / "project-b")

    service._write_state(
        project_chat.ConversationState(
            conversation_id="conversation-a",
            project_path=shared_project,
            title="First thread",
            created_at="2026-03-07T13:00:00Z",
            updated_at="2026-03-07T13:01:00Z",
            turns=[
                project_chat.ConversationTurn(
                    id="turn-a-1",
                    role="user",
                    content="First thread context",
                    timestamp="2026-03-07T13:01:00Z",
                )
            ],
        )
    )
    service._write_state(
        project_chat.ConversationState(
            conversation_id="conversation-b",
            project_path=shared_project,
            title="Second thread",
            created_at="2026-03-07T13:02:00Z",
            updated_at="2026-03-07T13:05:00Z",
            turns=[
                project_chat.ConversationTurn(
                    id="turn-b-1",
                    role="assistant",
                    content="Second thread context",
                    timestamp="2026-03-07T13:05:00Z",
                )
            ],
        )
    )
    service._write_state(
        project_chat.ConversationState(
            conversation_id="conversation-c",
            project_path=other_project,
            title="Other project thread",
            created_at="2026-03-07T13:03:00Z",
            updated_at="2026-03-07T13:04:00Z",
        )
    )

    summaries = service.list_conversations(shared_project)

    assert [summary["conversation_id"] for summary in summaries] == ["conversation-b", "conversation-a"]
    assert summaries[0]["title"] == "Second thread"
    assert summaries[0]["last_message_preview"] == "Second thread context"
    assert all(summary["project_path"] == shared_project for summary in summaries)


def test_send_project_conversation_turn_endpoint_uses_real_service_signature(
    api_client,
    monkeypatch,
    tmp_path: Path,
) -> None:
    service = server.PROJECT_CHAT
    partial_snapshots: list[dict[str, object]] = []

    class FakeSession:
        def turn(
            self,
            prompt: str,
            model: str | None,
            *,
            on_event=None,
            on_dynamic_tool_call=None,
        ) -> project_chat.ChatTurnResult:
            if on_event is not None:
                on_event(
                    project_chat.ChatTurnLiveEvent(
                        kind="reasoning_summary",
                        content_delta="Checking whether a spec proposal makes sense.",
                    )
                )
                on_event(project_chat.ChatTurnLiveEvent(kind="assistant_delta", content_delta="Working on it"))
                on_event(
                    project_chat.ChatTurnLiveEvent(
                        kind="tool_call_started",
                        tool_call_id="call-pwd",
                        tool_call=project_chat.ToolCallRecord(
                            id="call-pwd",
                            kind="command_execution",
                            status="running",
                            title="Run command",
                            command="pwd",
                        ),
                    )
                )
                partial_snapshots.append(service.get_snapshot("conversation-test", str(tmp_path)))
            return project_chat.ChatTurnResult(
                assistant_message='{"assistant_message":"ACK","spec_proposal":null}',
            )

    monkeypatch.setattr(service, "_build_session", lambda conversation_id, project_path: FakeSession())

    response = api_client.post(
        "/api/conversations/conversation-test/turns",
        json={
            "project_path": str(tmp_path),
            "message": "hello",
            "model": "gpt-test",
        },
    )

    assert response.status_code == 200
    payload = response.json()
    assert payload["conversation_id"] == "conversation-test"
    assert [turn["role"] for turn in payload["turns"]] == ["user", "assistant"]
    assert payload["turns"][1]["content"] == "ACK"
    assert payload["turns"][1]["status"] == "complete"
    assert [event["kind"] for event in payload["turn_events"]] == [
        "reasoning_summary",
        "tool_call_started",
        "assistant_completed",
    ]
    assert partial_snapshots
    assert [turn["role"] for turn in partial_snapshots[-1]["turns"]] == ["user", "assistant"]
    assert partial_snapshots[-1]["turns"][1]["content"] == ""
    assert partial_snapshots[-1]["turns"][1]["status"] == "streaming"
    assert [event["kind"] for event in partial_snapshots[-1]["turn_events"]] == [
        "reasoning_summary",
        "tool_call_started",
    ]


def test_snapshot_compacts_streamed_assistant_deltas(tmp_path: Path) -> None:
    service = project_chat.ProjectChatService(tmp_path)
    project_paths = ensure_project_paths(tmp_path, str(tmp_path))
    state = project_chat.ConversationState(
        conversation_id="conversation-compact",
        project_path=str(tmp_path),
        title="Compact thread",
        created_at="2026-03-07T18:00:00Z",
        updated_at="2026-03-07T18:00:03Z",
        turns=[
            project_chat.ConversationTurn(
                id="turn-user-1",
                role="user",
                content="hi",
                timestamp="2026-03-07T18:00:00Z",
                status="complete",
            ),
            project_chat.ConversationTurn(
                id="turn-assistant-1",
                role="assistant",
                content="hello",
                timestamp="2026-03-07T18:00:03Z",
                status="complete",
                parent_turn_id="turn-user-1",
            ),
        ],
        turn_events=[
            project_chat.ConversationTurnEvent(
                id="event-assistant-delta-1",
                turn_id="turn-assistant-1",
                sequence=1,
                timestamp="2026-03-07T18:00:01Z",
                kind="assistant_delta",
                content_delta="hel",
            ),
            project_chat.ConversationTurnEvent(
                id="event-reasoning-1",
                turn_id="turn-assistant-1",
                sequence=2,
                timestamp="2026-03-07T18:00:01Z",
                kind="reasoning_summary",
                content_delta="Thinking about the repository structure.",
            ),
            project_chat.ConversationTurnEvent(
                id="event-assistant-delta-2",
                turn_id="turn-assistant-1",
                sequence=3,
                timestamp="2026-03-07T18:00:02Z",
                kind="assistant_delta",
                content_delta="lo",
            ),
            project_chat.ConversationTurnEvent(
                id="event-assistant-completed-1",
                turn_id="turn-assistant-1",
                sequence=4,
                timestamp="2026-03-07T18:00:03Z",
                kind="assistant_completed",
                message="Assistant turn completed.",
            ),
        ],
    )
    service._write_state(state)

    snapshot = service.get_snapshot("conversation-compact", str(tmp_path))

    assert [event["kind"] for event in snapshot["turn_events"]] == ["reasoning_summary", "assistant_completed"]
    persisted = json.loads(
        (project_paths.conversations_dir / "conversation-compact" / "state.json").read_text(encoding="utf-8")
    )
    assert [event["kind"] for event in persisted["turn_events"]] == ["reasoning_summary", "assistant_completed"]


def test_list_project_conversations_endpoint_returns_project_threads(api_client, tmp_path: Path) -> None:
    service = server.PROJECT_CHAT
    service._write_state(
        project_chat.ConversationState(
            conversation_id="conversation-a",
            project_path=str(tmp_path),
            title="Design thread",
            created_at="2026-03-07T14:00:00Z",
            updated_at="2026-03-07T14:02:00Z",
            turns=[
                project_chat.ConversationTurn(
                    id="turn-a-1",
                    role="user",
                    content="Design thread preview",
                    timestamp="2026-03-07T14:02:00Z",
                )
            ],
        )
    )

    response = api_client.get("/api/projects/conversations", params={"project_path": str(tmp_path)})

    assert response.status_code == 200
    payload = response.json()
    assert [entry["conversation_id"] for entry in payload] == ["conversation-a"]
    assert payload[0]["title"] == "Design thread"
    assert payload[0]["last_message_preview"] == "Design thread preview"


def test_delete_project_conversation_endpoint_removes_thread_state(api_client, tmp_path: Path) -> None:
    service = server.PROJECT_CHAT
    conversation_id = "conversation-delete-me"
    project_paths = ensure_project_paths(tmp_path / ".sparkspawn", str(tmp_path))
    service._write_state(
        project_chat.ConversationState(
            conversation_id=conversation_id,
            project_path=str(tmp_path),
            title="Delete me",
            created_at="2026-03-07T14:00:00Z",
            updated_at="2026-03-07T14:02:00Z",
        )
    )
    service._write_session_state(
        project_chat.ConversationSessionState(
            conversation_id=conversation_id,
            updated_at="2026-03-07T14:02:00Z",
            project_path=str(tmp_path),
            runtime_project_path=str(tmp_path),
            thread_id="thread-delete-me",
        )
    )

    response = api_client.delete(
        f"/api/conversations/{conversation_id}",
        params={"project_path": str(tmp_path)},
    )

    assert response.status_code == 200
    assert response.json() == {
        "status": "deleted",
        "conversation_id": conversation_id,
        "project_path": str(tmp_path.resolve()),
    }
    assert not (project_paths.conversations_dir / conversation_id).exists()

    list_response = api_client.get("/api/projects/conversations", params={"project_path": str(tmp_path)})
    assert list_response.status_code == 200
    assert list_response.json() == []
