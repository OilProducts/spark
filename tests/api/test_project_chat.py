from __future__ import annotations

import json
import threading
import time
from pathlib import Path

import pytest

import attractor.api.server as server
import spark_common.codex_app_server as codex_app_server
import spark_common.process_line_reader as process_line_reader
from spark.authoring_assets import (
    attractor_spec_path,
    dot_authoring_guide_path,
    flow_extensions_spec_path,
)
import workspace.project_chat as project_chat
import workspace.project_chat_models as project_chat_models
import workspace.project_chat_session as project_chat_session
import workspace.attractor_client as attractor_client
from workspace.prompt_templates import PROMPTS_FILE_NAME
from workspace.storage import conversation_handles_path, ensure_project_paths


def test_extract_command_text_handles_list_and_string_payloads() -> None:
    assert codex_app_server.extract_command_text({"command": ["git", "status", "--short"]}) == "git status --short"
    assert codex_app_server.extract_command_text({"commandLine": "npm test"}) == "npm test"


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
    assert service._prompt_templates.chat
    assert service._prompt_templates.execution_planning


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
        conversation_handle="amber-otter",
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
            changes=[project_chat_models.SpecEditProposalChange(path="specs/spark-ui-ux.md", before="old", after="new")],
            status="approved",
        ),
        "Needs refinement",
    )

    assert "Conversation handle: amber-otter" in chat_prompt
    assert "CHAT /tmp/project :: Latest message :: USER: Older message" in chat_prompt
    assert '"id": "proposal-1"' in execution_prompt
    assert execution_prompt.endswith(":: Needs refinement")


def test_project_chat_prompt_includes_flow_authoring_boundary(tmp_path: Path) -> None:
    service = project_chat.ProjectChatService(tmp_path)
    state = project_chat.ConversationState(
        conversation_id="conversation-flow-authoring",
        project_path="/tmp/project",
        conversation_handle="amber-otter",
    )

    prompt = service._build_chat_prompt(state, "Create a flow that drafts and reviews an email.")

    assert f"flow library at `{(tmp_path / 'flows').resolve(strict=False)}`" in prompt
    assert f"`{dot_authoring_guide_path()}`" in prompt
    assert f"`{flow_extensions_spec_path()}`" in prompt
    assert f"`{attractor_spec_path()}`" in prompt
    assert "spark flow validate --flow <name> --text" in prompt


def test_project_chat_service_rejects_malformed_prompt_templates(tmp_path: Path) -> None:
    prompts_path = tmp_path / "config" / PROMPTS_FILE_NAME
    prompts_path.parent.mkdir(parents=True, exist_ok=True)
    prompts_path.write_text("[project_chat]\nchat = '''unterminated\n", encoding="utf-8")

    with pytest.raises(RuntimeError, match="Invalid prompt templates file"):
        project_chat.ProjectChatService(tmp_path)


def test_project_chat_service_rejects_prompt_templates_missing_required_keys(tmp_path: Path) -> None:
    prompts_path = tmp_path / "config" / PROMPTS_FILE_NAME
    prompts_path.parent.mkdir(parents=True, exist_ok=True)
    prompts_path.write_text(
        "\n".join(
            [
                "[project_chat]",
                "chat = '''CHAT {{project_path}}'''",
                "",
            ]
        ),
        encoding="utf-8",
    )

    with pytest.raises(RuntimeError, match="missing required templates"):
        project_chat.ProjectChatService(tmp_path)


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

    assert codex_app_server.extract_file_paths(payload) == [
        "frontend/src/components/ProjectsPanel.tsx",
        "frontend/src/lib/apiClient.ts",
        "frontend/src/store.ts",
    ]


def test_append_tool_output_keeps_latest_tail() -> None:
    output = codex_app_server.append_tool_output("abc", "def", limit=4)

    assert output == "cdef"


def test_process_turn_message_normalizes_item_reasoning_delta_by_item_and_summary_index() -> None:
    state = codex_app_server.CodexAppServerTurnState()
    events = codex_app_server.process_turn_message(
        {
            "method": "item/reasoning/summaryTextDelta",
            "params": {
                "itemId": "rs-1",
                "summaryIndex": 0,
                "delta": "**Summ",
            },
        },
        state,
    )

    assert len(events) == 1
    assert events[0].kind == "reasoning_delta"
    assert events[0].text == "**Summ"
    assert events[0].item_id == "rs-1"
    assert events[0].summary_index == 0


