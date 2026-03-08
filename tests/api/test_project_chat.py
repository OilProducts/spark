from __future__ import annotations

import json
from pathlib import Path

import attractor.api.server as server
import attractor.api.project_chat as project_chat


def test_extract_command_text_handles_list_and_string_payloads() -> None:
    assert project_chat._extract_command_text({"command": ["git", "status", "--short"]}) == "git status --short"
    assert project_chat._extract_command_text({"commandLine": "npm test"}) == "npm test"


def test_extract_live_assistant_message_handles_partial_and_complete_json() -> None:
    assert project_chat._extract_live_assistant_message(
        '{"assistant_message":"Hello.","spec_proposal":null}'
    ) == "Hello."
    assert project_chat._extract_live_assistant_message(
        '{"assistant_message":"Hello'
    ) == "Hello"
    assert project_chat._extract_live_assistant_message("plain text") == "plain text"


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


def test_should_draft_spec_proposal_uses_change_intent_not_informational_queries() -> None:
    assert project_chat._should_draft_spec_proposal("Let's clean up the top bar.")
    assert project_chat._should_draft_spec_proposal("We should rename this panel.")
    assert not project_chat._should_draft_spec_proposal("What's this project about?")
    assert not project_chat._should_draft_spec_proposal("Summarize the tools you have available.")


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


def test_chat_session_turn_completes_on_task_complete_event(monkeypatch) -> None:
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


def test_chat_session_turn_completes_after_final_answer_quiet_period(monkeypatch) -> None:
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
            None,
            None,
        ]
    )
    monotonic_values = iter([0.0, 0.0, 0.0, 0.0, 0.5, 1.5, 1.5])

    monkeypatch.setattr(session, "_ensure_process", lambda: None)

    def fake_ensure_thread(model: str | None) -> None:
        session._thread_id = "thread-123"
        session._thread_initialized = True

    monkeypatch.setattr(project_chat.time, "monotonic", lambda: next(monotonic_values))
    monkeypatch.setattr(session, "_ensure_thread", fake_ensure_thread)
    monkeypatch.setattr(
        session,
        "_send_request",
        lambda method, params: {"result": {"turn": {"id": "turn-123", "status": "inProgress", "items": []}}},
    )
    monkeypatch.setattr(session, "_read_line", lambda wait: next(lines, None))

    result = session.turn("hello", None)

    assert result.assistant_message == '{"assistant_message":"Ack"}'


def test_chat_session_turn_completes_after_live_assistant_quiet_period(monkeypatch) -> None:
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
            None,
        ]
    )
    monotonic_values = iter([0.0, 0.0, 0.4, 0.4, 0.8, 3.3, 3.3, 3.3])

    monkeypatch.setattr(session, "_ensure_process", lambda: None)

    def fake_ensure_thread(model: str | None) -> None:
        session._thread_id = "thread-123"
        session._thread_initialized = True

    monkeypatch.setattr(project_chat.time, "monotonic", lambda: next(monotonic_values))
    monkeypatch.setattr(session, "_ensure_thread", fake_ensure_thread)
    monkeypatch.setattr(
        session,
        "_send_request",
        lambda method, params: {"result": {"turn": {"id": "turn-123", "status": "inProgress", "items": []}}},
    )
    monkeypatch.setattr(session, "_read_line", lambda wait: next(lines, None))

    result = session.turn("hello", None)

    assert result.assistant_message == '{"assistant_message":"Ack"}'


