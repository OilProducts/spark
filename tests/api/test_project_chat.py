from __future__ import annotations

import json
import threading
import time
from pathlib import Path
from typing import Any, Callable, Optional

import pytest

import attractor.api.server as server
import spark.app as product_app
import spark.chat.service as project_chat
import spark.chat.session as project_chat_session
import spark.workspace.attractor_client as attractor_client
from spark_common.codex_app_client import CodexAppServerTurnResult
import spark_common.codex_app_server as codex_app_server
import spark_common.process_line_reader as process_line_reader
from spark.authoring_assets import (
    dot_authoring_guide_path,
    spark_operations_guide_path,
)
from tests.support.flow_fixtures import seed_flow_fixture
from spark.chat.prompt_templates import PROMPTS_FILE_NAME
from spark.workspace.storage import conversation_handles_path, ensure_project_paths


TEST_DISPATCH_FLOW = "test-dispatch.dot"


def _project_chat_service() -> project_chat.ProjectChatService:
    return product_app.get_project_chat()


def _completed_turn_result(
    *,
    thread_id: str = "thread-123",
    turn_id: str = "turn-123",
    assistant_message: str = "Ack",
    plan_message: str = "",
    command_text: str = "",
    token_total: Optional[int] = None,
    error: Optional[str] = None,
) -> CodexAppServerTurnResult:
    state = codex_app_server.CodexAppServerTurnState()
    state.final_agent_message = assistant_message
    state.final_plan_message = plan_message or None
    state.turn_status = "completed"
    if command_text:
        state.command_chunks.append(command_text)
    if token_total is not None:
        state.last_token_total = token_total
    if error:
        state.turn_status = "failed"
        state.turn_error = error
        state.last_error = error
    return CodexAppServerTurnResult(thread_id=thread_id, turn_id=turn_id, state=state)


class StubChatClient:
    def __init__(self) -> None:
        self.proc: object | None = None
        self.ensure_process_calls = 0
        self.resume_calls: list[dict[str, Any]] = []
        self.start_calls: list[dict[str, Any]] = []
        self.run_turn_calls: list[dict[str, Any]] = []
        self.resume_result: Optional[str] = None
        self.start_result: str = "thread-123"
        self.default_model_result: Optional[str] = "gpt-test"
        self.run_turn_handler: Optional[Callable[..., CodexAppServerTurnResult]] = None
        self.raw_logger = None
        self.closed = False

    def close(self) -> None:
        self.closed = True
        self.proc = None

    def clear_raw_rpc_logger(self) -> None:
        self.raw_logger = None

    def ensure_process(self, *, popen_factory) -> None:
        self.ensure_process_calls += 1
        if self.proc is None:
            self.proc = object()

    def resume_thread(
        self,
        thread_id: str,
        *,
        model: str | None,
        cwd: str | None = None,
        approval_policy: str = "never",
    ) -> Optional[str]:
        self.resume_calls.append(
            {
                "thread_id": thread_id,
                "model": model,
                "cwd": cwd,
                "approval_policy": approval_policy,
            }
        )
        return self.resume_result

    def run_turn(self, **kwargs) -> CodexAppServerTurnResult:
        self.run_turn_calls.append(kwargs)
        if self.run_turn_handler is not None:
            return self.run_turn_handler(**kwargs)
        return _completed_turn_result(thread_id=kwargs["thread_id"])

    def default_model(self) -> Optional[str]:
        return self.default_model_result

    def set_raw_rpc_logger(self, callback) -> None:
        self.raw_logger = callback

    def start_thread(
        self,
        *,
        model: str | None,
        cwd: str | None = None,
        approval_policy: str = "never",
        ephemeral: bool,
    ) -> str:
        self.start_calls.append(
            {
                "model": model,
                "cwd": cwd,
                "approval_policy": approval_policy,
                "ephemeral": ephemeral,
            }
        )
        return self.start_result


def _seed_flow(name: str) -> None:
    seed_flow_fixture(product_app.get_settings().flows_dir, "minimal-valid.dot", as_name=name)


def test_extract_command_text_handles_list_and_string_payloads() -> None:
    assert codex_app_server.extract_command_text({"command": ["git", "status", "--short"]}) == "git status --short"
    assert codex_app_server.extract_command_text({"commandLine": "npm test"}) == "npm test"


def test_parse_chat_response_payload_accepts_plain_text_and_json() -> None:
    assistant_message, payload = project_chat._parse_chat_response_payload("Plain text reply.")

    assert assistant_message == "Plain text reply."
    assert payload is None

    assistant_message, payload = project_chat._parse_chat_response_payload(
        '{"assistant_message":"Hello.","flow_run_request":null}'
    )

    assert assistant_message == "Hello."
    assert payload == {
        "assistant_message": "Hello.",
        "flow_run_request": None,
    }


def test_project_chat_service_creates_default_prompt_templates_file(tmp_path: Path) -> None:
    service = project_chat.ProjectChatService(tmp_path)

    prompts_path = tmp_path / "config" / PROMPTS_FILE_NAME

    assert prompts_path.exists()
    prompt_text = prompts_path.read_text(encoding="utf-8")
    assert "[project_chat]" in prompt_text
    assert "{{recent_conversation}}" not in prompt_text
    assert "{{latest_user_message}}" in prompt_text
    assert "plan = '''" in prompt_text
    assert service._prompt_templates.chat
    assert service._prompt_templates.plan
    assert "{{recent_conversation}}" not in service._prompt_templates.chat