def test_tool_call_from_command_execution_item_uses_completed_payload() -> None:
    tool_call = project_chat_session._tool_call_from_item(
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
    tool_call = project_chat_session._tool_call_from_item(
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


def test_build_segment_upsert_payload_serializes_segment(tmp_path: Path) -> None:
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
    segment = project_chat.ConversationSegment(
        id="segment-tool-app-turn-1-call-1",
        turn_id="turn-assistant-1",
        order=1,
        kind="tool_call",
        role="system",
        status="completed",
        timestamp="2026-03-06T23:00:02Z",
        updated_at="2026-03-06T23:00:03Z",
        completed_at="2026-03-06T23:00:03Z",
        tool_call=project_chat.ToolCallRecord(
            id="call-1",
            kind="command_execution",
            status="completed",
            title="Run command",
            command="ls -1 /app",
            output="AGENTS.md\n",
        ),
        source=project_chat.ConversationSegmentSource(app_turn_id="app-turn-1", item_id="call-1"),
    )
    payload = service._build_segment_upsert_payload(state, segment)

    assert payload["type"] == "segment_upsert"
    assert payload["conversation_id"] == "conversation-test"
    assert payload["segment"]["id"] == "segment-tool-app-turn-1-call-1"
    assert payload["segment"]["tool_call"]["output"] == "AGENTS.md\n"


def test_conversation_state_rejects_unsupported_snapshot_shape() -> None:
    with pytest.raises(ValueError, match="Unsupported conversation state schema"):
        project_chat.ConversationState.from_dict(
            {
                "conversation_id": "conversation-test",
                "project_path": "/tmp/project",
                "title": "Legacy reasoning stream",
                "turns": [],
                "turn_events": [],
            }
        )


def test_list_conversations_skips_invalid_local_state_files(tmp_path: Path) -> None:
    service = project_chat.ProjectChatService(tmp_path)
    project_path = str(tmp_path.resolve())
    service._write_state(
        project_chat.ConversationState(
            conversation_id="conversation-valid",
            project_path=project_path,
            title="Valid thread",
            created_at="2026-03-13T10:00:00Z",
            updated_at="2026-03-13T10:01:00Z",
            turns=[
                project_chat.ConversationTurn(
                    id="turn-user-1",
                    role="user",
                    content="Latest valid thread",
                    timestamp="2026-03-13T10:00:00Z",
                ),
            ],
        )
    )
    invalid_state_path = service._conversation_state_path("conversation-invalid", project_path)
    invalid_state_path.parent.mkdir(parents=True, exist_ok=True)
    invalid_state_path.write_text(
        json.dumps(
            {
                "conversation_id": "conversation-invalid",
                "project_path": project_path,
                "title": "Invalid thread",
                "turns": [],
            }
        ),
        encoding="utf-8",
    )

    summaries = service.list_conversations(project_path)

    assert [summary["conversation_id"] for summary in summaries] == ["conversation-valid"]


def test_materialize_segment_for_live_event_completes_matching_assistant_item_by_item_id(tmp_path: Path) -> None:
    service = project_chat.ProjectChatService(tmp_path)
    state = project_chat.ConversationState(
        conversation_id="conversation-test",
        project_path="/tmp/project",
        turns=[
            project_chat.ConversationTurn(
                id="turn-user-1",
                role="user",
                content="Walk me through it.",
                timestamp="2026-03-13T10:00:00Z",
            ),
            project_chat.ConversationTurn(
                id="turn-assistant-1",
                role="assistant",
                content="",
                timestamp="2026-03-13T10:00:01Z",
                status="streaming",
                parent_turn_id="turn-user-1",
            ),
        ],
    )
    assistant_turn = state.turns[-1]

    commentary_delta = project_chat.ChatTurnLiveEvent(
        kind="assistant_delta",
        content_delta="I’m checking the prompt template path.",
        app_turn_id="app-turn-1",
        item_id="item-msg-1",
        phase="commentary",
    )
    final_delta = project_chat.ChatTurnLiveEvent(
        kind="assistant_delta",
        content_delta="Here is the final grounded answer.",
        app_turn_id="app-turn-1",
        item_id="item-msg-2",
        phase="final_answer",
    )

    service._materialize_segment_for_live_event(state, assistant_turn, commentary_delta)
    service._materialize_segment_for_live_event(state, assistant_turn, final_delta)

    commentary_complete = project_chat.ChatTurnLiveEvent(
        kind="assistant_completed",
        content_delta="I’m checking the prompt template path.",
        app_turn_id="app-turn-1",
        item_id="item-msg-1",
        phase="commentary",
    )
    final_complete = project_chat.ChatTurnLiveEvent(
        kind="assistant_completed",
        content_delta="Here is the final grounded answer.",
        app_turn_id="app-turn-1",
        item_id="item-msg-2",
        phase="final_answer",
    )

    service._materialize_segment_for_live_event(state, assistant_turn, commentary_complete)
    service._materialize_segment_for_live_event(state, assistant_turn, final_complete)

    assistant_segments = [segment for segment in state.segments if segment.kind == "assistant_message"]

    assert [segment.id for segment in assistant_segments] == [
        "segment-assistant-app-turn-1-item-msg-1",
        "segment-assistant-app-turn-1-item-msg-2",
    ]
    assert [segment.phase for segment in assistant_segments] == ["commentary", "final_answer"]
    assert [segment.status for segment in assistant_segments] == ["complete", "complete"]
    assert [segment.content for segment in assistant_segments] == [
        "I’m checking the prompt template path.",
        "Here is the final grounded answer.",
    ]


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


def test_chat_session_turn_drains_notifications_queued_during_turn_start_response(monkeypatch) -> None:
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
                    "jsonrpc": "2.0",
                    "id": 1,
                    "result": {
                        "turn": {
                            "id": "turn-123",
                            "status": "inProgress",
                            "items": [],
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
    monkeypatch.setattr(session, "_send_json", lambda payload: None)
    session._proc = type("DummyProc", (), {"poll": lambda self: None})()

    def fake_ensure_thread(model: str | None) -> None:
        session._thread_id = "thread-123"
        session._thread_initialized = True

    monkeypatch.setattr(session, "_ensure_thread", fake_ensure_thread)
    monkeypatch.setattr(session, "_read_line", lambda wait: next(lines, None))

    result = session.turn("hello", None)

    assert result.assistant_message == '{"assistant_message":"Ack"}'
    assert not session._pending_messages


def test_chat_session_turn_times_out_after_final_answer_without_turn_completed(monkeypatch) -> None:
    session = project_chat.CodexAppServerChatSession("/tmp/project")
    lines = iter(
        [
            json.dumps(
                {
                    "method": "item/agentMessage/delta",
                    "params": {"delta": "Ack", "itemId": "msg-1"},
                }
            ),
            json.dumps(
                {
                    "method": "item/completed",
                    "params": {
                        "item": {
                            "type": "agentMessage",
                            "id": "msg-1",
                            "phase": "final_answer",
                            "text": "Ack",
                        }
                    },
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

    monkeypatch.setattr(project_chat_session, "CHAT_TURN_IDLE_TIMEOUT_SECONDS", 1.0)
    monkeypatch.setattr(project_chat_session.time, "monotonic", lambda: next(monotonic_values))
    monkeypatch.setattr(session, "_ensure_thread", fake_ensure_thread)
    monkeypatch.setattr(
        session,
        "_send_request",
        lambda method, params: {"result": {"turn": {"id": "turn-123", "status": "inProgress", "items": []}}},
    )
    monkeypatch.setattr(session, "_read_line", lambda wait: next(lines, None))

    with pytest.raises(RuntimeError, match="timed out waiting for activity"):
        session.turn("hello", None)


def test_chat_session_raw_rpc_logger_records_transport_lines(monkeypatch) -> None:
    session = project_chat.CodexAppServerChatSession("/tmp/project")
    outgoing_lines: list[str] = []
    captured: list[tuple[str, str]] = []

    class DummyStdin:
        def write(self, text: str) -> None:
            outgoing_lines.append(text)

        def flush(self) -> None:
            return None

    class DummyProc:
        stdin = DummyStdin()

        def poll(self) -> int | None:
            return None

    session._proc = DummyProc()
    session.set_raw_rpc_logger(lambda direction, line: captured.append((direction, line)))

    session._send_json({"jsonrpc": "2.0", "id": 7, "method": "ping"})
    monkeypatch.setattr(session, "_read_line", lambda wait: '{"jsonrpc":"2.0","id":7,"result":{}}')

    response = session._wait_for_response(7)

    assert outgoing_lines == ['{"jsonrpc": "2.0", "id": 7, "method": "ping"}\n']
    assert response == {"jsonrpc": "2.0", "id": 7, "result": {}}
    assert captured == [
        ("outgoing", '{"jsonrpc": "2.0", "id": 7, "method": "ping"}'),
        ("incoming", '{"jsonrpc":"2.0","id":7,"result":{}}'),
    ]


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
                    "method": "item/completed",
                    "params": {
                        "item": {
                            "type": "agentMessage",
                            "id": "msg-123",
                            "phase": "final_answer",
                            "text": "I found the main entry points.",
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
                    "method": "item/completed",
                    "params": {
                        "item": {
                            "type": "agentMessage",
                            "id": "msg-123",
                            "phase": "final_answer",
                            "text": "I’m checking the project structure first.",
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


def test_process_line_reader_drains_buffered_lines_in_order() -> None:
    class FakeStdout:
        def __init__(self, lines: list[str]) -> None:
            self._lines = list(lines)

        def readline(self) -> str:
            if not self._lines:
                return ""
            return self._lines.pop(0)

    reader = process_line_reader.ProcessLineReader(FakeStdout(["one\n", "two\n"]))

    assert reader.read_line(0.1) == "one"
    assert reader.read_line(0.1) == "two"
    assert reader.read_line(0.1) is None


def test_chat_session_initialize_uses_stable_api_surface(monkeypatch) -> None:
    session = project_chat.CodexAppServerChatSession("/tmp/project")
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

    monkeypatch.setattr(project_chat_session.subprocess, "Popen", lambda *args, **kwargs: DummyProc())
    monkeypatch.setattr(session, "_send_json", lambda payload: notifications.append(payload))

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
                "clientInfo": {"name": "spark", "version": "0.1"},
            },
        )
    ]
    assert notifications == [{"jsonrpc": "2.0", "method": "initialized", "params": {}}]


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


def test_chat_session_turn_ignores_non_matching_turn_completed(monkeypatch) -> None:
    session = project_chat.CodexAppServerChatSession("/tmp/project")
    lines = iter(
        [
            json.dumps(
                {
                    "method": "item/agentMessage/delta",
                    "params": {"delta": "Ack", "itemId": "msg-123"},
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
                            "text": "Ack",
                        }
                    },
                }
            ),
            json.dumps(
                {
                    "method": "turn/completed",
                    "params": {
                        "turn": {
                            "id": "turn-stale",
                            "status": "completed",
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

    assert result.assistant_message == "Ack"


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

    monkeypatch.setattr(project_chat_session, "CHAT_TURN_IDLE_TIMEOUT_SECONDS", 1.0)
    monkeypatch.setattr(project_chat_session.time, "monotonic", lambda: next(monotonic_values))
    monkeypatch.setattr(session, "_ensure_thread", fake_ensure_thread)
    monkeypatch.setattr(
        session,
        "_send_request",
        lambda method, params: {"result": {"turn": {"id": "turn-123", "status": "inProgress", "items": []}}},
    )
    monkeypatch.setattr(session, "_read_line", lambda wait: next(lines, None))

    with pytest.raises(RuntimeError, match="timed out waiting for activity"):
        session.turn("hello", None)


def test_send_turn_marks_assistant_failed_after_timeout_without_retry(tmp_path: Path, monkeypatch) -> None:
    service = project_chat.ProjectChatService(tmp_path)
    calls: list[str] = []

    class TimeoutSession:
        def turn(
            self,
            prompt: str,
            model: str | None,
            *,
            on_event=None,
            on_dynamic_tool_call=None,
        ) -> project_chat.ChatTurnResult:
            calls.append(prompt)
            if on_event is not None:
                on_event(
                    project_chat.ChatTurnLiveEvent(
                        kind="assistant_completed",
                        content_delta="hi",
                        app_turn_id="app-turn-1",
                        item_id="msg-1",
                        phase="final_answer",
                    )
                )
            raise RuntimeError("codex app-server turn timed out waiting for activity")

        def _close(self) -> None:
            return None

    monkeypatch.setattr(service, "_build_session", lambda conversation_id, project_path: TimeoutSession())

    with pytest.raises(RuntimeError, match="timed out waiting for activity"):
        service.send_turn("conversation-test", str(tmp_path), "hi", None)

    state = service._read_state("conversation-test", str(tmp_path))
    assert state is not None
    assert len(calls) == 1
    assert "Latest user message:\nhi" in calls[0]
    assert state.turns[-1].role == "assistant"
    assert state.turns[-1].status == "failed"
    assert state.turns[-1].error == "codex app-server turn timed out waiting for activity"
    assistant_segments = [segment for segment in state.segments if segment.turn_id == state.turns[-1].id]
    assert len(assistant_segments) == 1
    assert assistant_segments[0].kind == "assistant_message"
    assert assistant_segments[0].status == "failed"
    assert assistant_segments[0].content == "hi"


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
            if on_event is not None:
                on_event(
                    project_chat.ChatTurnLiveEvent(
                        kind="assistant_completed",
                        content_delta="This looks like a Collatz implementation project.",
                        app_turn_id="app-turn-1",
                        item_id="msg-1",
                        phase="final_answer",
                    )
                )
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


def test_create_spec_edit_proposal_places_artifact_on_latest_assistant_turn(tmp_path: Path) -> None:
    service = project_chat.ProjectChatService(tmp_path)
    conversation_id = "conversation-test"
    project_path = str(tmp_path.resolve())
    assistant_turn_older = project_chat.ConversationTurn(
        id="turn-assistant-older",
        role="assistant",
        content="Older assistant reply.",
        timestamp="2026-03-13T10:00:00Z",
        status="complete",
    )
    assistant_turn_newer = project_chat.ConversationTurn(
        id="turn-assistant-newer",
        role="assistant",
        content="Latest assistant reply.",
        timestamp="2026-03-13T10:02:00Z",
        status="complete",
    )
    service._write_state(
        project_chat.ConversationState(
            conversation_id=conversation_id,
            project_path=project_path,
            title="Proposal placement",
            created_at="2026-03-13T10:00:00Z",
            updated_at="2026-03-13T10:02:00Z",
            turns=[
                project_chat.ConversationTurn(
                    id="turn-user-1",
                    role="user",
                    content="First request",
                    timestamp="2026-03-13T09:59:00Z",
                ),
                assistant_turn_older,
                project_chat.ConversationTurn(
                    id="turn-user-2",
                    role="user",
                    content="Second request",
                    timestamp="2026-03-13T10:01:00Z",
                ),
                assistant_turn_newer,
            ],
        )
    )

    result = service.create_spec_edit_proposal(
        conversation_id,
        project_path,
        {
            "summary": "Clarify the approval gate.",
            "changes": [
                {
                    "path": "specs/spark-workspace.md#proposal-review",
                    "before": "Planning begins immediately.",
                    "after": "Planning begins only after approval.",
                }
            ],
        },
    )

    assert result["turn_id"] == "turn-assistant-newer"

    snapshot = service.get_snapshot(conversation_id, project_path)
    proposal_segment = next(
        segment for segment in snapshot["segments"] if segment["id"] == result["segment_id"]
    )
    assert proposal_segment["turn_id"] == "turn-assistant-newer"
    assert proposal_segment["artifact_id"] == result["proposal_id"]
    assert snapshot["spec_edit_proposals"][0]["summary"] == "Clarify the approval gate."


def test_create_spec_edit_proposal_by_handle_route_resolves_conversation(
    product_api_client,
    tmp_path: Path,
) -> None:
    project_dir = tmp_path.resolve()
    conversation_id = "conversation-by-handle"
    state = project_chat.ConversationState(
        conversation_id=conversation_id,
        project_path=str(project_dir),
        title="Handle placement",
        created_at="2026-03-13T10:00:00Z",
        updated_at="2026-03-13T10:01:00Z",
        turns=[
            project_chat.ConversationTurn(
                id="turn-user-1",
                role="user",
                content="Please propose a spec change.",
                timestamp="2026-03-13T10:00:00Z",
            ),
            project_chat.ConversationTurn(
                id="turn-assistant-1",
                role="assistant",
                content="I can capture that proposal.",
                timestamp="2026-03-13T10:01:00Z",
                status="complete",
            ),
        ],
    )
    server.PROJECT_CHAT._write_state(state)
    snapshot = server.PROJECT_CHAT.get_snapshot(conversation_id, str(project_dir))

    response = product_api_client.post(
        f"/workspace/api/conversations/by-handle/{snapshot['conversation_handle']}/spec-edit-proposals",
        json={
            "summary": "Capture the approved spec gate.",
            "changes": [
                {
                    "path": "specs/spark-workspace.md#proposal-review",
                    "before": "Planning begins immediately.",
                    "after": "Planning begins only after the proposal is approved.",
                }
            ],
            "rationale": "Keep the workflow human-approved.",
        },
    )

    assert response.status_code == 200
    payload = response.json()
    assert payload["ok"] is True
    assert payload["conversation_id"] == conversation_id
    assert payload["conversation_handle"] == snapshot["conversation_handle"]
    assert payload["turn_id"] == "turn-assistant-1"


def test_create_flow_run_request_places_artifact_on_latest_assistant_turn(tmp_path: Path) -> None:
    service = project_chat.ProjectChatService(tmp_path)
    conversation_id = "conversation-test"
    project_path = str(tmp_path.resolve())
    service._write_state(
        project_chat.ConversationState(
            conversation_id=conversation_id,
            project_path=project_path,
            title="Flow run placement",
            created_at="2026-03-13T10:00:00Z",
            updated_at="2026-03-13T10:02:00Z",
            turns=[
                project_chat.ConversationTurn(
                    id="turn-user-1",
                    role="user",
                    content="First request",
                    timestamp="2026-03-13T09:59:00Z",
                ),
                project_chat.ConversationTurn(
                    id="turn-assistant-older",
                    role="assistant",
                    content="Older assistant reply.",
                    timestamp="2026-03-13T10:00:00Z",
                    status="complete",
                ),
                project_chat.ConversationTurn(
                    id="turn-user-2",
                    role="user",
                    content="Second request",
                    timestamp="2026-03-13T10:01:00Z",
                ),
                project_chat.ConversationTurn(
                    id="turn-assistant-newer",
                    role="assistant",
                    content="Latest assistant reply.",
                    timestamp="2026-03-13T10:02:00Z",
                    status="complete",
                ),
            ],
        )
    )

    result = service.create_flow_run_request(
        conversation_id,
        project_path,
        {
            "flow_name": "implement-spec.dot",
            "summary": "Run implementation for the approved scope.",
            "goal": "Implement the approved scope.",
            "launch_context": {
                "context.request.summary": "Implement the approved scope.",
                "context.request.target_paths": ["src/workspace", "tests/api"],
            },
            "model": "gpt-5.4",
        },
    )

    assert result["turn_id"] == "turn-assistant-newer"

    snapshot = service.get_snapshot(conversation_id, project_path)
    request_segment = next(
        segment for segment in snapshot["segments"] if segment["id"] == result["segment_id"]
    )
    assert request_segment["turn_id"] == "turn-assistant-newer"
    assert request_segment["artifact_id"] == result["flow_run_request_id"]
    assert snapshot["flow_run_requests"][0]["flow_name"] == "implement-spec.dot"
    assert snapshot["flow_run_requests"][0]["summary"] == "Run implementation for the approved scope."
    assert snapshot["flow_run_requests"][0]["launch_context"] == {
        "context.request.summary": "Implement the approved scope.",
        "context.request.target_paths": ["src/workspace", "tests/api"],
    }


def test_flow_run_request_routes_create_and_approve_launch(
    product_api_client,
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    project_dir = tmp_path.resolve()
    conversation_id = "conversation-flow-run"
    state = project_chat.ConversationState(
        conversation_id=conversation_id,
        project_path=str(project_dir),
        title="Flow request route",
        created_at="2026-03-13T10:00:00Z",
        updated_at="2026-03-13T10:01:00Z",
        turns=[
            project_chat.ConversationTurn(
                id="turn-user-1",
                role="user",
                content="Please run the implementation flow.",
                timestamp="2026-03-13T10:00:00Z",
            ),
            project_chat.ConversationTurn(
                id="turn-assistant-1",
                role="assistant",
                content="I can request that launch.",
                timestamp="2026-03-13T10:01:00Z",
                status="complete",
            ),
        ],
    )
    server.PROJECT_CHAT._write_state(state)
    snapshot = server.PROJECT_CHAT.get_snapshot(conversation_id, str(project_dir))

    flows_dir = server.get_settings().flows_dir
    flows_dir.mkdir(parents=True, exist_ok=True)
    (flows_dir / "implement-spec.dot").write_text("digraph implement_spec { start -> done; }\n", encoding="utf-8")

    start_calls: list[dict[str, object | None]] = []

    async def fake_start_pipeline(
        self,
        *,
        run_id: str | None,
        flow_name: str,
        working_directory: str,
        model: str | None,
        goal: str | None = None,
        launch_context: dict[str, object] | None = None,
        spec_id: str | None = None,
        plan_id: str | None = None,
    ) -> dict[str, object]:
        start_calls.append(
            {
                "run_id": run_id,
                "flow_name": flow_name,
                "working_directory": working_directory,
                "model": model,
                "goal": goal,
                "launch_context": launch_context,
                "spec_id": spec_id,
                "plan_id": plan_id,
            }
        )
        return {"status": "started", "run_id": "run-flow-123"}

    monkeypatch.setattr(attractor_client.AttractorApiClient, "start_pipeline", fake_start_pipeline)

    create_response = product_api_client.post(
        f"/workspace/api/conversations/by-handle/{snapshot['conversation_handle']}/flow-run-requests",
        json={
            "flow_name": "implement-spec.dot",
            "summary": "Run implementation for the approved scope.",
            "goal": "Implement the approved scope.",
            "launch_context": {
                "context.request.summary": "Implement the approved scope.",
                "context.request.acceptance_criteria": [
                    "Approved work items are implemented.",
                    "Required tests are updated.",
                ],
            },
            "model": "gpt-5.4",
        },
    )

    assert create_response.status_code == 200
    create_payload = create_response.json()
    assert create_payload["ok"] is True
    assert create_payload["conversation_id"] == conversation_id
    request_id = create_payload["flow_run_request_id"]

    review_response = product_api_client.post(
        f"/workspace/api/conversations/{conversation_id}/flow-run-requests/{request_id}/review",
        json={
            "project_path": str(project_dir),
            "disposition": "approved",
            "message": "Approved for launch.",
        },
    )

    assert review_response.status_code == 200
    approved_snapshot = review_response.json()
    request_payload = next(
        entry for entry in approved_snapshot["flow_run_requests"] if entry["id"] == request_id
    )
    assert request_payload["status"] == "launched"
    assert request_payload["run_id"] == "run-flow-123"
    assert request_payload["review_message"] == "Approved for launch."
    assert start_calls == [
        {
            "run_id": None,
            "flow_name": "implement-spec.dot",
            "working_directory": str(project_dir),
            "model": "gpt-5.4",
            "goal": "Implement the approved scope.",
            "launch_context": {
                "context.request.summary": "Implement the approved scope.",
                "context.request.acceptance_criteria": [
                    "Approved work items are implemented.",
                    "Required tests are updated.",
                ],
            },
            "spec_id": None,
            "plan_id": None,
        }
    ]


def test_direct_flow_launch_routes_create_inline_artifact_and_launch(
    product_api_client,
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    project_dir = tmp_path.resolve()
    conversation_id = "conversation-flow-launch"
    state = project_chat.ConversationState(
        conversation_id=conversation_id,
        project_path=str(project_dir),
        title="Flow launch route",
        created_at="2026-03-13T10:00:00Z",
        updated_at="2026-03-13T10:01:00Z",
        turns=[
            project_chat.ConversationTurn(
                id="turn-user-1",
                role="user",
                content="Launch the implementation flow now.",
                timestamp="2026-03-13T10:00:00Z",
            ),
            project_chat.ConversationTurn(
                id="turn-assistant-1",
                role="assistant",
                content="I can launch that now.",
                timestamp="2026-03-13T10:01:00Z",
                status="complete",
            ),
        ],
    )
    server.PROJECT_CHAT._write_state(state)
    snapshot = server.PROJECT_CHAT.get_snapshot(conversation_id, str(project_dir))

    flows_dir = server.get_settings().flows_dir
    flows_dir.mkdir(parents=True, exist_ok=True)
    (flows_dir / "implement-spec.dot").write_text("digraph implement_spec { start -> done; }\n", encoding="utf-8")

    start_calls: list[dict[str, object | None]] = []

    async def fake_start_pipeline(
        self,
        *,
        run_id: str | None,
        flow_name: str,
        working_directory: str,
        model: str | None,
        goal: str | None = None,
        launch_context: dict[str, object] | None = None,
        spec_id: str | None = None,
        plan_id: str | None = None,
    ) -> dict[str, object]:
        start_calls.append(
            {
                "run_id": run_id,
                "flow_name": flow_name,
                "working_directory": working_directory,
                "model": model,
                "goal": goal,
                "launch_context": launch_context,
                "spec_id": spec_id,
                "plan_id": plan_id,
            }
        )
        return {"status": "started", "run_id": "run-launch-123"}

    monkeypatch.setattr(attractor_client.AttractorApiClient, "start_pipeline", fake_start_pipeline)

    launch_response = product_api_client.post(
        "/workspace/api/runs/launch",
        json={
            "flow_name": "implement-spec.dot",
            "summary": "Launch implementation immediately.",
            "conversation_handle": snapshot["conversation_handle"],
            "project_path": str(project_dir),
            "goal": "Implement the approved scope.",
            "launch_context": {
                "context.request.summary": "Implement the approved scope.",
            },
            "model": "gpt-5.4",
        },
    )

    assert launch_response.status_code == 200
    launch_payload = launch_response.json()
    assert launch_payload["ok"] is True
    assert launch_payload["conversation_id"] == conversation_id
    assert launch_payload["conversation_handle"] == snapshot["conversation_handle"]
    assert launch_payload["run_id"] == "run-launch-123"
    assert launch_payload["flow_launch_id"].startswith("flow-launch-")

    updated_snapshot = server.PROJECT_CHAT.get_snapshot(conversation_id, str(project_dir))
    flow_launch = next(
        entry for entry in updated_snapshot["flow_launches"] if entry["id"] == launch_payload["flow_launch_id"]
    )
    assert flow_launch["status"] == "launched"
    assert flow_launch["run_id"] == "run-launch-123"
    assert flow_launch["goal"] == "Implement the approved scope."
    segment = next(
        entry for entry in updated_snapshot["segments"] if entry["artifact_id"] == launch_payload["flow_launch_id"]
    )
    assert segment["kind"] == "flow_launch"
    assert segment["turn_id"] == "turn-assistant-1"
    assert start_calls == [
        {
            "run_id": None,
            "flow_name": "implement-spec.dot",
            "working_directory": str(project_dir),
            "model": "gpt-5.4",
            "goal": "Implement the approved scope.",
            "launch_context": {
                "context.request.summary": "Implement the approved scope.",
            },
            "spec_id": None,
            "plan_id": None,
        }
    ]


def test_mark_execution_workflow_started_loads_conversation_without_project_argument(tmp_path: Path) -> None:
    service = project_chat.ProjectChatService(tmp_path)
    service._write_state(
        project_chat.ConversationState(
            conversation_id="conversation-test",
            project_path=str(tmp_path),
            title="Workflow state test",
            created_at="2026-03-11T02:00:00Z",
            updated_at="2026-03-11T02:00:00Z",
        )
    )

    snapshot = service.mark_execution_workflow_started(
        "conversation-test",
        "workflow-123",
        "spec_edit_approval",
    )

    assert snapshot["execution_workflow"]["run_id"] == "workflow-123"
    assert snapshot["execution_workflow"]["status"] == "running"
    assert snapshot["execution_workflow"]["flow_source"] == "spec_edit_approval"


def test_prepare_execution_workflow_launch_builds_prompt_from_approved_proposal(tmp_path: Path) -> None:
    service = project_chat.ProjectChatService(tmp_path)
    state = project_chat.ConversationState(
        conversation_id="conversation-test",
        project_path=str(tmp_path),
        title="Workflow run test",
        created_at="2026-03-11T02:00:00Z",
        updated_at="2026-03-11T02:00:00Z",
        spec_edit_proposals=[
            project_chat.SpecEditProposal(
                id="proposal-1",
                created_at="2026-03-11T02:00:00Z",
                summary="Summary",
                changes=[project_chat_models.SpecEditProposalChange(path="specs/project.md", before="old", after="new")],
                status="applied",
                canonical_spec_edit_id="spec-edit-collatz-001",
            )
        ],
    )
    service._write_state(state)

    launch_spec = service.prepare_execution_workflow_launch(
        "conversation-test",
        "proposal-1",
        "Focus the first step on validating the CLI contract.",
    )

    assert launch_spec.conversation_id == "conversation-test"
    assert launch_spec.project_path == str(tmp_path)
    assert launch_spec.proposal_id == "proposal-1"
    assert launch_spec.spec_id == "spec-edit-collatz-001"
    assert '"id": "proposal-1"' in launch_spec.prompt
    assert "Focus the first step on validating the CLI contract." in launch_spec.prompt


def test_complete_execution_workflow_creates_execution_card_and_clears_matching_run(tmp_path: Path) -> None:
    service = project_chat.ProjectChatService(tmp_path)
    state = project_chat.ConversationState(
        conversation_id="conversation-test",
        project_path=str(tmp_path),
        title="Workflow run test",
        created_at="2026-03-11T02:00:00Z",
        updated_at="2026-03-11T02:00:00Z",
        spec_edit_proposals=[
            project_chat.SpecEditProposal(
                id="proposal-1",
                created_at="2026-03-11T02:00:00Z",
                summary="Summary",
                changes=[project_chat_models.SpecEditProposalChange(path="specs/project.md", before="old", after="new")],
                status="applied",
                canonical_spec_edit_id="spec-edit-collatz-001",
            )
        ],
        execution_workflow=project_chat_models.ExecutionWorkflowState(
            run_id="workflow-123",
            status="running",
            flow_source="plan-generation.dot",
        ),
    )

    service._write_state(state)

    execution_card = service.complete_execution_workflow(
        "conversation-test",
        "proposal-1",
        "plan-generation.dot",
        "implement-spec.dot",
        "workflow-123",
        json.dumps(
            {
                "title": "Execution plan",
                "summary": "Plan summary",
                "objective": "Implement the approved spec edit.",
                "work_items": [
                    {
                        "id": "work-1",
                        "title": "Update spec",
                        "description": "Apply the approved change.",
                        "acceptance_criteria": ["Spec updated"],
                        "depends_on": [],
                    }
                ],
            }
        ),
    )

    assert execution_card.source_workflow_run_id == "workflow-123"
    assert execution_card.flow_source == "implement-spec.dot"
    snapshot = service.get_snapshot("conversation-test", str(tmp_path))
    assert snapshot["execution_workflow"]["status"] == "idle"
    assert snapshot["execution_cards"][0]["id"] == execution_card.id
    assert snapshot["turns"][-1]["artifact_id"] == execution_card.id


def test_fail_execution_workflow_marks_matching_run_failed(tmp_path: Path) -> None:
    service = project_chat.ProjectChatService(tmp_path)
    state = project_chat.ConversationState(
        conversation_id="conversation-test",
        project_path=str(tmp_path),
        title="Workflow run test",
        created_at="2026-03-11T02:00:00Z",
        updated_at="2026-03-11T02:00:00Z",
        execution_workflow=project_chat_models.ExecutionWorkflowState(
            run_id="workflow-123",
            status="running",
            flow_source="plan-generation.dot",
        ),
    )
    service._write_state(state)

    snapshot = service.fail_execution_workflow(
        "conversation-test",
        "workflow-123",
        "plan-generation.dot",
        "boom",
    )

    assert snapshot["execution_workflow"]["status"] == "failed"
    assert snapshot["execution_workflow"]["error"] == "boom"


def test_note_execution_card_dispatched_records_event(tmp_path: Path) -> None:
    service = project_chat.ProjectChatService(tmp_path)
    state = project_chat.ConversationState(
        conversation_id="conversation-test",
        project_path=str(tmp_path),
        title="Workflow run test",
        created_at="2026-03-11T02:00:00Z",
        updated_at="2026-03-11T02:00:00Z",
        execution_cards=[
            project_chat.ExecutionCard(
                id="execution-card-1",
                title="Execution plan",
                summary="Plan summary",
                objective="Do the thing.",
                source_spec_edit_id="spec-edit-1",
                source_workflow_run_id="workflow-plan-1",
                created_at="2026-03-11T02:00:00Z",
                updated_at="2026-03-11T02:00:00Z",
                status="approved",
            )
        ],
    )
    service._write_state(state)

    snapshot = service.note_execution_card_dispatched(
        "conversation-test",
        "execution-card-1",
        "run-123",
        "implement-spec.dot",
    )

    assert snapshot["event_log"][-1]["message"] == "Dispatched execution card execution-card-1 as run run-123 using implement-spec.dot."


def test_chat_session_emits_assistant_completed_from_item_completed(monkeypatch) -> None:
    session = project_chat.CodexAppServerChatSession("/tmp/project")
    lines = iter(
        [
            json.dumps(
                {
                    "method": "item/agentMessage/delta",
                    "params": {"delta": "Ack"},
                }
            ),
            json.dumps(
                {
                    "method": "item/completed",
                    "params": {
                        "item": {
                            "type": "AgentMessage",
                            "id": "msg-1",
                            "content": [{"type": "Text", "text": "Ack"}],
                            "phase": "final_answer",
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
    captured_events: list[project_chat.ChatTurnLiveEvent] = []

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
        on_event=lambda event: captured_events.append(event),
    )

    assert result.assistant_message == "Ack"
    assert [event.kind for event in captured_events] == ["assistant_delta", "assistant_completed"]
    assert captured_events[1].item_id == "msg-1"
    assert captured_events[1].phase == "final_answer"
    assert captured_events[-1].content_delta == "Ack"


def test_chat_session_handles_command_output_delta_without_reasoning_fallback_helper(monkeypatch) -> None:
    session = project_chat.CodexAppServerChatSession("/tmp/project")
    lines = iter(
        [
            json.dumps(
                {
                    "method": "item/reasoning/summaryTextDelta",
                    "params": {"delta": "Thinking...", "itemId": "rs-1", "summaryIndex": 0},
                }
            ),
            json.dumps(
                {
                    "method": "item/commandExecution/outputDelta",
                    "params": {"delta": "output", "itemId": "cmd-1"},
                }
            ),
            json.dumps(
                {
                    "method": "item/agentMessage/delta",
                    "params": {"delta": "Ack", "itemId": "msg-1"},
                }
            ),
            json.dumps(
                {
                    "method": "item/completed",
                    "params": {
                        "item": {
                            "type": "AgentMessage",
                            "id": "msg-1",
                            "content": [{"type": "Text", "text": "Ack"}],
                            "phase": "final_answer",
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
    captured_events: list[project_chat.ChatTurnLiveEvent] = []

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
        on_event=lambda event: captured_events.append(event),
    )

    assert result.assistant_message == "Ack"
    assert [event.kind for event in captured_events] == ["reasoning_summary", "assistant_delta", "assistant_completed"]


def test_chat_session_emits_assistant_completed_for_commentary_item(monkeypatch) -> None:
    session = project_chat.CodexAppServerChatSession("/tmp/project")
    lines = iter(
        [
            json.dumps(
                {
                    "method": "item/agentMessage/delta",
                    "params": {"delta": "I’m drafting the proposal now.", "itemId": "msg-1"},
                }
            ),
            json.dumps(
                {
                    "method": "item/completed",
                    "params": {
                        "item": {
                            "type": "AgentMessage",
                            "id": "msg-1",
                            "content": [{"type": "Text", "text": "I’m drafting the proposal now."}],
                            "phase": "commentary",
                        }
                    },
                }
            ),
            json.dumps(
                {
                    "method": "item/agentMessage/delta",
                    "params": {
                        "delta": "Done.",
                        "itemId": "msg-2",
                    },
                }
            ),
            json.dumps(
                {
                    "method": "item/completed",
                    "params": {
                        "item": {
                            "type": "AgentMessage",
                            "id": "msg-2",
                            "content": [{"type": "Text", "text": "Done."}],
                            "phase": "final_answer",
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
    captured_events: list[project_chat.ChatTurnLiveEvent] = []

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

    result = session.turn("hello", None, on_event=captured_events.append)

    assert result.assistant_message == "Done."
    assert [event.kind for event in captured_events] == [
        "assistant_delta",
        "assistant_completed",
        "assistant_delta",
        "assistant_completed",
    ]
    assert captured_events[0].item_id == "msg-1"
    assert captured_events[1].phase == "commentary"
    assert captured_events[2].item_id == "msg-2"
    assert captured_events[3].phase == "final_answer"


def test_build_session_ignores_unsupported_persisted_thread_state(tmp_path: Path, monkeypatch) -> None:
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
                "thread_id": "stale-thread",
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


def test_send_turn_writes_raw_jsonrpc_log(tmp_path: Path, monkeypatch) -> None:
    service = project_chat.ProjectChatService(tmp_path)

    class FakeSession:
        def __init__(self) -> None:
            self.raw_logger = None

        def set_raw_rpc_logger(self, callback) -> None:
            self.raw_logger = callback

        def clear_raw_rpc_logger(self) -> None:
            self.raw_logger = None

        def turn(
            self,
            prompt: str,
            model: str | None,
            *,
            on_event=None,
            on_dynamic_tool_call=None,
        ) -> project_chat.ChatTurnResult:
            assert self.raw_logger is not None
            self.raw_logger("outgoing", '{"jsonrpc":"2.0","id":1,"method":"turn/start"}')
            self.raw_logger("incoming", '{"jsonrpc":"2.0","method":"turn/completed","params":{"turn":{"status":"completed"}}}')
            if on_event is not None:
                on_event(
                    project_chat.ChatTurnLiveEvent(
                        kind="assistant_completed",
                        content_delta="Logged.",
                        app_turn_id="app-turn-1",
                        item_id="msg-1",
                        phase="final_answer",
                    )
                )
            return project_chat.ChatTurnResult(
                assistant_message='{"assistant_message":"Logged."}',
            )

    monkeypatch.setattr(service, "_build_session", lambda conversation_id, project_path: FakeSession())

    snapshot = service.send_turn("conversation-test", str(tmp_path), "hello", None)

    assert snapshot["turns"][-1]["content"] == "Logged."
    raw_log_path = ensure_project_paths(tmp_path, str(tmp_path)).conversations_dir / "conversation-test" / "raw-log.jsonl"
    raw_entries = [json.loads(line) for line in raw_log_path.read_text(encoding="utf-8").splitlines()]
    assert [(entry["direction"], entry["line"]) for entry in raw_entries] == [
        ("outgoing", '{"jsonrpc":"2.0","id":1,"method":"turn/start"}'),
        ("incoming", '{"jsonrpc":"2.0","method":"turn/completed","params":{"turn":{"status":"completed"}}}'),
    ]


def test_start_turn_returns_initial_snapshot_before_background_completion(tmp_path: Path, monkeypatch) -> None:
    service = project_chat.ProjectChatService(tmp_path)
    entered_turn = threading.Event()
    finish_turn = threading.Event()

    class FakeSession:
        def turn(
            self,
            prompt: str,
            model: str | None,
            *,
            on_event=None,
            on_dynamic_tool_call=None,
        ) -> project_chat.ChatTurnResult:
            entered_turn.set()
            assert finish_turn.wait(timeout=2)
            if on_event is not None:
                on_event(
                    project_chat.ChatTurnLiveEvent(
                        kind="reasoning_summary",
                        content_delta="Checking the repository.",
                    )
                )
                on_event(
                    project_chat.ChatTurnLiveEvent(
                        kind="assistant_completed",
                        content_delta="ACK",
                        app_turn_id="app-turn-1",
                        item_id="msg-1",
                        phase="final_answer",
                    )
                )
            return project_chat.ChatTurnResult(
                assistant_message='{"assistant_message":"ACK"}',
            )

    monkeypatch.setattr(service, "_build_session", lambda conversation_id, project_path: FakeSession())

    snapshot = service.start_turn("conversation-test", str(tmp_path), "hello", None)

    assert [turn["role"] for turn in snapshot["turns"]] == ["user", "assistant"]
    assert snapshot["turns"][-1]["status"] == "pending"
    assert snapshot["turns"][-1]["content"] == ""
    assert entered_turn.wait(timeout=2)

    finish_turn.set()
    deadline = time.time() + 2.0
    final_snapshot: dict[str, object] | None = None
    while time.time() < deadline:
        candidate = service.get_snapshot("conversation-test", str(tmp_path))
        if candidate["turns"][-1]["status"] == "complete":
            final_snapshot = candidate
            break
        time.sleep(0.02)

    assert final_snapshot is not None
    assert final_snapshot["turns"][-1]["content"] == "ACK"
    assert [segment["kind"] for segment in final_snapshot["segments"]] == [
        "reasoning",
        "assistant_message",
    ]


def test_start_turn_rejects_overlapping_active_assistant_turn(tmp_path: Path) -> None:
    service = project_chat.ProjectChatService(tmp_path)
    service._write_state(
        project_chat.ConversationState(
            conversation_id="conversation-test",
            project_path=str(tmp_path),
            turns=[
                project_chat.ConversationTurn(
                    id="turn-user-1",
                    role="user",
                    content="First request",
                    timestamp="2026-03-15T14:00:00Z",
                    status="complete",
                ),
                project_chat.ConversationTurn(
                    id="turn-assistant-1",
                    role="assistant",
                    content="",
                    timestamp="2026-03-15T14:00:01Z",
                    status="streaming",
                    parent_turn_id="turn-user-1",
                ),
            ],
        )
    )

    with pytest.raises(
        project_chat.TurnInProgressError,
        match="assistant turn is still in progress",
    ):
        service.start_turn("conversation-test", str(tmp_path), "Second request", None)


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
    product_api_client,
    monkeypatch,
    tmp_path: Path,
) -> None:
    service = server.PROJECT_CHAT
    entered_turn = threading.Event()
    finish_turn = threading.Event()

    class FakeSession:
        def turn(
            self,
            prompt: str,
            model: str | None,
            *,
            on_event=None,
            on_dynamic_tool_call=None,
        ) -> project_chat.ChatTurnResult:
            entered_turn.set()
            assert finish_turn.wait(timeout=2)
            if on_event is not None:
                on_event(
                    project_chat.ChatTurnLiveEvent(
                        kind="reasoning_summary",
                        content_delta="Checking whether a spec proposal makes sense.",
                    )
                )
                on_event(
                    project_chat.ChatTurnLiveEvent(
                        kind="assistant_delta",
                        content_delta="Working on it",
                        app_turn_id="app-turn-1",
                        item_id="msg-1",
                        phase="final_answer",
                    )
                )
                on_event(
                    project_chat.ChatTurnLiveEvent(
                        kind="assistant_completed",
                        content_delta="Working on it",
                        app_turn_id="app-turn-1",
                        item_id="msg-1",
                        phase="final_answer",
                    )
                )
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
            return project_chat.ChatTurnResult(
                assistant_message='{"assistant_message":"ACK","spec_proposal":null}',
            )

    monkeypatch.setattr(service, "_build_session", lambda conversation_id, project_path: FakeSession())

    response = product_api_client.post(
        "/workspace/api/conversations/conversation-test/turns",
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
    assert payload["turns"][1]["content"] == ""
    assert payload["turns"][1]["status"] == "pending"
    assert payload["segments"] == []
    assert entered_turn.wait(timeout=2)

    finish_turn.set()
    deadline = time.time() + 2.0
    final_snapshot: dict[str, object] | None = None
    while time.time() < deadline:
        candidate = service.get_snapshot("conversation-test", str(tmp_path))
        if candidate["turns"][-1]["status"] == "complete":
            final_snapshot = candidate
            break
        time.sleep(0.02)

    assert final_snapshot is not None
    assert [turn["role"] for turn in final_snapshot["turns"]] == ["user", "assistant"]
    assert final_snapshot["turns"][1]["content"] == "Working on it"
    assert final_snapshot["turns"][1]["status"] == "complete"
    assert [segment["kind"] for segment in final_snapshot["segments"]] == [
        "reasoning",
        "assistant_message",
        "tool_call",
    ]


def test_snapshot_rejects_unsupported_turn_event_only_payload(tmp_path: Path) -> None:
    service = project_chat.ProjectChatService(tmp_path)
    project_paths = ensure_project_paths(tmp_path, str(tmp_path))
    invalid_payload = {
        "conversation_id": "conversation-compact",
        "project_path": str(tmp_path),
        "title": "Compact thread",
        "created_at": "2026-03-07T18:00:00Z",
        "updated_at": "2026-03-07T18:00:03Z",
        "turns": [
            {
                "id": "turn-user-1",
                "role": "user",
                "content": "hi",
                "timestamp": "2026-03-07T18:00:00Z",
                "status": "complete",
                "kind": "message",
            },
            {
                "id": "turn-assistant-1",
                "role": "assistant",
                "content": "hello",
                "timestamp": "2026-03-07T18:00:03Z",
                "status": "complete",
                "kind": "message",
                "parent_turn_id": "turn-user-1",
            },
        ],
        "turn_events": [
            {
                "id": "event-assistant-delta-1",
                "turn_id": "turn-assistant-1",
                "sequence": 1,
                "timestamp": "2026-03-07T18:00:01Z",
                "kind": "assistant_delta",
                "content_delta": "hel",
            },
            {
                "id": "event-reasoning-1",
                "turn_id": "turn-assistant-1",
                "sequence": 2,
                "timestamp": "2026-03-07T18:00:01Z",
                "kind": "reasoning_summary",
                "content_delta": "Thinking about the repository structure.",
            },
            {
                "id": "event-assistant-delta-2",
                "turn_id": "turn-assistant-1",
                "sequence": 3,
                "timestamp": "2026-03-07T18:00:02Z",
                "kind": "assistant_delta",
                "content_delta": "lo",
            },
            {
                "id": "event-assistant-completed-1",
                "turn_id": "turn-assistant-1",
                "sequence": 4,
                "timestamp": "2026-03-07T18:00:03Z",
                "kind": "assistant_completed",
                "message": "Assistant turn completed.",
            },
        ],
    }
    (project_paths.conversations_dir / "conversation-compact").mkdir(parents=True, exist_ok=True)
    (project_paths.conversations_dir / "conversation-compact" / "state.json").write_text(
        json.dumps(invalid_payload, indent=2),
        encoding="utf-8",
    )

    with pytest.raises(ValueError, match="Unsupported conversation state schema"):
        service.get_snapshot("conversation-compact", str(tmp_path))


def test_list_project_conversations_endpoint_returns_project_threads(product_api_client, tmp_path: Path) -> None:
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

    response = product_api_client.get("/workspace/api/projects/conversations", params={"project_path": str(tmp_path)})

    assert response.status_code == 200
    payload = response.json()
    assert [entry["conversation_id"] for entry in payload] == ["conversation-a"]
    assert payload[0]["conversation_handle"]
    assert payload[0]["title"] == "Design thread"
    assert payload[0]["last_message_preview"] == "Design thread preview"


def test_delete_project_conversation_endpoint_removes_thread_state(product_api_client, tmp_path: Path) -> None:
    service = server.PROJECT_CHAT
    conversation_id = "conversation-delete-me"
    project_paths = ensure_project_paths(tmp_path / ".spark", str(tmp_path))
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

    response = product_api_client.delete(
        f"/workspace/api/conversations/{conversation_id}",
        params={"project_path": str(tmp_path)},
    )

    assert response.status_code == 200
    assert response.json() == {
        "status": "deleted",
        "conversation_id": conversation_id,
        "project_path": str(tmp_path.resolve()),
    }
    assert not (project_paths.conversations_dir / conversation_id).exists()
    handle_index = json.loads(conversation_handles_path(tmp_path / ".spark").read_text(encoding="utf-8"))
    assert conversation_id not in handle_index["conversation_ids"]

    list_response = product_api_client.get("/workspace/api/projects/conversations", params={"project_path": str(tmp_path)})
    assert list_response.status_code == 200
    assert list_response.json() == []