def test_send_turn_retries_when_app_server_request_times_out_before_progress(tmp_path: Path, monkeypatch) -> None:
    service = project_chat.ProjectChatService(tmp_path)

    class TimedOutSession:
        def turn(self, prompt: str, model: str | None, *, on_progress=None) -> project_chat.ChatTurnResult:
            raise RuntimeError("codex app-server request timed out waiting for response")

        def _close(self) -> None:
            return None

    class FreshSession:
        def turn(self, prompt: str, model: str | None, *, on_progress=None) -> project_chat.ChatTurnResult:
            return project_chat.ChatTurnResult(
                assistant_message='{"assistant_message":"hi"}',
                tool_calls=[],
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
        def turn(self, prompt: str, model: str | None, *, on_progress=None) -> project_chat.ChatTurnResult:
            return project_chat.ChatTurnResult(
                assistant_message="This looks like a Collatz implementation project.",
                tool_calls=[],
            )

        def _close(self) -> None:
            return None

    monkeypatch.setattr(service, "_build_session", lambda conversation_id, project_path: PlainTextSession())

    snapshot = service.send_turn("conversation-test", str(tmp_path), "What's this project about?", None)

    assert snapshot["turns"][-1]["role"] == "assistant"
    assert snapshot["turns"][-1]["status"] == "complete"
    assert snapshot["turns"][-1]["content"] == "This looks like a Collatz implementation project."


def test_send_turn_drafts_spec_proposal_separately_from_plain_text_reply(tmp_path: Path, monkeypatch) -> None:
    service = project_chat.ProjectChatService(tmp_path)

    class PlainTextSession:
        def turn(self, prompt: str, model: str | None, *, on_progress=None) -> project_chat.ChatTurnResult:
            return project_chat.ChatTurnResult(
                assistant_message="I can tighten the top bar and relocate the overflow metadata.",
                tool_calls=[],
            )

        def _close(self) -> None:
            return None

    monkeypatch.setattr(service, "_build_session", lambda conversation_id, project_path: PlainTextSession())

    snapshot = service.send_turn(
        "conversation-test",
        str(tmp_path),
        "Let's clean up the top bar.",
        None,
        spec_proposal_drafter=lambda prompt, model: json.dumps(
            {
                "summary": "Reduce top-bar chrome and keep only project context.",
                "changes": [
                    {
                        "path": "specs/ui-spec.md#home-header",
                        "before": "The home header surfaces execution controls and runtime metadata.",
                        "after": "The home header shows only navigation and active project context.",
                    }
                ],
            }
        ),
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
        on_progress=lambda progress: progress_messages.append(progress.assistant_message),
    )

    assert result.assistant_message == '{"assistant_message":"Ack"}'
    assert any(message == '{"assistant_message":"Ack' for message in progress_messages)
    assert all("assistantassistant" not in message for message in progress_messages)


def test_send_turn_retries_with_fresh_session_after_timeout(tmp_path: Path, monkeypatch) -> None:
    service = project_chat.ProjectChatService(tmp_path)

    class TimedOutSession:
        def turn(self, prompt: str, model: str | None, *, on_progress=None) -> project_chat.ChatTurnResult:
            raise RuntimeError("codex app-server turn timed out waiting for activity")

        def _close(self) -> None:
            return None

    class FreshSession:
        def turn(self, prompt: str, model: str | None, *, on_progress=None) -> project_chat.ChatTurnResult:
            return project_chat.ChatTurnResult(
                assistant_message='{"assistant_message":"Recovered."}',
                tool_calls=[],
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
        def turn(self, prompt: str, model: str | None, *, on_progress=None) -> project_chat.ChatTurnResult:
            if on_progress is not None:
                on_progress(
                    project_chat.ChatTurnResult(
                        assistant_message='{"assistant_message":"Working on it","spec_proposal":null}',
                        tool_calls=[
                            project_chat.ToolCallRecord(
                                id="call-pwd",
                                kind="command_execution",
                                status="running",
                                title="Run command",
                                command="pwd",
                            )
                        ],
                    )
                )
                partial_snapshots.append(service.get_snapshot("conversation-test", str(tmp_path)))
            return project_chat.ChatTurnResult(
                assistant_message='{"assistant_message":"ACK","spec_proposal":null}',
                tool_calls=[
                    project_chat.ToolCallRecord(
                        id="call-pwd",
                        kind="command_execution",
                        status="completed",
                        title="Run command",
                        command="pwd",
                        output=str(tmp_path),
                    )
                ],
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
        "tool_call_started",
        "assistant_completed",
    ]
    assert partial_snapshots
    assert [turn["role"] for turn in partial_snapshots[-1]["turns"]] == ["user", "assistant"]
    assert partial_snapshots[-1]["turns"][1]["content"] == "Working on it"
    assert partial_snapshots[-1]["turns"][1]["status"] == "streaming"
    assert [event["kind"] for event in partial_snapshots[-1]["turn_events"]] == [
        "tool_call_started",
    ]


def test_snapshot_compacts_streamed_assistant_deltas(tmp_path: Path) -> None:
    service = project_chat.ProjectChatService(tmp_path)
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
                id="event-assistant-delta-2",
                turn_id="turn-assistant-1",
                sequence=2,
                timestamp="2026-03-07T18:00:02Z",
                kind="assistant_delta",
                content_delta="lo",
            ),
            project_chat.ConversationTurnEvent(
                id="event-assistant-completed-1",
                turn_id="turn-assistant-1",
                sequence=3,
                timestamp="2026-03-07T18:00:03Z",
                kind="assistant_completed",
                message="Assistant turn completed.",
            ),
        ],
    )
    service._write_state(state)

    snapshot = service.get_snapshot("conversation-compact", str(tmp_path))

    assert [event["kind"] for event in snapshot["turn_events"]] == ["assistant_completed"]
    persisted = json.loads((tmp_path / "conversations" / "conversation-compact" / "state.json").read_text(encoding="utf-8"))
    assert [event["kind"] for event in persisted["turn_events"]] == ["assistant_completed"]


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
    assert not (tmp_path / "conversations" / conversation_id).exists()

    list_response = api_client.get("/api/projects/conversations", params={"project_path": str(tmp_path)})
    assert list_response.status_code == 200
    assert list_response.json() == []