def test_project_chat_service_uses_custom_prompt_templates(tmp_path: Path) -> None:
    prompts_path = tmp_path / "config" / PROMPTS_FILE_NAME
    prompts_path.parent.mkdir(parents=True, exist_ok=True)
    prompts_path.write_text(
        "\n".join(
            [
                "[project_chat]",
                "chat = '''CHAT {{project_path}} :: {{latest_user_message}}'''",
                "plan = '''PLAN {{project_path}} :: {{latest_user_message}}'''",
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

    assert "Conversation handle: amber-otter" in chat_prompt
    assert "CHAT /tmp/project :: Latest message" in chat_prompt
    assert "Older message" not in chat_prompt


def test_prepare_turn_uses_plan_prompt_template_when_chat_mode_is_plan(tmp_path: Path) -> None:
    prompts_path = tmp_path / "config" / PROMPTS_FILE_NAME
    prompts_path.parent.mkdir(parents=True, exist_ok=True)
    prompts_path.write_text(
        "\n".join(
            [
                "[project_chat]",
                "chat = '''CHAT {{project_path}} :: {{latest_user_message}}'''",
                "plan = '''PLAN {{project_path}} :: {{latest_user_message}}'''",
                "",
            ]
        ),
        encoding="utf-8",
    )
    service = project_chat.ProjectChatService(tmp_path)

    prepared, _ = service._prepare_turn(
        "conversation-plan-template",
        str(tmp_path),
        "Plan the fix.",
        None,
        "plan",
    )

    assert prepared.chat_mode == "plan"
    assert "PLAN " + str(tmp_path.resolve()) + " :: Plan the fix." in prepared.prompt
    assert "Official Codex Plan instructions remain the planning authority." in prepared.prompt
    assert "planning theater or workflow artifacts" not in prepared.prompt


def test_project_chat_prompt_includes_flow_authoring_boundary(tmp_path: Path) -> None:
    service = project_chat.ProjectChatService(tmp_path)
    state = project_chat.ConversationState(
        conversation_id="conversation-flow-authoring",
        project_path="/tmp/project",
        conversation_handle="amber-otter",
    )

    prompt = service._build_chat_prompt(state, "Create a flow that drafts and reviews an email.")

    assert "Spark agent control surface" in prompt
    assert "workspace tool interface" not in prompt
    assert "`spark flow list`" in prompt
    assert "`spark flow describe --flow <name>`" in prompt
    assert "`spark flow get --flow <name>`" in prompt
    assert "`spark flow validate --file <path> --text`" in prompt
    assert "`spark convo run-request ...`" in prompt
    assert "`spark run launch ...`" in prompt
    assert f"flow library at `{(tmp_path / 'flows').resolve(strict=False)}`" in prompt
    assert f"`{dot_authoring_guide_path()}`" in prompt
    assert f"`{spark_operations_guide_path()}`" in prompt
    assert "`--conversation amber-otter`" in prompt
    assert "detached, project-scoped action with no inline chat artifact" in prompt
    assert "spark flow validate --file <path> --text" in prompt


def test_project_chat_service_rejects_malformed_prompt_templates(tmp_path: Path) -> None:
    prompts_path = tmp_path / "config" / PROMPTS_FILE_NAME
    prompts_path.parent.mkdir(parents=True, exist_ok=True)
    prompts_path.write_text("[project_chat]\nchat = '''unterminated\n", encoding="utf-8")

    with pytest.raises(RuntimeError, match="Invalid prompt templates file"):
        project_chat.ProjectChatService(tmp_path)


def test_project_chat_service_rejects_deprecated_recent_conversation_prompt_placeholder(tmp_path: Path) -> None:
    prompts_path = tmp_path / "config" / PROMPTS_FILE_NAME
    prompts_path.parent.mkdir(parents=True, exist_ok=True)
    prompts_path.write_text(
        "\n".join(
            [
                "[project_chat]",
                "chat = '''CHAT {{project_path}} :: {{latest_user_message}} :: {{recent_conversation}}'''",
                "",
            ]
        ),
        encoding="utf-8",
    )

    with pytest.raises(RuntimeError, match=r"deprecated placeholder .*recent_conversation"):
        project_chat.ProjectChatService(tmp_path)


def test_project_chat_service_rejects_prompt_templates_missing_required_keys(tmp_path: Path) -> None:
    prompts_path = tmp_path / "config" / PROMPTS_FILE_NAME
    prompts_path.parent.mkdir(parents=True, exist_ok=True)
    prompts_path.write_text(
        "\n".join(
            [
                "[project_chat]",
                "unused = '''CHAT {{project_path}}'''",
                "",
            ]
        ),
        encoding="utf-8",
    )

    with pytest.raises(RuntimeError, match="missing required templates"):
        project_chat.ProjectChatService(tmp_path)


def test_extract_file_paths_deduplicates_nested_entries() -> None:
    payload = {
        "path": "frontend/src/features/projects/ProjectsPanel.tsx",
        "files": [
            "frontend/src/features/projects/ProjectsPanel.tsx",
            {"path": "frontend/src/lib/apiClient.ts"},
        ],
        "changes": [
            {"filePath": "frontend/src/features/projects/ProjectsPanel.tsx"},
            {"file_path": "frontend/src/store.ts"},
        ],
    }

    assert codex_app_server.extract_file_paths(payload) == [
        "frontend/src/features/projects/ProjectsPanel.tsx",
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


def test_process_turn_message_normalizes_context_compaction_events() -> None:
    state = codex_app_server.CodexAppServerTurnState()

    started = codex_app_server.process_turn_message(
        {
            "method": "item/started",
            "params": {
                "item": {
                    "type": "contextCompaction",
                    "id": "compact-1",
                },
            },
        },
        state,
    )
    completed = codex_app_server.process_turn_message(
        {
            "method": "item/completed",
            "params": {
                "item": {
                    "type": "contextCompaction",
                    "id": "compact-1",
                },
            },
        },
        state,
    )
    fallback = codex_app_server.process_turn_message(
        {
            "method": "thread/compacted",
            "params": {},
        },
        state,
    )

    assert [event.kind for event in started] == ["context_compaction_started"]
    assert started[0].item_id == "compact-1"
    assert [event.kind for event in completed] == ["context_compaction_completed"]
    assert completed[0].item_id == "compact-1"
    assert [event.kind for event in fallback] == ["context_compaction_completed"]
    assert fallback[0].item_id is None


def test_process_turn_message_retains_full_token_usage_payload() -> None:
    state = codex_app_server.CodexAppServerTurnState()
    payload = {
        "last": {
            "inputTokens": 120,
            "cachedInputTokens": 20,
            "outputTokens": 18,
            "reasoningOutputTokens": 5,
            "totalTokens": 138,
        },
        "total": {
            "inputTokens": 200,
            "cachedInputTokens": 30,
            "outputTokens": 44,
            "reasoningOutputTokens": 12,
            "totalTokens": 244,
        },
    }

    events = codex_app_server.process_turn_message(
        {
            "method": "thread/tokenUsage/updated",
            "params": {
                "tokenUsage": payload,
            },
        },
        state,
    )

    assert len(events) == 1
    assert events[0].kind == "token_usage_updated"
    assert events[0].token_usage == payload
    assert state.last_token_total == 244
    assert state.last_token_usage_payload == payload


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
                {"path": "frontend/src/features/projects/ProjectsPanel.tsx"},
                {"filePath": "frontend/src/store.ts"},
            ],
        }
    )

    assert tool_call is not None
    assert tool_call.id
    assert tool_call.kind == "file_change"
    assert tool_call.status == "running"
    assert tool_call.file_paths == [
        "frontend/src/features/projects/ProjectsPanel.tsx",
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
        model="gpt-5.4",
    )

    service._write_session_state(session_state)
    loaded = service._read_session_state("conversation-test")

    assert loaded is not None
    assert loaded.conversation_id == "conversation-test"
    assert loaded.thread_id == "thread-123"
    assert loaded.model == "gpt-5.4"
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
            model="gpt-5.4",
        )
    )

    session = service._build_session("conversation-test", "/tmp/project")

    assert session._thread_id == "thread-restored"
    assert session._model == "gpt-5.4"


def test_chat_session_resumes_persisted_thread_before_starting(monkeypatch) -> None:
    updated_thread_ids: list[str] = []
    session = project_chat.CodexAppServerChatSession(
        "/tmp/project",
        persisted_thread_id="thread-existing",
        on_thread_id_updated=updated_thread_ids.append,
    )
    client = StubChatClient()
    client.resume_result = "thread-existing"
    session._client = client

    session._ensure_thread("gpt-test")

    assert len(client.resume_calls) == 1
    assert client.resume_calls[0]["thread_id"] == "thread-existing"
    assert client.resume_calls[0]["model"] == "gpt-test"
    assert client.start_calls == []
    assert session._thread_id == "thread-existing"
    assert updated_thread_ids == ["thread-existing"]


def test_chat_session_starts_new_durable_thread_when_resume_fails(monkeypatch) -> None:
    updated_thread_ids: list[str] = []
    session = project_chat.CodexAppServerChatSession(
        "/tmp/project",
        persisted_thread_id="thread-stale",
        on_thread_id_updated=updated_thread_ids.append,
    )
    client = StubChatClient()
    client.resume_result = None
    client.start_result = "thread-fresh"
    session._client = client

    session._ensure_thread("gpt-test")

    assert len(client.resume_calls) == 1
    assert len(client.start_calls) == 1
    assert client.start_calls[0]["ephemeral"] is False
    assert session._thread_id == "thread-fresh"
    assert updated_thread_ids == ["thread-fresh"]


def test_chat_session_reuses_initialized_thread_across_turns(monkeypatch) -> None:
    session = project_chat.CodexAppServerChatSession("/tmp/project")
    client = StubChatClient()
    client.resume_result = "thread-existing"
    client.run_turn_handler = lambda **kwargs: _completed_turn_result(thread_id=kwargs["thread_id"])
    session._client = client
    session._thread_id = "thread-existing"

    first = session.turn("hello", None)
    second = session.turn("again", None)

    assert first.assistant_message == "Ack"
    assert second.assistant_message == "Ack"
    assert len(client.resume_calls) == 1
    assert client.start_calls == []
    assert [call["thread_id"] for call in client.run_turn_calls] == ["thread-existing", "thread-existing"]


def test_chat_session_turn_forwards_chat_mode_to_run_turn(monkeypatch) -> None:
    session = project_chat.CodexAppServerChatSession("/tmp/project")
    client = StubChatClient()
    client.resume_result = "thread-existing"
    client.run_turn_handler = lambda **kwargs: _completed_turn_result(thread_id=kwargs["thread_id"])
    session._client = client
    session._thread_id = "thread-existing"

    result = session.turn("hello", None, chat_mode="plan")

    assert result.assistant_message == "Ack"
    assert client.run_turn_calls[0]["chat_mode"] == "plan"


def test_chat_session_uses_plan_text_when_turn_has_only_plan_item(monkeypatch) -> None:
    session = project_chat.CodexAppServerChatSession("/tmp/project")
    progress_updates: list[project_chat.ChatTurnLiveEvent] = []
    client = StubChatClient()
    client.resume_result = "thread-existing"
    session._client = client
    session._thread_id = "thread-existing"

    def run_turn(**kwargs) -> CodexAppServerTurnResult:
        on_event = kwargs["on_event"]
        on_event(
            codex_app_server.CodexAppServerTurnEvent(
                kind="plan_delta",
                text="1. Capture the plan-only session path.\n",
                item_id="plan-1",
            )
        )
        on_event(
            codex_app_server.CodexAppServerTurnEvent(
                kind="plan_completed",
                text="1. Capture the plan-only session path.\n2. Validate the fix.",
                item_id="plan-1",
            )
        )
        return _completed_turn_result(
            thread_id=kwargs["thread_id"],
            assistant_message="",
            plan_message="1. Capture the plan-only session path.\n2. Validate the fix.",
        )

    client.run_turn_handler = run_turn

    result = session.turn("hello", None, chat_mode="plan", on_event=progress_updates.append)

    assert result.assistant_message == "1. Capture the plan-only session path.\n2. Validate the fix."
    assert [event.kind for event in progress_updates] == ["plan_delta", "plan_completed"]


def test_chat_session_surfaces_reasoning_summary_progress(monkeypatch) -> None:
    session = project_chat.CodexAppServerChatSession("/tmp/project")
    progress_updates: list[project_chat.ChatTurnLiveEvent] = []
    client = StubChatClient()
    session._client = client

    def run_turn(**kwargs) -> CodexAppServerTurnResult:
        on_event = kwargs["on_event"]
        on_event(
            codex_app_server.CodexAppServerTurnEvent(
                kind="reasoning_delta",
                text="Scanning the repository structure.",
            )
        )
        on_event(
            codex_app_server.CodexAppServerTurnEvent(
                kind="assistant_delta",
                text="I found the main entry points.",
            )
        )
        on_event(
            codex_app_server.CodexAppServerTurnEvent(
                kind="assistant_message_completed",
                text="I found the main entry points.",
                item_id="msg-123",
                phase="final_answer",
            )
        )
        return _completed_turn_result(
            thread_id=kwargs["thread_id"],
            assistant_message="I found the main entry points.",
        )

    client.run_turn_handler = run_turn

    result = session.turn("hello", None, on_event=progress_updates.append)

    assert result.assistant_message == "I found the main entry points."
    assert any(
        update.kind == "reasoning_summary" and update.content_delta == "Scanning the repository structure."
        for update in progress_updates
    )


def test_chat_session_surfaces_reasoning_summary_text_deltas(monkeypatch) -> None:
    session = project_chat.CodexAppServerChatSession("/tmp/project")
    progress_updates: list[project_chat.ChatTurnLiveEvent] = []
    client = StubChatClient()
    session._client = client

    def run_turn(**kwargs) -> CodexAppServerTurnResult:
        on_event = kwargs["on_event"]
        on_event(
            codex_app_server.CodexAppServerTurnEvent(
                kind="reasoning_delta",
                text="Draft draft that minimal proposal a best think",
            )
        )
        on_event(
            codex_app_server.CodexAppServerTurnEvent(
                kind="assistant_delta",
                text="I’m checking the project structure first.",
            )
        )
        on_event(
            codex_app_server.CodexAppServerTurnEvent(
                kind="assistant_message_completed",
                text="I’m checking the project structure first.",
                item_id="msg-123",
                phase="final_answer",
            )
        )
        return _completed_turn_result(
            thread_id=kwargs["thread_id"],
            assistant_message="I’m checking the project structure first.",
        )

    client.run_turn_handler = run_turn

    result = session.turn("hello", None, on_event=progress_updates.append)

    assert result.assistant_message == "I’m checking the project structure first."
    assert any(
        update.kind == "reasoning_summary"
        and update.content_delta == "Draft draft that minimal proposal a best think"
        for update in progress_updates
    )


def test_chat_session_surfaces_context_compaction_progress(monkeypatch) -> None:
    session = project_chat.CodexAppServerChatSession("/tmp/project")
    progress_updates: list[project_chat.ChatTurnLiveEvent] = []
    client = StubChatClient()
    session._client = client

    def run_turn(**kwargs) -> CodexAppServerTurnResult:
        on_turn_started = kwargs["on_turn_started"]
        on_turn_started("turn-123")
        on_event = kwargs["on_event"]
        on_event(
            codex_app_server.CodexAppServerTurnEvent(
                kind="context_compaction_started",
                item_id="compact-1",
            )
        )
        on_event(
            codex_app_server.CodexAppServerTurnEvent(
                kind="context_compaction_completed",
                item_id="compact-1",
            )
        )
        on_event(
            codex_app_server.CodexAppServerTurnEvent(
                kind="assistant_message_completed",
                text="Ack",
                item_id="msg-1",
                phase="final_answer",
            )
        )
        return _completed_turn_result(thread_id=kwargs["thread_id"], assistant_message="Ack")

    client.run_turn_handler = run_turn

    result = session.turn("hello", None, on_event=progress_updates.append)

    assert result.assistant_message == "Ack"
    assert [event.kind for event in progress_updates] == [
        "context_compaction_started",
        "context_compaction_completed",
        "assistant_completed",
    ]
    assert progress_updates[0].app_turn_id == "turn-123"
    assert progress_updates[0].item_id == "compact-1"
    assert progress_updates[1].app_turn_id == "turn-123"
    assert progress_updates[1].item_id == "compact-1"


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


def test_send_turn_marks_assistant_failed_after_timeout_without_retry(tmp_path: Path, monkeypatch) -> None:
    service = project_chat.ProjectChatService(tmp_path)
    calls: list[str] = []

    class TimeoutSession:
        def turn(
            self,
            prompt: str,
            model: str | None,
            *,
            chat_mode: str = "chat",
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


def test_send_turn_starts_new_thread_cleanly_when_persisted_resume_fails(tmp_path: Path) -> None:
    service = project_chat.ProjectChatService(tmp_path)
    conversation_id = "conversation-test"
    project_path = str(tmp_path.resolve())
    service._write_state(
        project_chat.ConversationState(
            conversation_id=conversation_id,
            project_path=project_path,
            conversation_handle="amber-otter",
            turns=[
                project_chat.ConversationTurn(
                    id="turn-user-1",
                    role="user",
                    content="Older message",
                    timestamp="2026-03-08T12:00:00Z",
                ),
                project_chat.ConversationTurn(
                    id="turn-assistant-1",
                    role="assistant",
                    content="Older reply",
                    timestamp="2026-03-08T12:01:00Z",
                    status="complete",
                ),
            ],
        )
    )
    service._write_session_state(
        project_chat.ConversationSessionState(
            conversation_id=conversation_id,
            updated_at="2026-03-08T12:02:00Z",
            project_path=project_path,
            runtime_project_path=project_path,
            thread_id="thread-stale",
        )
    )

    session = service._build_session(conversation_id, project_path)
    client = StubChatClient()
    client.resume_result = None
    client.start_result = "thread-fresh"
    captured_prompts: list[str] = []

    def run_turn(**kwargs) -> CodexAppServerTurnResult:
        captured_prompts.append(kwargs["prompt"])
        kwargs["on_event"](
            codex_app_server.CodexAppServerTurnEvent(
                kind="assistant_message_completed",
                text="Ack",
                item_id="msg-1",
                phase="final_answer",
            )
        )
        return _completed_turn_result(thread_id=kwargs["thread_id"], assistant_message="Ack")

    client.run_turn_handler = run_turn
    session._client = client

    snapshot = service.send_turn(conversation_id, project_path, "Latest message", None)

    assert snapshot["turns"][-1]["role"] == "assistant"
    assert snapshot["turns"][-1]["status"] == "complete"
    assert snapshot["turns"][-1]["content"] == "Ack"
    assert len(client.resume_calls) == 1
    assert client.resume_calls[0]["thread_id"] == "thread-stale"
    assert len(client.start_calls) == 1
    assert client.start_calls[0]["ephemeral"] is False
    assert session._thread_id == "thread-fresh"
    assert len(captured_prompts) == 1
    assert "Latest user message:\nLatest message" in captured_prompts[0]
    assert "Older message" not in captured_prompts[0]
    assert "Older reply" not in captured_prompts[0]


def test_send_turn_accepts_plain_text_final_response(tmp_path: Path, monkeypatch) -> None:
    service = project_chat.ProjectChatService(tmp_path)

    class PlainTextSession:
        def turn(
            self,
            prompt: str,
            model: str | None,
            *,
            chat_mode: str = "chat",
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


def test_update_conversation_settings_upserts_shell_and_persists_chat_mode(tmp_path: Path) -> None:
    service = project_chat.ProjectChatService(tmp_path)

    snapshot = service.update_conversation_settings("conversation-settings", str(tmp_path), "plan")

    assert snapshot["conversation_id"] == "conversation-settings"
    assert snapshot["chat_mode"] == "plan"
    assert snapshot["turns"] == [
        {
            "id": snapshot["turns"][0]["id"],
            "role": "system",
            "content": "plan",
            "timestamp": snapshot["turns"][0]["timestamp"],
            "status": "complete",
            "kind": "mode_change",
        }
    ]
    reloaded = service.get_snapshot("conversation-settings", str(tmp_path))
    assert reloaded["chat_mode"] == "plan"
    assert [turn["kind"] for turn in reloaded["turns"]] == ["mode_change"]


def test_update_conversation_settings_does_not_duplicate_mode_change_when_mode_matches(tmp_path: Path) -> None:
    service = project_chat.ProjectChatService(tmp_path)

    first = service.update_conversation_settings("conversation-settings", str(tmp_path), "plan")
    second = service.update_conversation_settings("conversation-settings", str(tmp_path), "plan")

    assert first["chat_mode"] == "plan"
    assert second["chat_mode"] == "plan"
    assert [turn["kind"] for turn in second["turns"]] == ["mode_change"]
    assert second["turns"][0]["content"] == "plan"


def test_send_turn_persists_mode_change_turn_before_user_turn_when_chat_mode_changes(
    tmp_path: Path,
    monkeypatch,
) -> None:
    service = project_chat.ProjectChatService(tmp_path)
    captured_chat_modes: list[str] = []

    class PlainTextSession:
        def turn(
            self,
            prompt: str,
            model: str | None,
            *,
            chat_mode: str = "chat",
            on_event=None,
            on_dynamic_tool_call=None,
        ) -> project_chat.ChatTurnResult:
            captured_chat_modes.append(chat_mode)
            if on_event is not None:
                on_event(
                    project_chat.ChatTurnLiveEvent(
                        kind="assistant_completed",
                        content_delta="Plan mode acknowledged.",
                        app_turn_id="app-turn-1",
                        item_id="msg-1",
                        phase="final_answer",
                    )
                )
            return project_chat.ChatTurnResult(assistant_message="Plan mode acknowledged.")

        def _close(self) -> None:
            return None

    monkeypatch.setattr(service, "_build_session", lambda conversation_id, project_path: PlainTextSession())

    snapshot = service.send_turn(
        "conversation-mode-switch",
        str(tmp_path),
        "Acknowledge the plan mode switch.",
        None,
        "plan",
    )

    assert snapshot["chat_mode"] == "plan"
    assert [turn["kind"] for turn in snapshot["turns"]] == ["mode_change", "message", "message"]
    assert [turn["role"] for turn in snapshot["turns"]] == ["system", "user", "assistant"]
    assert snapshot["turns"][0]["content"] == "plan"
    assert snapshot["turns"][1]["content"] == "Acknowledge the plan mode switch."
    assert snapshot["turns"][2]["content"] == "Plan mode acknowledged."
    assert captured_chat_modes == ["plan"]


def test_send_turn_uses_persisted_plan_chat_mode_for_execution(tmp_path: Path, monkeypatch) -> None:
    service = project_chat.ProjectChatService(tmp_path)
    captured_chat_modes: list[str] = []

    class PlainTextSession:
        def turn(
            self,
            prompt: str,
            model: str | None,
            *,
            chat_mode: str = "chat",
            on_event=None,
            on_dynamic_tool_call=None,
        ) -> project_chat.ChatTurnResult:
            captured_chat_modes.append(chat_mode)
            if on_event is not None:
                on_event(
                    project_chat.ChatTurnLiveEvent(
                        kind="assistant_completed",
                        content_delta="Still in plan mode.",
                        app_turn_id="app-turn-1",
                        item_id="msg-1",
                        phase="final_answer",
                    )
                )
            return project_chat.ChatTurnResult(assistant_message="Still in plan mode.")

    service.update_conversation_settings("conversation-persisted-plan", str(tmp_path), "plan")
    monkeypatch.setattr(service, "_build_session", lambda conversation_id, project_path: PlainTextSession())

    snapshot = service.send_turn(
        "conversation-persisted-plan",
        str(tmp_path),
        "Continue the plan.",
        None,
    )

    assert snapshot["chat_mode"] == "plan"
    assert captured_chat_modes == ["plan"]


def test_send_turn_completes_plan_only_real_session_path_with_plan_preview_fallback(tmp_path: Path) -> None:
    service = project_chat.ProjectChatService(tmp_path)
    conversation_id = "conversation-plan-only"
    project_path = str(tmp_path.resolve())
    session = service._build_session(conversation_id, project_path)
    client = StubChatClient()
    client.start_result = "thread-plan"

    def run_turn(**kwargs) -> CodexAppServerTurnResult:
        on_event = kwargs["on_event"]
        on_event(
            codex_app_server.CodexAppServerTurnEvent(
                kind="plan_delta",
                text="1. Patch the real session path.\n",
                item_id="plan-1",
            )
        )
        on_event(
            codex_app_server.CodexAppServerTurnEvent(
                kind="plan_completed",
                text="1. Patch the real session path.\n2. Add the regression coverage.",
                item_id="plan-1",
            )
        )
        return _completed_turn_result(
            thread_id=kwargs["thread_id"],
            assistant_message="",
            plan_message="1. Patch the real session path.\n2. Add the regression coverage.",
        )

    client.run_turn_handler = run_turn
    session._client = client

    snapshot = service.send_turn(
        conversation_id,
        project_path,
        "Plan the remaining fixes.",
        None,
        "plan",
    )

    assistant_turn = snapshot["turns"][-1]
    assert snapshot["chat_mode"] == "plan"
    assert assistant_turn["role"] == "assistant"
    assert assistant_turn["status"] == "complete"
    assert assistant_turn["content"] == "1. Patch the real session path.\n2. Add the regression coverage."
    assert [segment["kind"] for segment in snapshot["segments"]] == ["plan"]
    assert snapshot["segments"][0]["content"] == "1. Patch the real session path.\n2. Add the regression coverage."
    assert snapshot["segments"][0]["status"] == "complete"
    summaries = service.list_conversations(project_path)
    assert summaries[0]["last_message_preview"] == "1. Patch the real session path.\n2. Add the regression coverage."


def test_send_turn_buffers_plan_mode_assistant_completion_without_leaking_markup(
    tmp_path: Path,
    monkeypatch,
) -> None:
    service = project_chat.ProjectChatService(tmp_path)
    progress_updates: list[dict[str, Any]] = []
    raw_plan_markup = "<proposed_plan>\n1. Patch the real session path.\n2. Add the regression coverage.\n</proposed_plan>"

    class PlanSession:
        def turn(
            self,
            prompt: str,
            model: str | None,
            *,
            chat_mode: str = "chat",
            on_event=None,
            on_dynamic_tool_call=None,
        ) -> project_chat.ChatTurnResult:
            assert chat_mode == "plan"
            if on_event is not None:
                on_event(
                    project_chat.ChatTurnLiveEvent(
                        kind="reasoning_summary",
                        content_delta="Checking the active session path.",
                    )
                )
                on_event(
                    project_chat.ChatTurnLiveEvent(
                        kind="assistant_completed",
                        content_delta=raw_plan_markup,
                        app_turn_id="app-turn-1",
                        item_id="msg-1",
                        phase="final_answer",
                    )
                )
                on_event(
                    project_chat.ChatTurnLiveEvent(
                        kind="plan_delta",
                        content_delta="1. Patch the real session path.\n",
                        app_turn_id="app-turn-1",
                        item_id="plan-1",
                    )
                )
                on_event(
                    project_chat.ChatTurnLiveEvent(
                        kind="plan_completed",
                        content_delta="1. Patch the real session path.\n2. Add the regression coverage.",
                        app_turn_id="app-turn-1",
                        item_id="plan-1",
                    )
                )
            return project_chat.ChatTurnResult(assistant_message=raw_plan_markup)

    monkeypatch.setattr(service, "_build_session", lambda conversation_id, project_path: PlanSession())

    snapshot = service.send_turn(
        "conversation-buffered-plan",
        str(tmp_path),
        "Plan the remaining fixes.",
        None,
        "plan",
        progress_callback=progress_updates.append,
    )

    live_segment_kinds = [
        payload["segment"]["kind"]
        for payload in progress_updates
        if payload.get("type") == "segment_upsert"
    ]
    assert "assistant_message" not in live_segment_kinds
    assert "plan" in live_segment_kinds
    assert snapshot["turns"][-1]["content"] == "1. Patch the real session path.\n2. Add the regression coverage."
    assert [segment["kind"] for segment in snapshot["segments"]] == ["reasoning", "plan"]
    assert all("<proposed_plan>" not in segment["content"] for segment in snapshot["segments"])


def test_send_turn_persists_context_compaction_segment_transition(
    tmp_path: Path,
    monkeypatch,
) -> None:
    service = project_chat.ProjectChatService(tmp_path)
    progress_updates: list[dict[str, Any]] = []

    class CompactionSession:
        def turn(
            self,
            prompt: str,
            model: str | None,
            *,
            chat_mode: str = "chat",
            on_event=None,
            on_dynamic_tool_call=None,
        ) -> project_chat.ChatTurnResult:
            if on_event is not None:
                on_event(
                    project_chat.ChatTurnLiveEvent(
                        kind="context_compaction_started",
                        app_turn_id="app-turn-1",
                        item_id="compact-1",
                    )
                )
                on_event(
                    project_chat.ChatTurnLiveEvent(
                        kind="context_compaction_completed",
                        app_turn_id="app-turn-1",
                        item_id="compact-1",
                    )
                )
                on_event(
                    project_chat.ChatTurnLiveEvent(
                        kind="assistant_completed",
                        content_delta="Ack",
                        app_turn_id="app-turn-1",
                        item_id="msg-1",
                        phase="final_answer",
                    )
                )
            return project_chat.ChatTurnResult(assistant_message="Ack")

    monkeypatch.setattr(service, "_build_session", lambda conversation_id, project_path: CompactionSession())

    snapshot = service.send_turn(
        "conversation-context-compaction",
        str(tmp_path),
        "Continue the turn.",
        None,
        progress_callback=progress_updates.append,
    )

    compaction_segments = [segment for segment in snapshot["segments"] if segment["kind"] == "context_compaction"]
    compaction_payloads = [
        payload["segment"]
        for payload in progress_updates
        if payload.get("type") == "segment_upsert" and payload["segment"]["kind"] == "context_compaction"
    ]

    assert len(compaction_segments) == 1
    assert compaction_segments[0]["status"] == "complete"
    assert compaction_segments[0]["content"] == "Context compacted to continue the turn."
    assert compaction_segments[0]["source"] == {
        "app_turn_id": "app-turn-1",
        "item_id": "compact-1",
    }
    assert [payload["status"] for payload in compaction_payloads] == ["running", "complete"]
    assert [payload["content"] for payload in compaction_payloads] == [
        "Compacting conversation context…",
        "Context compacted to continue the turn.",
    ]


def test_send_turn_persists_context_compaction_from_thread_compacted_fallback(
    tmp_path: Path,
    monkeypatch,
) -> None:
    service = project_chat.ProjectChatService(tmp_path)

    class CompactionFallbackSession:
        def turn(
            self,
            prompt: str,
            model: str | None,
            *,
            chat_mode: str = "chat",
            on_event=None,
            on_dynamic_tool_call=None,
        ) -> project_chat.ChatTurnResult:
            if on_event is not None:
                on_event(
                    project_chat.ChatTurnLiveEvent(
                        kind="context_compaction_completed",
                        app_turn_id="app-turn-1",
                    )
                )
                on_event(
                    project_chat.ChatTurnLiveEvent(
                        kind="assistant_completed",
                        content_delta="Ack",
                        app_turn_id="app-turn-1",
                        item_id="msg-1",
                        phase="final_answer",
                    )
                )
            return project_chat.ChatTurnResult(assistant_message="Ack")

    monkeypatch.setattr(service, "_build_session", lambda conversation_id, project_path: CompactionFallbackSession())

    snapshot = service.send_turn(
        "conversation-context-compaction-fallback",
        str(tmp_path),
        "Continue the turn.",
        None,
    )

    compaction_segments = [segment for segment in snapshot["segments"] if segment["kind"] == "context_compaction"]

    assert len(compaction_segments) == 1
    assert compaction_segments[0]["status"] == "complete"
    assert compaction_segments[0]["content"] == "Context compacted to continue the turn."
    assert compaction_segments[0]["source"] == {
        "app_turn_id": "app-turn-1",
    }


def test_send_turn_deduplicates_context_compaction_duplicate_completion_signals(
    tmp_path: Path,
    monkeypatch,
) -> None:
    service = project_chat.ProjectChatService(tmp_path)
    progress_updates: list[dict[str, Any]] = []

    class DuplicateCompactionSession:
        def turn(
            self,
            prompt: str,
            model: str | None,
            *,
            chat_mode: str = "chat",
            on_event=None,
            on_dynamic_tool_call=None,
        ) -> project_chat.ChatTurnResult:
            if on_event is not None:
                on_event(
                    project_chat.ChatTurnLiveEvent(
                        kind="context_compaction_started",
                        app_turn_id="app-turn-1",
                        item_id="compact-1",
                    )
                )
                on_event(
                    project_chat.ChatTurnLiveEvent(
                        kind="context_compaction_completed",
                        app_turn_id="app-turn-1",
                        item_id="compact-1",
                    )
                )
                on_event(
                    project_chat.ChatTurnLiveEvent(
                        kind="context_compaction_completed",
                        app_turn_id="app-turn-1",
                    )
                )
                on_event(
                    project_chat.ChatTurnLiveEvent(
                        kind="assistant_completed",
                        content_delta="Ack",
                        app_turn_id="app-turn-1",
                        item_id="msg-1",
                        phase="final_answer",
                    )
                )
            return project_chat.ChatTurnResult(assistant_message="Ack")

    monkeypatch.setattr(service, "_build_session", lambda conversation_id, project_path: DuplicateCompactionSession())

    snapshot = service.send_turn(
        "conversation-context-compaction-deduped",
        str(tmp_path),
        "Continue the turn.",
        None,
        progress_callback=progress_updates.append,
    )

    compaction_segments = [segment for segment in snapshot["segments"] if segment["kind"] == "context_compaction"]
    compaction_payloads = [
        payload["segment"]
        for payload in progress_updates
        if payload.get("type") == "segment_upsert" and payload["segment"]["kind"] == "context_compaction"
    ]

    assert len(compaction_segments) == 1
    assert compaction_segments[0]["status"] == "complete"
    assert compaction_segments[0]["content"] == "Context compacted to continue the turn."
    assert len(compaction_payloads) == 2
    assert [payload["status"] for payload in compaction_payloads] == ["running", "complete"]


def test_send_turn_persists_plan_mode_assistant_remainder_after_completion(
    tmp_path: Path,
    monkeypatch,
) -> None:
    service = project_chat.ProjectChatService(tmp_path)
    progress_updates: list[dict[str, Any]] = []
    raw_response = (
        "I checked the repository state.\n\n"
        "<proposed_plan>\n"
        "1. Patch the real session path.\n"
        "2. Add the regression coverage.\n"
        "</proposed_plan>\n\n"
        "After that, validate with uv run pytest -q."
    )
    expected_remainder = "I checked the repository state.\n\nAfter that, validate with uv run pytest -q."

    class PlanSession:
        def turn(
            self,
            prompt: str,
            model: str | None,
            *,
            chat_mode: str = "chat",
            on_event=None,
            on_dynamic_tool_call=None,
        ) -> project_chat.ChatTurnResult:
            assert chat_mode == "plan"
            if on_event is not None:
                on_event(
                    project_chat.ChatTurnLiveEvent(
                        kind="assistant_completed",
                        content_delta=raw_response,
                        app_turn_id="app-turn-1",
                        item_id="msg-1",
                        phase="final_answer",
                    )
                )
                on_event(
                    project_chat.ChatTurnLiveEvent(
                        kind="plan_completed",
                        content_delta="1. Patch the real session path.\n2. Add the regression coverage.",
                        app_turn_id="app-turn-1",
                        item_id="plan-1",
                    )
                )
            return project_chat.ChatTurnResult(assistant_message=raw_response)

    monkeypatch.setattr(service, "_build_session", lambda conversation_id, project_path: PlanSession())

    snapshot = service.send_turn(
        "conversation-plan-remainder",
        str(tmp_path),
        "Plan the remaining fixes.",
        None,
        "plan",
        progress_callback=progress_updates.append,
    )

    assistant_segments = [segment for segment in snapshot["segments"] if segment["kind"] == "assistant_message"]
    assistant_payloads = [
        payload["segment"]
        for payload in progress_updates
        if payload.get("type") == "segment_upsert" and payload["segment"]["kind"] == "assistant_message"
    ]

    assert snapshot["turns"][-1]["content"] == expected_remainder
    assert [segment["kind"] for segment in snapshot["segments"]] == ["plan", "assistant_message"]
    assert len(assistant_segments) == 1
    assert assistant_segments[0]["content"] == expected_remainder
    assert len(assistant_payloads) == 1
    assert assistant_payloads[0]["content"] == expected_remainder
    assert "<proposed_plan>" not in assistant_segments[0]["content"]


def test_send_turn_passes_default_chat_mode_for_execution(tmp_path: Path, monkeypatch) -> None:
    service = project_chat.ProjectChatService(tmp_path)
    captured_chat_modes: list[str] = []

    class PlainTextSession:
        def turn(
            self,
            prompt: str,
            model: str | None,
            *,
            chat_mode: str = "chat",
            on_event=None,
            on_dynamic_tool_call=None,
        ) -> project_chat.ChatTurnResult:
            captured_chat_modes.append(chat_mode)
            if on_event is not None:
                on_event(
                    project_chat.ChatTurnLiveEvent(
                        kind="assistant_completed",
                        content_delta="Default chat mode.",
                        app_turn_id="app-turn-1",
                        item_id="msg-1",
                        phase="final_answer",
                    )
                )
            return project_chat.ChatTurnResult(assistant_message="Default chat mode.")

    monkeypatch.setattr(service, "_build_session", lambda conversation_id, project_path: PlainTextSession())

    snapshot = service.send_turn(
        "conversation-default-chat",
        str(tmp_path),
        "Say hello.",
        None,
    )

    assert snapshot["chat_mode"] == "chat"
    assert captured_chat_modes == ["chat"]


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
            "flow_name": TEST_DISPATCH_FLOW,
            "summary": "Run implementation for the approved scope.",
            "goal": "Implement the approved scope.",
            "launch_context": {
                "context.request.summary": "Implement the approved scope.",
                "context.request.target_paths": ["src/spark/workspace", "tests/api"],
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
    assert snapshot["flow_run_requests"][0]["flow_name"] == TEST_DISPATCH_FLOW
    assert snapshot["flow_run_requests"][0]["summary"] == "Run implementation for the approved scope."
    assert snapshot["flow_run_requests"][0]["launch_context"] == {
        "context.request.summary": "Implement the approved scope.",
        "context.request.target_paths": ["src/spark/workspace", "tests/api"],
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
    service = _project_chat_service()
    service._write_state(state)
    snapshot = service.get_snapshot(conversation_id, str(project_dir))

    _seed_flow(TEST_DISPATCH_FLOW)

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
            "flow_name": TEST_DISPATCH_FLOW,
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
            "flow_name": TEST_DISPATCH_FLOW,
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
    service = _project_chat_service()
    service._write_state(state)
    snapshot = service.get_snapshot(conversation_id, str(project_dir))

    _seed_flow(TEST_DISPATCH_FLOW)

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
            "flow_name": TEST_DISPATCH_FLOW,
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

    updated_snapshot = _project_chat_service().get_snapshot(conversation_id, str(project_dir))
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
            "flow_name": TEST_DISPATCH_FLOW,
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


def test_direct_flow_launch_uses_flow_ui_default_model_when_request_model_missing(
    product_api_client,
    tmp_path: Path,
) -> None:
    project_dir = tmp_path / "project"
    project_dir.mkdir(parents=True, exist_ok=True)
    flow_name = "test-ui-default-model.dot"
    flows_dir = product_app.get_settings().flows_dir
    flows_dir.mkdir(parents=True, exist_ok=True)
    (flows_dir / flow_name).write_text(
        """
        digraph G {
            graph [ui_default_llm_model="gpt-flow-default"]
            start [shape=Mdiamond]
            done [shape=Msquare]
            start -> done
        }
        """.strip()
        + "\n",
        encoding="utf-8",
    )

    launch_response = product_api_client.post(
        "/workspace/api/runs/launch",
        json={
            "flow_name": flow_name,
            "summary": "Launch a flow without an explicit model override.",
            "project_path": str(project_dir),
        },
    )

    assert launch_response.status_code == 200
    launch_payload = launch_response.json()
    run_id = launch_payload["run_id"]

    pipeline_payload: dict[str, object] = {}
    for _ in range(200):
        pipeline_response = product_api_client.get(f"/attractor/pipelines/{run_id}")
        assert pipeline_response.status_code == 200
        pipeline_payload = pipeline_response.json()
        if pipeline_payload["status"] != "running":
            break
        time.sleep(0.01)

    assert pipeline_payload["status"] == "completed"
    assert pipeline_payload["model"] == "gpt-flow-default"


def test_chat_session_emits_assistant_completed_from_item_completed(monkeypatch) -> None:
    session = project_chat.CodexAppServerChatSession("/tmp/project")
    captured_events: list[project_chat.ChatTurnLiveEvent] = []
    client = StubChatClient()
    session._client = client

    def run_turn(**kwargs) -> CodexAppServerTurnResult:
        on_event = kwargs["on_event"]
        on_event(codex_app_server.CodexAppServerTurnEvent(kind="assistant_delta", text="Ack"))
        on_event(
            codex_app_server.CodexAppServerTurnEvent(
                kind="assistant_message_completed",
                text="Ack",
                item_id="msg-1",
                phase="final_answer",
            )
        )
        return _completed_turn_result(thread_id=kwargs["thread_id"], assistant_message="Ack")

    client.run_turn_handler = run_turn

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
    captured_events: list[project_chat.ChatTurnLiveEvent] = []
    client = StubChatClient()
    session._client = client

    def run_turn(**kwargs) -> CodexAppServerTurnResult:
        on_event = kwargs["on_event"]
        on_event(
            codex_app_server.CodexAppServerTurnEvent(
                kind="reasoning_delta",
                text="Thinking...",
                item_id="rs-1",
                summary_index=0,
            )
        )
        on_event(
            codex_app_server.CodexAppServerTurnEvent(
                kind="command_output_delta",
                text="output",
                item_id="cmd-1",
            )
        )
        on_event(
            codex_app_server.CodexAppServerTurnEvent(
                kind="assistant_delta",
                text="Ack",
                item_id="msg-1",
            )
        )
        on_event(
            codex_app_server.CodexAppServerTurnEvent(
                kind="assistant_message_completed",
                text="Ack",
                item_id="msg-1",
                phase="final_answer",
            )
        )
        return _completed_turn_result(thread_id=kwargs["thread_id"], assistant_message="Ack")

    client.run_turn_handler = run_turn

    result = session.turn(
        "hello",
        None,
        on_event=lambda event: captured_events.append(event),
    )

    assert result.assistant_message == "Ack"
    assert [event.kind for event in captured_events] == ["reasoning_summary", "assistant_delta", "assistant_completed"]


def test_chat_session_emits_assistant_completed_for_commentary_item(monkeypatch) -> None:
    session = project_chat.CodexAppServerChatSession("/tmp/project")
    captured_events: list[project_chat.ChatTurnLiveEvent] = []
    client = StubChatClient()
    session._client = client

    def run_turn(**kwargs) -> CodexAppServerTurnResult:
        on_event = kwargs["on_event"]
        on_event(
            codex_app_server.CodexAppServerTurnEvent(
                kind="assistant_delta",
                text="I’m drafting the proposal now.",
                item_id="msg-1",
            )
        )
        on_event(
            codex_app_server.CodexAppServerTurnEvent(
                kind="assistant_message_completed",
                text="I’m drafting the proposal now.",
                item_id="msg-1",
                phase="commentary",
            )
        )
        on_event(
            codex_app_server.CodexAppServerTurnEvent(
                kind="assistant_delta",
                text="Done.",
                item_id="msg-2",
            )
        )
        on_event(
            codex_app_server.CodexAppServerTurnEvent(
                kind="assistant_message_completed",
                text="Done.",
                item_id="msg-2",
                phase="final_answer",
            )
        )
        return _completed_turn_result(thread_id=kwargs["thread_id"], assistant_message="Done.")

    client.run_turn_handler = run_turn

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

    def fake_init(
        self,
        working_dir: str,
        *,
        persisted_thread_id=None,
        persisted_model=None,
        on_thread_id_updated=None,
        on_model_updated=None,
    ):
        captured["working_dir"] = working_dir
        captured["persisted_thread_id"] = persisted_thread_id
        captured["persisted_model"] = persisted_model
        captured["on_thread_id_updated"] = on_thread_id_updated
        captured["on_model_updated"] = on_model_updated

    monkeypatch.setattr(project_chat.CodexAppServerChatSession, "__init__", fake_init)

    service._build_session(conversation_id, project_path)

    assert captured["working_dir"] == project_path
    assert captured["persisted_thread_id"] is None
    assert captured["persisted_model"] is None


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
            chat_mode: str = "chat",
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
            chat_mode: str = "chat",
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


def test_conversation_titles_and_previews_ignore_mode_change_turns(tmp_path: Path) -> None:
    service = project_chat.ProjectChatService(tmp_path)
    project_path = str(tmp_path / "project-a")

    service._write_state(
        project_chat.ConversationState(
            conversation_id="conversation-mode-summary",
            project_path=project_path,
            title="",
            created_at="2026-03-07T13:00:00Z",
            updated_at="2026-03-07T13:05:00Z",
            turns=[
                project_chat.ConversationTurn(
                    id="turn-mode-1",
                    role="system",
                    content="plan",
                    timestamp="2026-03-07T13:00:00Z",
                    kind="mode_change",
                ),
                project_chat.ConversationTurn(
                    id="turn-user-1",
                    role="user",
                    content="Design thread title",
                    timestamp="2026-03-07T13:01:00Z",
                ),
                project_chat.ConversationTurn(
                    id="turn-assistant-1",
                    role="assistant",
                    content="Design thread preview",
                    timestamp="2026-03-07T13:02:00Z",
                ),
                project_chat.ConversationTurn(
                    id="turn-mode-2",
                    role="system",
                    content="chat",
                    timestamp="2026-03-07T13:03:00Z",
                    kind="mode_change",
                ),
            ],
        )
    )

    summary = service.list_conversations(project_path)[0]

    assert summary["title"] == "Design thread title"
    assert summary["last_message_preview"] == "Design thread preview"


def test_send_project_conversation_turn_endpoint_uses_real_service_signature(
    product_api_client,
    monkeypatch,
    tmp_path: Path,
) -> None:
    service = _project_chat_service()
    entered_turn = threading.Event()
    finish_turn = threading.Event()

    class FakeSession:
        def turn(
            self,
            prompt: str,
            model: str | None,
            *,
            chat_mode: str = "chat",
            on_event=None,
            on_dynamic_tool_call=None,
        ) -> project_chat.ChatTurnResult:
            entered_turn.set()
            assert finish_turn.wait(timeout=2)
            if on_event is not None:
                on_event(
                    project_chat.ChatTurnLiveEvent(
                        kind="reasoning_summary",
                        content_delta="Checking whether a flow run request makes sense.",
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
                assistant_message='{"assistant_message":"ACK","flow_run_request":null}',
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


def test_update_project_conversation_settings_endpoint_upserts_shell_and_rejects_project_mismatch(
    product_api_client,
    tmp_path: Path,
) -> None:
    project_path = str(tmp_path.resolve())
    response = product_api_client.put(
        "/workspace/api/conversations/conversation-settings/settings",
        json={
            "project_path": project_path,
            "chat_mode": "plan",
        },
    )

    assert response.status_code == 200
    payload = response.json()
    assert payload["conversation_id"] == "conversation-settings"
    assert payload["chat_mode"] == "plan"
    assert [turn["kind"] for turn in payload["turns"]] == ["mode_change"]
    assert payload["turns"][0]["role"] == "system"
    assert payload["turns"][0]["content"] == "plan"

    service = _project_chat_service()
    mismatch_response = product_api_client.put(
        "/workspace/api/conversations/conversation-settings/settings",
        json={
            "project_path": str((tmp_path / "other-project").resolve()),
            "chat_mode": "chat",
        },
    )

    assert mismatch_response.status_code == 400
    assert "different project path" in mismatch_response.json()["detail"]
    assert service.get_snapshot("conversation-settings", project_path)["chat_mode"] == "plan"


def test_send_project_conversation_turn_endpoint_switches_chat_mode_atomically(
    product_api_client,
    monkeypatch,
    tmp_path: Path,
) -> None:
    service = _project_chat_service()

    class PlainTextSession:
        def turn(
            self,
            prompt: str,
            model: str | None,
            *,
            chat_mode: str = "chat",
            on_event=None,
            on_dynamic_tool_call=None,
        ) -> project_chat.ChatTurnResult:
            if on_event is not None:
                on_event(
                    project_chat.ChatTurnLiveEvent(
                        kind="assistant_completed",
                        content_delta="Mode switch acknowledged.",
                        app_turn_id="app-turn-1",
                        item_id="msg-1",
                        phase="final_answer",
                    )
                )
            return project_chat.ChatTurnResult(assistant_message="Mode switch acknowledged.")

    monkeypatch.setattr(service, "_build_session", lambda conversation_id, project_path: PlainTextSession())

    response = product_api_client.post(
        "/workspace/api/conversations/conversation-atomic/turns",
        json={
            "project_path": str(tmp_path),
            "message": "Plan the mode switch.",
            "chat_mode": "plan",
        },
    )

    assert response.status_code == 200
    payload = response.json()
    assert payload["chat_mode"] == "plan"

    deadline = time.time() + 2.0
    final_snapshot: dict[str, object] | None = None
    while time.time() < deadline:
        candidate = service.get_snapshot("conversation-atomic", str(tmp_path))
        if candidate["turns"][-1]["status"] == "complete":
            final_snapshot = candidate
            break
        time.sleep(0.02)

    assert final_snapshot is not None
    assert final_snapshot["chat_mode"] == "plan"
    assert [turn["kind"] for turn in final_snapshot["turns"]] == ["mode_change", "message", "message"]
    assert final_snapshot["turns"][0]["content"] == "plan"
    assert final_snapshot["turns"][1]["content"] == "Plan the mode switch."
    assert final_snapshot["turns"][-1]["content"] == "Mode switch acknowledged."


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
    service = _project_chat_service()
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
    service = _project_chat_service()
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
