from __future__ import annotations

import asyncio
import json
import threading
import time
from pathlib import Path
from types import SimpleNamespace
from typing import Any, Callable, Optional

import pytest

import attractor.api.server as server
import spark.app as product_app
import spark.chat.service as project_chat
import spark.chat.session as project_chat_session
import spark.workspace.api as workspace_api
import spark.workspace.attractor_client as attractor_client
from spark_common.codex_app_client import (
    CodexAppServerThreadResumeFailure,
    CodexAppServerThreadResumeResult,
    CodexAppServerTurnResult,
)
import spark_common.codex_app_protocol as codex_app_protocol
import spark_common.process_line_reader as process_line_reader
from spark_common.turn_stream import TurnStreamEvent, TurnStreamSource
from spark.authoring_assets import (
    dot_authoring_guide_path,
    spark_operations_guide_path,
)
from tests.support.flow_fixtures import seed_flow_fixture
from spark.chat.prompt_templates import PROMPTS_FILE_NAME
from spark.workspace.conversations.models import (
    ConversationSegment,
    ConversationState,
    ConversationTurn,
    RequestUserInputOption,
    RequestUserInputQuestion,
    RequestUserInputRecord,
    ToolCallRecord,
)
from spark.workspace.storage import conversation_handles_path, ensure_project_paths, update_project_record


TEST_DISPATCH_FLOW = "test-dispatch.dot"
TEST_TIMESTAMP = "2026-04-30T12:00:00Z"


def _project_chat_service() -> project_chat.ProjectChatService:
    return product_app.get_project_chat()


def _seed_tool_output_conversation(
    *,
    conversation_id: str = "conversation-tool-output",
    project_path: str = "/tmp/project-tool-output",
    output: str,
) -> ConversationState:
    service = _project_chat_service()
    state = ConversationState(
        conversation_id=conversation_id,
        project_path=project_path,
        title="Tool output",
        created_at=TEST_TIMESTAMP,
        updated_at=TEST_TIMESTAMP,
        turns=[
            ConversationTurn(
                id="turn-user",
                role="user",
                content="Run it",
                timestamp=TEST_TIMESTAMP,
            ),
            ConversationTurn(
                id="turn-assistant",
                role="assistant",
                content="Done",
                timestamp=TEST_TIMESTAMP,
            ),
        ],
        segments=[
            ConversationSegment(
                id="segment-tool",
                turn_id="turn-assistant",
                order=1,
                kind="tool_call",
                role="system",
                status="complete",
                timestamp=TEST_TIMESTAMP,
                updated_at=TEST_TIMESTAMP,
                tool_call=ToolCallRecord(
                    id="tool-call",
                    kind="command_execution",
                    status="completed",
                    title="Large output",
                    command="printf output",
                    output=output,
                ),
            )
        ],
    )
    service._touch_conversation_state(state)
    service._write_state(state)
    return state


def _assert_ui_tool_output_preview(snapshot: dict[str, Any], full_output: str) -> None:
    tool_segment = next(segment for segment in snapshot["segments"] if segment["id"] == "segment-tool")
    tool_call = tool_segment["tool_call"]
    assert tool_call["output"] == full_output[:8192]
    assert tool_call["output_truncated"] is True
    assert tool_call["output_size"] == len(full_output.encode("utf-8"))


def _completed_turn_result(
    *,
    thread_id: str = "thread-123",
    turn_id: str = "turn-123",
    assistant_message: str = "Ack",
    plan_message: str = "",
    command_text: str = "",
    token_total: Optional[int] = None,
    token_usage_payload: Optional[dict[str, Any]] = None,
    error: Optional[str] = None,
) -> CodexAppServerTurnResult:
    state = codex_app_protocol.CodexAppServerTurnState()
    state.final_agent_message = assistant_message
    state.final_plan_message = plan_message or None
    state.turn_status = "completed"
    if command_text:
        state.command_chunks.append(command_text)
    if token_total is not None:
        state.last_token_total = token_total
    if token_usage_payload is not None:
        state.last_token_usage_payload = token_usage_payload
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
        self.resume_failure: CodexAppServerThreadResumeFailure | None = None
        self.resume_raw_lines: list[tuple[str, str]] = []
        self.start_result: str = "thread-123"
        self.default_model_result: Optional[str] = "gpt-test"
        self.run_turn_handler: Optional[Callable[..., CodexAppServerTurnResult]] = None
        self.raw_logger = None
        self.closed = False
        self.sent_responses: list[dict[str, Any]] = []

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
    ) -> CodexAppServerThreadResumeResult:
        self.resume_calls.append(
            {
                "thread_id": thread_id,
                "model": model,
                "cwd": cwd,
                "approval_policy": approval_policy,
            }
        )
        if self.raw_logger is not None:
            for direction, line in self.resume_raw_lines:
                self.raw_logger(direction, line)
        return CodexAppServerThreadResumeResult(
            thread_id=self.resume_result,
            failure=self.resume_failure,
        )

    def run_turn(self, **kwargs) -> CodexAppServerTurnResult:
        self.run_turn_calls.append(kwargs)
        if self.run_turn_handler is not None:
            return self.run_turn_handler(**kwargs)
        return _completed_turn_result(thread_id=kwargs["thread_id"])

    def send_response(
        self,
        request_id: Any,
        result: Optional[dict[str, Any]] = None,
        error: Optional[dict[str, Any]] = None,
    ) -> None:
        self.sent_responses.append(
            {
                "request_id": request_id,
                "result": result,
                "error": error,
            }
        )

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


def _request_user_input_record(request_id: str = "request-1") -> RequestUserInputRecord:
    return RequestUserInputRecord(
        request_id=request_id,
        status="pending",
        questions=[
            RequestUserInputQuestion(
                id="path_choice",
                header="Path",
                question="Which path should I take?",
                question_type="MULTIPLE_CHOICE",
                options=[
                    RequestUserInputOption(label="Inline card", description="Keep the request inline."),
                    RequestUserInputOption(label="Composer takeover", description="Move the request into the composer."),
                ],
                allow_other=True,
                is_secret=False,
            ),
            RequestUserInputQuestion(
                id="constraints",
                header="Constraints",
                question="What constraints matter?",
                question_type="FREEFORM",
                options=[],
                allow_other=False,
                is_secret=False,
            ),
        ],
    )


def test_extract_command_text_handles_list_and_string_payloads() -> None:
    assert codex_app_protocol.extract_command_text({"command": ["git", "status", "--short"]}) == "git status --short"
    assert codex_app_protocol.extract_command_text({"commandLine": "npm test"}) == "npm test"


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


def test_conversation_snapshot_truncates_tool_output_without_mutating_persisted_state() -> None:
    large_output = "x" * 9000
    state = _seed_tool_output_conversation(output=large_output)

    snapshot = _project_chat_service().get_snapshot(state.conversation_id, state.project_path)

    tool_call = snapshot["segments"][0]["tool_call"]
    assert tool_call["output"] == large_output[:8192]
    assert tool_call["output_truncated"] is True
    assert tool_call["output_size"] == len(large_output)

    persisted_state = _project_chat_service()._read_state(state.conversation_id, state.project_path)
    assert persisted_state is not None
    assert persisted_state.segments[0].tool_call is not None
    assert persisted_state.segments[0].tool_call.output == large_output


def test_conversation_snapshot_marks_small_tool_output_untruncated() -> None:
    output = "small output"
    state = _seed_tool_output_conversation(output=output)

    snapshot = _project_chat_service().get_snapshot(state.conversation_id, state.project_path)

    tool_call = snapshot["segments"][0]["tool_call"]
    assert tool_call["output"] == output
    assert tool_call["output_truncated"] is False
    assert tool_call["output_size"] == len(output)


def test_conversation_segment_tool_output_endpoint_returns_full_output(product_api_client) -> None:
    large_output = "full output\n" * 1000
    state = _seed_tool_output_conversation(output=large_output)

    response = product_api_client.get(
        f"/workspace/api/conversations/{state.conversation_id}/segments/segment-tool/tool-output",
        params={"project_path": state.project_path},
    )

    assert response.status_code == 200
    assert response.json() == {
        "output": large_output,
        "output_size": len(large_output.encode("utf-8")),
    }


def test_segment_upsert_payload_truncates_tool_output() -> None:
    state = _seed_tool_output_conversation(output="y" * 9000)
    segment = state.segments[0]

    payload = _project_chat_service()._build_segment_upsert_payload(state, segment)

    tool_call = payload["segment"]["tool_call"]
    assert len(tool_call["output"].encode("utf-8")) == 8192
    assert tool_call["output_truncated"] is True
    assert tool_call["output_size"] == 9000


def test_update_conversation_settings_return_truncates_historical_tool_output() -> None:
    large_output = "settings-output" * 700
    state = _seed_tool_output_conversation(output=large_output)

    snapshot = _project_chat_service().update_conversation_settings(
        state.conversation_id,
        state.project_path,
        chat_mode="plan",
    )

    _assert_ui_tool_output_preview(snapshot, large_output)
    persisted_state = _project_chat_service()._read_state(state.conversation_id, state.project_path)
    assert persisted_state is not None
    assert persisted_state.segments[0].tool_call is not None
    assert persisted_state.segments[0].tool_call.output == large_output


def test_start_turn_return_truncates_historical_tool_output(monkeypatch: pytest.MonkeyPatch) -> None:
    large_output = "start-output" * 800
    state = _seed_tool_output_conversation(output=large_output)
    entered_turn = threading.Event()
    finish_turn = threading.Event()

    class BlockingSession:
        def turn(self, *args, **kwargs) -> project_chat.ChatTurnResult:
            entered_turn.set()
            assert finish_turn.wait(timeout=2)
            on_event = kwargs.get("on_event")
            if on_event is not None:
                on_event(
                    project_chat.TurnStreamEvent(
                        kind="content_completed",
                        channel="assistant",
                        source=TurnStreamSource(app_turn_id="app-turn-1", item_id="msg-1"),
                        content_delta="Done.",
                        phase="final_answer",
                    )
                )
            return project_chat.ChatTurnResult(assistant_message="Done.")

    service = _project_chat_service()
    monkeypatch.setattr(service, "_build_session", lambda *args, **kwargs: BlockingSession())

    snapshot = service.start_turn(state.conversation_id, state.project_path, "Continue.", None)

    _assert_ui_tool_output_preview(snapshot, large_output)
    assert entered_turn.wait(timeout=2)
    finish_turn.set()


def test_send_turn_final_return_truncates_historical_tool_output(monkeypatch: pytest.MonkeyPatch) -> None:
    large_output = "final-output" * 800
    state = _seed_tool_output_conversation(output=large_output)

    class PlainTextSession:
        def turn(self, *args, **kwargs) -> project_chat.ChatTurnResult:
            on_event = kwargs.get("on_event")
            if on_event is not None:
                on_event(
                    project_chat.TurnStreamEvent(
                        kind="content_completed",
                        channel="assistant",
                        source=TurnStreamSource(app_turn_id="app-turn-1", item_id="msg-1"),
                        content_delta="Done.",
                        phase="final_answer",
                    )
                )
            return project_chat.ChatTurnResult(assistant_message="Done.")

    service = _project_chat_service()
    monkeypatch.setattr(service, "_build_session", lambda *args, **kwargs: PlainTextSession())

    snapshot = service.send_turn(state.conversation_id, state.project_path, "Continue.", None)

    _assert_ui_tool_output_preview(snapshot, large_output)


def test_request_user_input_answer_return_truncates_historical_tool_output() -> None:
    large_output = "request-output" * 800
    state = _seed_tool_output_conversation(output=large_output)
    request = _request_user_input_record()
    state.segments.append(
        project_chat.ConversationSegment(
            id="segment-request-user-input-app-turn-1-request-1",
            turn_id="turn-assistant",
            order=2,
            kind="request_user_input",
            role="system",
            status="pending",
            timestamp=TEST_TIMESTAMP,
            updated_at=TEST_TIMESTAMP,
            content="Which path should I take?",
            request_user_input=request,
        )
    )
    _project_chat_service()._write_state(state)

    snapshot = _project_chat_service().submit_request_user_input_answer(
        state.conversation_id,
        state.project_path,
        request.request_id,
        {
            "path_choice": "Inline card",
            "constraints": "Preserve the inline timeline.",
        },
    )

    _assert_ui_tool_output_preview(snapshot, large_output)


def test_review_flow_run_request_return_truncates_historical_tool_output() -> None:
    large_output = "flow-review-output" * 600
    state = _seed_tool_output_conversation(output=large_output)
    state.flow_run_requests.append(
        project_chat.FlowRunRequest(
            id="flow-request-1",
            created_at=TEST_TIMESTAMP,
            updated_at=TEST_TIMESTAMP,
            flow_name=TEST_DISPATCH_FLOW,
            summary="Run implementation.",
            project_path=state.project_path,
            conversation_id=state.conversation_id,
            source_turn_id="turn-assistant",
        )
    )
    _project_chat_service()._write_state(state)

    snapshot, flow_request = _project_chat_service().review_flow_run_request(
        state.conversation_id,
        state.project_path,
        "flow-request-1",
        "rejected",
        "Not now.",
        None,
        None,
    )

    assert flow_request.status == "rejected"
    _assert_ui_tool_output_preview(snapshot, large_output)


def test_review_proposed_plan_return_truncates_historical_tool_output(tmp_path: Path) -> None:
    large_output = "plan-review-output" * 600
    project_dir = tmp_path / "project"
    project_dir.mkdir(parents=True, exist_ok=True)
    state = _seed_tool_output_conversation(
        conversation_id="conversation-plan-tool-output",
        project_path=str(project_dir),
        output=large_output,
    )
    plan_segment = project_chat.ConversationSegment(
        id="segment-plan-inline",
        turn_id="turn-assistant",
        order=2,
        kind="plan",
        role="assistant",
        status="complete",
        timestamp=TEST_TIMESTAMP,
        updated_at=TEST_TIMESTAMP,
        completed_at=TEST_TIMESTAMP,
        content="# Proposed Plan\n\nDo the work.",
        artifact_id="proposed-plan-inline",
        source=project_chat.ConversationSegmentSource(),
    )
    state.segments.append(plan_segment)
    state.proposed_plans.append(
        project_chat.ProposedPlanArtifact(
            id="proposed-plan-inline",
            created_at=TEST_TIMESTAMP,
            updated_at=TEST_TIMESTAMP,
            title="Proposed Plan",
            content=plan_segment.content,
            project_path=str(project_dir),
            conversation_id=state.conversation_id,
            source_turn_id="turn-assistant",
            source_segment_id=plan_segment.id,
        )
    )
    _project_chat_service()._write_state(state)

    snapshot, proposed_plan, flow_launch = _project_chat_service().review_proposed_plan(
        state.conversation_id,
        state.project_path,
        "proposed-plan-inline",
        "rejected",
        "Needs work.",
    )

    assert proposed_plan.status == "rejected"
    assert flow_launch is None
    _assert_ui_tool_output_preview(snapshot, large_output)


def test_conversation_events_stream_does_not_emit_initial_snapshot(product_api_client, monkeypatch: pytest.MonkeyPatch) -> None:
    state = _seed_tool_output_conversation(output="small output")

    async def timeout_immediately(awaitable, timeout):
        awaitable.close()
        raise asyncio.TimeoutError

    monkeypatch.setattr(workspace_api.asyncio, "wait_for", timeout_immediately)

    class ConnectedRequest:
        async def is_disconnected(self) -> bool:
            return False

    async def read_first_stream_chunk() -> str:
        endpoint = None
        for route in product_app.app.routes:
            if getattr(route, "path", None) != "/workspace":
                continue
            for child_route in route.routes:
                if getattr(child_route, "path", None) == "/api/conversations/{conversation_id}/events":
                    endpoint = child_route.endpoint
                    break
        assert endpoint is not None
        response = await endpoint(state.conversation_id, ConnectedRequest(), state.project_path)
        iterator = response.body_iterator
        try:
            return await anext(iterator)
        finally:
            await iterator.aclose()

    assert asyncio.run(read_first_stream_chunk()) == ": keepalive\n\n"


def test_codex_app_server_chat_session_resumes_request_user_input_after_answer_submission() -> None:
    session = project_chat_session.CodexAppServerChatSession("/tmp/project-chat")
    stub_client = StubChatClient()
    session._client = stub_client
    events: list[project_chat.TurnStreamEvent] = []

    def run_turn_handler(**kwargs) -> CodexAppServerTurnResult:
        on_turn_started = kwargs.get("on_turn_started")
        if on_turn_started is not None:
            on_turn_started("app-turn-1")

        def answer_request() -> None:
            time.sleep(0.05)
            accepted = session.submit_request_user_input_answers(
                "path_choice",
                {"path_choice": "Inline card"},
            )
            assert accepted is True

        responder = threading.Thread(target=answer_request, daemon=True)
        responder.start()
        kwargs["server_request_handler"](
            {
                "jsonrpc": "2.0",
                "id": 7,
                "method": "item/tool/requestUserInput",
                "params": {
                    "threadId": "thread-123",
                    "turnId": "turn-123",
                    "itemId": "request-1",
                    "questions": [
                        {
                            "header": "Path",
                            "id": "path_choice",
                            "question": "Which path should I take?",
                            "isOther": True,
                            "isSecret": False,
                            "options": [
                                {"label": "Inline card", "description": "Keep the request inline."},
                                {"label": "Composer takeover", "description": "Move the request into the composer."},
                            ],
                        },
                    ],
                },
            }
        )
        responder.join(timeout=1)
        kwargs["on_event"](
            TurnStreamEvent(
                kind="content_completed",
                channel="assistant",
                content_delta="ANSWER=Inline card",
                source=TurnStreamSource(item_id="msg-1"),
                phase="final_answer",
            )
        )
        return _completed_turn_result(
            thread_id=kwargs["thread_id"],
            assistant_message="ANSWER=Inline card",
        )

    stub_client.run_turn_handler = run_turn_handler

    result = session.turn(
        "Use request_user_input.",
        "gpt-test",
        chat_mode="plan",
        on_event=events.append,
    )

    request_events = [event for event in events if event.kind == "request_user_input_requested"]

    assert result.assistant_message == "ANSWER=Inline card"
    assert len(request_events) == 1
    assert request_events[0].request_user_input is not None
    assert request_events[0].request_user_input.request_id == "request-1"
    assert [question.id for question in request_events[0].request_user_input.questions] == ["path_choice"]
    assert stub_client.sent_responses == [
        {
            "request_id": 7,
            "result": {
                "answers": {
                    "path_choice": {
                        "answers": ["Inline card"],
                    }
                }
            },
            "error": None,
        }
    ]


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

    with pytest.raises(
        RuntimeError,
        match=r"recent_conversation.*does not replay prior transcript text into prompts.*continuity resets",
    ):
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

    assert codex_app_protocol.extract_file_paths(payload) == [
        "frontend/src/features/projects/ProjectsPanel.tsx",
        "frontend/src/lib/apiClient.ts",
        "frontend/src/store.ts",
    ]


def test_append_tool_output_keeps_latest_tail() -> None:
    output = codex_app_protocol.append_tool_output("abc", "def", limit=4)

    assert output == "cdef"


def test_process_turn_message_normalizes_item_reasoning_delta_by_item_and_summary_index() -> None:
    state = codex_app_protocol.CodexAppServerTurnState()
    events = codex_app_protocol.process_turn_message(
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
    assert events[0].kind == "content_delta"
    assert events[0].channel == "reasoning"
    assert events[0].content_delta == "**Summ"
    assert events[0].source.item_id == "rs-1"
    assert events[0].source.summary_index == 0


def test_process_turn_message_normalizes_context_compaction_events() -> None:
    state = codex_app_protocol.CodexAppServerTurnState()

    started = codex_app_protocol.process_turn_message(
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
    completed = codex_app_protocol.process_turn_message(
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
    fallback = codex_app_protocol.process_turn_message(
        {
            "method": "thread/compacted",
            "params": {},
        },
        state,
    )

    assert [event.kind for event in started] == ["context_compaction_started"]
    assert started[0].source.item_id == "compact-1"
    assert [event.kind for event in completed] == ["context_compaction_completed"]
    assert completed[0].source.item_id == "compact-1"
    assert [event.kind for event in fallback] == ["context_compaction_completed"]
    assert fallback[0].source.item_id is None


def test_process_turn_message_retains_full_token_usage_payload() -> None:
    state = codex_app_protocol.CodexAppServerTurnState()
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

    events = codex_app_protocol.process_turn_message(
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


def test_process_turn_message_normalizes_command_output_as_tool_call_update() -> None:
    state = codex_app_protocol.CodexAppServerTurnState()

    events = codex_app_protocol.process_turn_message(
        {
            "method": "item/commandExecution/outputDelta",
            "params": {
                "itemId": "cmd-1",
                "delta": "output",
            },
        },
        state,
    )

    assert len(events) == 1
    assert events[0].kind == "tool_call_updated"
    assert events[0].source.raw_kind == "command_output_delta"
    assert events[0].source.item_id == "cmd-1"
    assert events[0].content_delta == "output"


def test_process_turn_message_uses_request_user_input_payload_field() -> None:
    state = codex_app_protocol.CodexAppServerTurnState()
    payload = {
        "itemId": "request-1",
        "questions": [
            {
                "id": "path_choice",
                "question": "Which path should I take?",
            },
        ],
    }

    events = codex_app_protocol.process_turn_message(
        {
            "method": "item/tool/requestUserInput",
            "params": payload,
        },
        state,
    )

    assert len(events) == 1
    assert events[0].kind == "request_user_input_requested"
    assert events[0].source.raw_kind == "request_user_input_requested"
    assert events[0].request_user_input == payload
    assert events[0].tool_call is None


def test_codex_app_server_chat_session_returns_token_usage_payload() -> None:
    session = project_chat_session.CodexAppServerChatSession("/tmp/project")
    stub_client = StubChatClient()
    session._client = stub_client
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
    events: list[project_chat.TurnStreamEvent] = []

    def run_turn_handler(**kwargs) -> CodexAppServerTurnResult:
        kwargs["on_event"](
            TurnStreamEvent(
                kind="token_usage_updated",
                token_usage=payload,
            )
        )
        kwargs["on_event"](
            TurnStreamEvent(
                kind="content_completed",
                channel="assistant",
                source=TurnStreamSource(item_id="msg-1"),
                content_delta="ACK",
                phase="final_answer",
            )
        )
        return _completed_turn_result(
            thread_id=kwargs["thread_id"],
            assistant_message="ACK",
            token_total=244,
            token_usage_payload=payload,
        )

    stub_client.run_turn_handler = run_turn_handler

    result = session.turn("hello", "gpt-test", on_event=events.append)

    assert result.assistant_message == "ACK"
    assert result.token_usage == payload
    assert [event.kind for event in events] == ["token_usage_updated", "content_completed"]
    assert events[0].token_usage == payload


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


def test_build_segment_upsert_payload_includes_matching_artifact_sidecar(tmp_path: Path) -> None:
    service = project_chat.ProjectChatService(tmp_path)
    state = project_chat.ConversationState(
        conversation_id="conversation-test",
        project_path="/tmp/project",
        flow_run_requests=[
            project_chat.FlowRunRequest(
                id="request-1",
                created_at="2026-03-06T23:00:02Z",
                updated_at="2026-03-06T23:00:03Z",
                flow_name="implementation.dot",
                summary="Run implementation.",
                project_path="/tmp/project",
                conversation_id="conversation-test",
                source_turn_id="turn-assistant-1",
                source_segment_id="segment-request-1",
            ),
        ],
        flow_launches=[
            project_chat.FlowLaunch(
                id="launch-1",
                created_at="2026-03-06T23:00:02Z",
                updated_at="2026-03-06T23:00:03Z",
                flow_name="implementation.dot",
                summary="Launch implementation.",
                project_path="/tmp/project",
                conversation_id="conversation-test",
                source_turn_id="turn-assistant-1",
                source_segment_id="segment-launch-1",
            ),
        ],
        proposed_plans=[
            project_chat.ProposedPlanArtifact(
                id="plan-1",
                created_at="2026-03-06T23:00:02Z",
                updated_at="2026-03-06T23:00:03Z",
                title="Implementation plan",
                content="Do the work.",
                project_path="/tmp/project",
                conversation_id="conversation-test",
                source_turn_id="turn-assistant-1",
                source_segment_id="segment-plan-1",
            ),
            project_chat.ProposedPlanArtifact(
                id="plan-other",
                created_at="2026-03-06T23:00:02Z",
                updated_at="2026-03-06T23:00:03Z",
                title="Other plan",
                content="Ignore this plan.",
                project_path="/tmp/project",
                conversation_id="conversation-test",
                source_turn_id="turn-assistant-1",
                source_segment_id="segment-plan-other",
            ),
        ],
    )
    segment = project_chat.ConversationSegment(
        id="segment-plan-1",
        turn_id="turn-assistant-1",
        order=1,
        kind="plan",
        role="assistant",
        status="complete",
        timestamp="2026-03-06T23:00:02Z",
        updated_at="2026-03-06T23:00:03Z",
        completed_at="2026-03-06T23:00:03Z",
        content="Do the work.",
        artifact_id="plan-1",
    )

    payload = service._build_segment_upsert_payload(state, segment)

    assert [entry["id"] for entry in payload["proposed_plans"]] == ["plan-1"]
    assert "flow_run_requests" not in payload
    assert "flow_launches" not in payload

    request_payload = service._build_segment_upsert_payload(
        state,
        project_chat.ConversationSegment(
            id="segment-request-1",
            turn_id="turn-assistant-1",
            order=2,
            kind="flow_run_request",
            role="system",
            status="complete",
            timestamp="2026-03-06T23:00:02Z",
            updated_at="2026-03-06T23:00:03Z",
            content="Run implementation.",
            artifact_id="request-1",
        ),
    )
    launch_payload = service._build_segment_upsert_payload(
        state,
        project_chat.ConversationSegment(
            id="segment-launch-1",
            turn_id="turn-assistant-1",
            order=3,
            kind="flow_launch",
            role="system",
            status="complete",
            timestamp="2026-03-06T23:00:02Z",
            updated_at="2026-03-06T23:00:03Z",
            content="Launch implementation.",
            artifact_id="launch-1",
        ),
    )
    assert [entry["id"] for entry in request_payload["flow_run_requests"]] == ["request-1"]
    assert [entry["id"] for entry in launch_payload["flow_launches"]] == ["launch-1"]


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


def test_conversation_state_rejects_schema_four_without_revision() -> None:
    with pytest.raises(ValueError, match="Unsupported conversation state schema"):
        project_chat.ConversationState.from_dict(
            {
                "schema_version": 4,
                "conversation_id": "conversation-test",
                "project_path": "/tmp/project",
                "title": "Legacy state",
                "turns": [],
                "segments": [],
            }
        )


def test_read_state_normalizes_request_user_input_without_persisting_revision(tmp_path: Path) -> None:
    service = project_chat.ProjectChatService(tmp_path)
    project_path = str((tmp_path / "project").resolve())
    state = project_chat.ConversationState(
        conversation_id="conversation-read-normalization",
        project_path=project_path,
        revision=7,
        title="Read normalization",
        created_at="2026-04-30T12:00:00Z",
        updated_at="2026-04-30T12:00:00Z",
        turns=[
            project_chat.ConversationTurn(
                id="turn-assistant-1",
                role="assistant",
                content="",
                timestamp="2026-04-30T12:00:00Z",
                status="streaming",
            ),
        ],
        segments=[
            project_chat.ConversationSegment(
                id="segment-request-user-input-1",
                turn_id="turn-assistant-1",
                order=1,
                kind="request_user_input",
                role="system",
                status="pending",
                timestamp="2026-04-30T12:00:00Z",
                updated_at="2026-04-30T12:00:00Z",
                content="Which path should I take?",
                request_user_input=project_chat.RequestUserInputRecord(
                    request_id="request-1",
                    status="expired",
                    questions=[],
                    answers={"path_choice": "Inline card"},
                    submitted_at="2026-04-30T12:00:01Z",
                ),
            ),
        ],
    )
    service._write_state(state)
    state_path = service._conversation_state_path(state.conversation_id, project_path)
    persisted_before = state_path.read_text(encoding="utf-8")

    loaded = service._read_state(state.conversation_id, project_path)

    assert loaded is not None
    assert loaded.revision == 7
    assert loaded.segments[0].status == "failed"
    assert loaded.turns[0].status == "failed"
    assert state_path.read_text(encoding="utf-8") == persisted_before


def test_get_snapshot_for_missing_conversation_returns_revision_zero_without_persisting(tmp_path: Path) -> None:
    service = project_chat.ProjectChatService(tmp_path)
    project_path = str((tmp_path / "project").resolve())
    project_paths = ensure_project_paths(tmp_path, project_path)
    state_path = project_paths.conversations_dir / "conversation-empty" / "state.json"

    snapshot = service.get_snapshot("conversation-empty", project_path)

    assert snapshot["conversation_id"] == "conversation-empty"
    assert snapshot["revision"] == 0
    assert snapshot["turns"] == []
    assert not state_path.exists()


def test_workflow_event_round_trips_structured_fields() -> None:
    event = project_chat.WorkflowEvent(
        message="Continuity reset: persisted thread could not be resumed.",
        timestamp="2026-04-20T21:30:00Z",
        kind="continuity_reset",
        error_code=project_chat_session.CONTINUITY_RESET_ERROR_CODE,
        details={
            "persisted_thread_id": "thread-stale",
            "replacement_thread_started": False,
            "resume_failure": {
                "kind": "resume_failed",
                "code": -32001,
                "message": "Persisted thread missing from runtime",
            },
        },
    )

    assert project_chat.WorkflowEvent.from_dict(event.to_dict()) == event


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

    commentary_delta = project_chat.TurnStreamEvent(
        kind="content_delta",
                channel="assistant",
        source=TurnStreamSource(app_turn_id="app-turn-1", item_id="item-msg-1"),
        content_delta="I’m checking the prompt template path.",
        phase="commentary",
    )
    final_delta = project_chat.TurnStreamEvent(
        kind="content_delta",
                channel="assistant",
        source=TurnStreamSource(app_turn_id="app-turn-1", item_id="item-msg-2"),
        content_delta="Here is the final grounded answer.",
        phase="final_answer",
    )

    service._materialize_segment_for_live_event(state, assistant_turn, commentary_delta)
    service._materialize_segment_for_live_event(state, assistant_turn, final_delta)

    commentary_complete = project_chat.TurnStreamEvent(
        kind="content_completed",
                channel="assistant",
        source=TurnStreamSource(app_turn_id="app-turn-1", item_id="item-msg-1"),
        content_delta="I’m checking the prompt template path.",
        phase="commentary",
    )
    final_complete = project_chat.TurnStreamEvent(
        kind="content_completed",
                channel="assistant",
        source=TurnStreamSource(app_turn_id="app-turn-1", item_id="item-msg-2"),
        content_delta="Here is the final grounded answer.",
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


def test_chat_session_raises_continuity_reset_when_resume_fails(monkeypatch) -> None:
    updated_thread_ids: list[str] = []
    session = project_chat.CodexAppServerChatSession(
        "/tmp/project",
        persisted_thread_id="thread-stale",
        on_thread_id_updated=updated_thread_ids.append,
    )
    client = StubChatClient()
    client.resume_failure = CodexAppServerThreadResumeFailure(
        kind="resume_failed",
        code=-32001,
        message="Persisted thread missing from runtime",
    )
    session._client = client

    with pytest.raises(project_chat_session.PersistedThreadContinuityResetError, match="Continuity reset"):
        session._ensure_thread("gpt-test")

    assert len(client.resume_calls) == 1
    assert client.start_calls == []
    assert session._thread_id == "thread-stale"
    assert updated_thread_ids == []


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

    result = session.turn("hello", None, chat_mode="plan", reasoning_effort="high")

    assert result.assistant_message == "Ack"
    assert client.run_turn_calls[0]["chat_mode"] == "plan"
    assert client.run_turn_calls[0]["reasoning_effort"] == "high"


def test_chat_session_uses_plan_text_when_turn_has_only_plan_item(monkeypatch) -> None:
    session = project_chat.CodexAppServerChatSession("/tmp/project")
    progress_updates: list[project_chat.TurnStreamEvent] = []
    client = StubChatClient()
    client.resume_result = "thread-existing"
    session._client = client
    session._thread_id = "thread-existing"

    def run_turn(**kwargs) -> CodexAppServerTurnResult:
        on_event = kwargs["on_event"]
        on_event(
            TurnStreamEvent(
                kind="content_delta",
                source=TurnStreamSource(item_id="plan-1"),
                channel="plan",
                content_delta="1. Capture the plan-only session path.\n",
            )
        )
        on_event(
            TurnStreamEvent(
                kind="content_completed",
                source=TurnStreamSource(item_id="plan-1"),
                channel="plan",
                content_delta="1. Capture the plan-only session path.\n2. Validate the fix.",
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
    assert [event.kind for event in progress_updates] == ["content_delta", "content_completed"]


def test_chat_session_surfaces_reasoning_summary_progress(monkeypatch) -> None:
    session = project_chat.CodexAppServerChatSession("/tmp/project")
    progress_updates: list[project_chat.TurnStreamEvent] = []
    client = StubChatClient()
    session._client = client

    def run_turn(**kwargs) -> CodexAppServerTurnResult:
        on_event = kwargs["on_event"]
        on_event(
            TurnStreamEvent(
                kind="content_delta",
                channel="reasoning",
                content_delta="Scanning the repository structure.",
            )
        )
        on_event(
            TurnStreamEvent(
                kind="content_delta",
                channel="assistant",
                content_delta="I found the main entry points.",
            )
        )
        on_event(
            TurnStreamEvent(
                kind="content_completed",
                channel="assistant",
                source=TurnStreamSource(item_id="msg-123"),
                content_delta="I found the main entry points.",
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
        update.kind == "content_delta" and update.content_delta == "Scanning the repository structure."
        for update in progress_updates
    )


def test_chat_session_surfaces_reasoning_summary_text_deltas(monkeypatch) -> None:
    session = project_chat.CodexAppServerChatSession("/tmp/project")
    progress_updates: list[project_chat.TurnStreamEvent] = []
    client = StubChatClient()
    session._client = client

    def run_turn(**kwargs) -> CodexAppServerTurnResult:
        on_event = kwargs["on_event"]
        on_event(
            TurnStreamEvent(
                kind="content_delta",
                channel="reasoning",
                content_delta="Draft draft that minimal proposal a best think",
            )
        )
        on_event(
            TurnStreamEvent(
                kind="content_delta",
                channel="assistant",
                content_delta="I’m checking the project structure first.",
            )
        )
        on_event(
            TurnStreamEvent(
                kind="content_completed",
                channel="assistant",
                source=TurnStreamSource(item_id="msg-123"),
                content_delta="I’m checking the project structure first.",
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
        update.kind == "content_delta"
        and update.content_delta == "Draft draft that minimal proposal a best think"
        for update in progress_updates
    )


def test_chat_session_surfaces_context_compaction_progress(monkeypatch) -> None:
    session = project_chat.CodexAppServerChatSession("/tmp/project")
    progress_updates: list[project_chat.TurnStreamEvent] = []
    client = StubChatClient()
    session._client = client

    def run_turn(**kwargs) -> CodexAppServerTurnResult:
        on_turn_started = kwargs["on_turn_started"]
        on_turn_started("turn-123")
        on_event = kwargs["on_event"]
        on_event(
            TurnStreamEvent(
                kind="context_compaction_started",
                source=TurnStreamSource(item_id="compact-1"),
            )
        )
        on_event(
            TurnStreamEvent(
                kind="context_compaction_completed",
                source=TurnStreamSource(item_id="compact-1"),
            )
        )
        on_event(
            TurnStreamEvent(
                kind="content_completed",
                channel="assistant",
                source=TurnStreamSource(item_id="msg-1"),
                content_delta="Ack",
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
        "content_completed",
    ]
    assert progress_updates[0].source.app_turn_id == "turn-123"
    assert progress_updates[0].source.item_id == "compact-1"
    assert progress_updates[1].source.app_turn_id == "turn-123"
    assert progress_updates[1].source.item_id == "compact-1"


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
            reasoning_effort: str | None = None,
            on_event=None,
            on_dynamic_tool_call=None,
        ) -> project_chat.ChatTurnResult:
            calls.append(prompt)
            if on_event is not None:
                on_event(
                    project_chat.TurnStreamEvent(
                        kind="content_completed",
                channel="assistant",
                        source=TurnStreamSource(app_turn_id="app-turn-1", item_id="msg-1"),
                        content_delta="hi",
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

def test_send_turn_marks_assistant_failed_after_non_runtime_exception(tmp_path: Path, monkeypatch) -> None:
    service = project_chat.ProjectChatService(tmp_path)

    class FailingSession:
        def turn(
            self,
            prompt: str,
            model: str | None,
            *,
            chat_mode: str = "chat",
            reasoning_effort: str | None = None,
            on_event=None,
            on_dynamic_tool_call=None,
        ) -> project_chat.ChatTurnResult:
            del prompt, model, chat_mode, reasoning_effort, on_event, on_dynamic_tool_call
            raise ValueError("provider exploded")

    monkeypatch.setattr(service, "_build_session", lambda conversation_id, project_path: FailingSession())

    with pytest.raises(ValueError, match="provider exploded"):
        service.send_turn("conversation-test", str(tmp_path), "hi", None)

    state = service._read_state("conversation-test", str(tmp_path))
    assert state is not None
    assert state.turns[-1].role == "assistant"
    assert state.turns[-1].status == "failed"
    assert state.turns[-1].error == "provider exploded"


def test_send_turn_fails_with_continuity_reset_when_persisted_resume_fails(tmp_path: Path) -> None:
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
    client.resume_failure = CodexAppServerThreadResumeFailure(
        kind="resume_failed",
        code=-32001,
        message="Persisted thread missing from runtime",
    )
    client.resume_raw_lines = [
        (
            "outgoing",
            '{"jsonrpc":"2.0","id":1,"method":"thread/resume","params":{"threadId":"thread-stale"}}',
        ),
        (
            "incoming",
            '{"jsonrpc":"2.0","id":1,"error":{"code":-32001,"message":"Persisted thread missing from runtime"}}',
        ),
    ]
    session._client = client

    with pytest.raises(project_chat_session.PersistedThreadContinuityResetError, match="Continuity reset"):
        service.send_turn(conversation_id, project_path, "Latest message", None)

    state = service._read_state(conversation_id, project_path)
    assert state is not None
    assert [turn.role for turn in state.turns] == ["user", "assistant", "user", "assistant"]
    assert state.turns[-2].content == "Latest message"
    assert state.turns[-1].role == "assistant"
    assert state.turns[-1].status == "failed"
    assert state.turns[-1].error_code == project_chat_session.CONTINUITY_RESET_ERROR_CODE
    assert "thread-stale" in (state.turns[-1].error or "")
    assert "Persisted thread missing from runtime" in (state.turns[-1].error or "")
    assert len(client.resume_calls) == 1
    assert client.resume_calls[0]["thread_id"] == "thread-stale"
    assert client.start_calls == []
    assert client.run_turn_calls == []
    assert session._thread_id is None

    session_payload = json.loads(
        (ensure_project_paths(tmp_path, project_path).conversations_dir / conversation_id / "session.json").read_text(
            encoding="utf-8"
        )
    )
    assert "thread_id" not in session_payload

    raw_log_path = ensure_project_paths(tmp_path, project_path).conversations_dir / conversation_id / "raw-log.jsonl"
    raw_entries = [json.loads(line) for line in raw_log_path.read_text(encoding="utf-8").splitlines()]
    assert [entry["direction"] for entry in raw_entries] == ["outgoing", "incoming"]
    assert "thread/resume" in raw_entries[0]["line"]
    assert "Persisted thread missing from runtime" in raw_entries[1]["line"]
    assert "thread/start" not in "\n".join(entry["line"] for entry in raw_entries)
    assert all(entry["direction"] in {"outgoing", "incoming"} for entry in raw_entries)

    snapshot = service.get_snapshot(conversation_id, project_path)
    continuity_events = [event for event in snapshot["event_log"] if event.get("kind") == "continuity_reset"]
    assert len(continuity_events) == 1
    continuity_event = continuity_events[0]
    assert continuity_event["error_code"] == project_chat_session.CONTINUITY_RESET_ERROR_CODE
    assert continuity_event["details"]["persisted_thread_id"] == "thread-stale"
    assert continuity_event["details"]["replacement_thread_started"] is False
    assert continuity_event["details"]["resume_failure"] == {
        "kind": "resume_failed",
        "code": -32001,
        "message": "Persisted thread missing from runtime",
    }
    assert "thread-stale" in continuity_event["message"]


def test_send_turn_starts_fresh_thread_on_next_explicit_message_after_continuity_reset(tmp_path: Path) -> None:
    service = project_chat.ProjectChatService(tmp_path)
    conversation_id = "conversation-test"
    project_path = str(tmp_path.resolve())
    service._write_state(
        project_chat.ConversationState(
            conversation_id=conversation_id,
            project_path=project_path,
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
    client.resume_failure = CodexAppServerThreadResumeFailure(
        kind="resume_failed",
        code=-32001,
        message="Persisted thread missing from runtime",
    )
    session._client = client

    with pytest.raises(project_chat_session.PersistedThreadContinuityResetError):
        service.send_turn(conversation_id, project_path, "First try", None)

    client.resume_failure = None
    client.start_result = "thread-fresh"

    def run_turn(**kwargs) -> CodexAppServerTurnResult:
        kwargs["on_event"](
            TurnStreamEvent(
                kind="content_completed",
                channel="assistant",
                source=TurnStreamSource(item_id="msg-1"),
                content_delta="Recovered.",
                phase="final_answer",
            )
        )
        return _completed_turn_result(
            thread_id=kwargs["thread_id"],
            assistant_message="Recovered.",
        )

    client.run_turn_handler = run_turn

    snapshot = service.send_turn(conversation_id, project_path, "Retry after reset", None)

    assert snapshot["turns"][-1]["status"] == "complete"
    assert snapshot["turns"][-1]["content"] == "Recovered."
    assert [call["thread_id"] for call in client.resume_calls] == ["thread-stale"]
    assert len(client.start_calls) == 1
    assert client.start_calls[0]["ephemeral"] is False
    session_payload = json.loads(
        (ensure_project_paths(tmp_path, project_path).conversations_dir / conversation_id / "session.json").read_text(
            encoding="utf-8"
        )
    )
    assert session_payload["thread_id"] == "thread-fresh"


def test_send_turn_accepts_plain_text_final_response(tmp_path: Path, monkeypatch) -> None:
    service = project_chat.ProjectChatService(tmp_path)
    progress_updates: list[dict[str, Any]] = []

    class PlainTextSession:
        def turn(
            self,
            prompt: str,
            model: str | None,
            *,
            chat_mode: str = "chat",
            reasoning_effort: str | None = None,
            on_event=None,
            on_dynamic_tool_call=None,
        ) -> project_chat.ChatTurnResult:
            if on_event is not None:
                on_event(
                    project_chat.TurnStreamEvent(
                        kind="content_completed",
                channel="assistant",
                        source=TurnStreamSource(app_turn_id="app-turn-1", item_id="msg-1"),
                        content_delta="This looks like a Collatz implementation project.",
                        phase="final_answer",
                    )
                )
            return project_chat.ChatTurnResult(
                assistant_message="This looks like a Collatz implementation project.",
            )

        def _close(self) -> None:
            return None

    monkeypatch.setattr(service, "_build_session", lambda conversation_id, project_path: PlainTextSession())

    snapshot = service.send_turn(
        "conversation-test",
        str(tmp_path),
        "What's this project about?",
        None,
        progress_callback=progress_updates.append,
    )

    assert snapshot["schema_version"] == 5
    assert snapshot["revision"] == 3
    assert {payload["revision"] for payload in progress_updates} == {1, 2, 3}
    assert snapshot["turns"][-1]["role"] == "assistant"
    assert snapshot["turns"][-1]["status"] == "complete"
    assert snapshot["turns"][-1]["content"] == "This looks like a Collatz implementation project."


def test_update_conversation_settings_upserts_shell_and_persists_chat_mode(tmp_path: Path) -> None:
    service = project_chat.ProjectChatService(tmp_path)

    snapshot = service.update_conversation_settings("conversation-settings", str(tmp_path), "plan")

    assert snapshot["conversation_id"] == "conversation-settings"
    assert snapshot["revision"] == 1
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
    assert reloaded["revision"] == 1
    assert reloaded["chat_mode"] == "plan"
    assert [turn["kind"] for turn in reloaded["turns"]] == ["mode_change"]


def test_update_conversation_settings_does_not_duplicate_mode_change_when_mode_matches(tmp_path: Path) -> None:
    service = project_chat.ProjectChatService(tmp_path)

    first = service.update_conversation_settings("conversation-settings", str(tmp_path), "plan")
    second = service.update_conversation_settings("conversation-settings", str(tmp_path), "plan")

    assert first["chat_mode"] == "plan"
    assert first["revision"] == 1
    assert second["chat_mode"] == "plan"
    assert second["revision"] == 2
    assert [turn["kind"] for turn in second["turns"]] == ["mode_change"]
    assert second["turns"][0]["content"] == "plan"


def test_update_conversation_settings_persists_model_and_reasoning_effort(tmp_path: Path) -> None:
    service = project_chat.ProjectChatService(tmp_path)

    snapshot = service.update_conversation_settings(
        "conversation-settings",
        str(tmp_path),
        model="gpt-5.4",
        reasoning_effort="HIGH",
        provider="OpenAI",
    )

    assert snapshot["chat_mode"] == "chat"
    assert snapshot["revision"] == 1
    assert snapshot["provider"] == "openai"
    assert snapshot["model"] == "gpt-5.4"
    assert snapshot["reasoning_effort"] == "high"
    assert snapshot["turns"] == []
    reloaded = service.get_snapshot("conversation-settings", str(tmp_path))
    assert reloaded["revision"] == 1
    assert reloaded["provider"] == "openai"
    assert reloaded["model"] == "gpt-5.4"
    assert reloaded["reasoning_effort"] == "high"


def test_update_conversation_settings_rejects_invalid_reasoning_effort(tmp_path: Path) -> None:
    service = project_chat.ProjectChatService(tmp_path)

    with pytest.raises(ValueError, match="Reasoning effort"):
        service.update_conversation_settings(
            "conversation-settings",
            str(tmp_path),
            reasoning_effort="extreme",
        )


def test_send_turn_persists_and_forwards_model_and_reasoning_effort(tmp_path: Path, monkeypatch) -> None:
    service = project_chat.ProjectChatService(tmp_path)
    captured_calls: list[dict[str, str | None]] = []

    class PlainTextSession:
        def turn(
            self,
            prompt: str,
            model: str | None,
            *,
            chat_mode: str = "chat",
            reasoning_effort: str | None = None,
            on_event=None,
            on_dynamic_tool_call=None,
        ) -> project_chat.ChatTurnResult:
            captured_calls.append(
                {
                    "model": model,
                    "reasoning_effort": reasoning_effort,
                    "chat_mode": chat_mode,
                }
            )
            if on_event is not None:
                on_event(
                    project_chat.TurnStreamEvent(
                        kind="content_completed",
                channel="assistant",
                        source=TurnStreamSource(app_turn_id="app-turn-1", item_id="msg-1"),
                        content_delta="Settings acknowledged.",
                        phase="final_answer",
                    )
                )
            return project_chat.ChatTurnResult(assistant_message="Settings acknowledged.")

    monkeypatch.setattr(service, "_build_session", lambda conversation_id, project_path: PlainTextSession())

    snapshot = service.send_turn(
        "conversation-model-settings",
        str(tmp_path),
        "Use these settings.",
        "gpt-5.4",
        reasoning_effort="xhigh",
    )

    assert snapshot["model"] == "gpt-5.4"
    assert snapshot["reasoning_effort"] == "xhigh"
    assert captured_calls == [
        {
            "model": "gpt-5.4",
            "reasoning_effort": "xhigh",
            "chat_mode": "chat",
        }
    ]
    reloaded = service.get_snapshot("conversation-model-settings", str(tmp_path))
    assert reloaded["model"] == "gpt-5.4"
    assert reloaded["reasoning_effort"] == "xhigh"


def test_send_turn_reuses_persisted_model_and_reasoning_effort_when_omitted(tmp_path: Path, monkeypatch) -> None:
    service = project_chat.ProjectChatService(tmp_path)
    captured_calls: list[dict[str, str | None]] = []
    service.update_conversation_settings(
        "conversation-model-settings",
        str(tmp_path),
        model="gpt-5.4-mini",
        reasoning_effort="medium",
    )

    class PlainTextSession:
        def turn(
            self,
            prompt: str,
            model: str | None,
            *,
            chat_mode: str = "chat",
            reasoning_effort: str | None = None,
            on_event=None,
            on_dynamic_tool_call=None,
        ) -> project_chat.ChatTurnResult:
            captured_calls.append(
                {
                    "model": model,
                    "reasoning_effort": reasoning_effort,
                }
            )
            if on_event is not None:
                on_event(
                    project_chat.TurnStreamEvent(
                        kind="content_completed",
                channel="assistant",
                        source=TurnStreamSource(app_turn_id="app-turn-1", item_id="msg-1"),
                        content_delta="Persisted settings used.",
                        phase="final_answer",
                    )
                )
            return project_chat.ChatTurnResult(assistant_message="Persisted settings used.")

    monkeypatch.setattr(service, "_build_session", lambda conversation_id, project_path: PlainTextSession())

    service.send_turn("conversation-model-settings", str(tmp_path), "Continue.", None)

    assert captured_calls == [
        {
            "model": "gpt-5.4-mini",
            "reasoning_effort": "medium",
        }
    ]


def test_send_turn_persists_and_forwards_provider(tmp_path: Path, monkeypatch) -> None:
    service = project_chat.ProjectChatService(tmp_path)
    captured_calls: list[dict[str, str | None]] = []

    class PlainTextSession:
        def turn(
            self,
            prompt: str,
            model: str | None,
            *,
            chat_mode: str = "chat",
            reasoning_effort: str | None = None,
            on_event=None,
            on_dynamic_tool_call=None,
        ) -> project_chat.ChatTurnResult:
            del prompt, chat_mode, reasoning_effort, on_dynamic_tool_call
            captured_calls[-1]["turn_model"] = model
            turn_index = len(captured_calls)
            if on_event is not None:
                on_event(
                    project_chat.TurnStreamEvent(
                        kind="content_completed",
                channel="assistant",
                        source=TurnStreamSource(app_turn_id=f"app-turn-{turn_index}", item_id=f"msg-{turn_index}"),
                        content_delta="Provider acknowledged.",
                        phase="final_answer",
                    )
                )
            return project_chat.ChatTurnResult(assistant_message="Provider acknowledged.")

    def fake_build_session(
        conversation_id: str,
        project_path: str,
        provider: str = "codex",
        model: str | None = None,
    ) -> PlainTextSession:
        captured_calls.append(
            {
                "conversation_id": conversation_id,
                "provider": provider,
                "model": model,
            }
        )
        return PlainTextSession()

    monkeypatch.setattr(service, "_build_session", fake_build_session)

    first = service.send_turn(
        "conversation-provider-settings",
        str(tmp_path),
        "Use OpenAI.",
        "gpt-5.4",
        provider="openai",
    )
    second = service.send_turn(
        "conversation-provider-settings",
        str(tmp_path),
        "Continue.",
        None,
    )

    assert first["provider"] == "openai"
    assert second["provider"] == "openai"
    assert captured_calls == [
        {
            "conversation_id": "conversation-provider-settings",
            "provider": "openai",
            "model": "gpt-5.4",
            "turn_model": "gpt-5.4",
        },
        {
            "conversation_id": "conversation-provider-settings",
            "provider": "openai",
            "model": "gpt-5.4",
            "turn_model": "gpt-5.4",
        },
    ]


def test_build_session_keys_chat_sessions_by_provider_and_model(tmp_path: Path, monkeypatch) -> None:
    service = project_chat.ProjectChatService(tmp_path)
    created: list[dict[str, str | None]] = []

    class FakeCodexSession:
        def __init__(self, working_dir: str, **kwargs) -> None:
            del kwargs
            self.marker = f"codex:{working_dir}"
            created.append({"provider": "codex", "model": None})

    class FakeUnifiedSession:
        def __init__(
            self,
            working_dir: str,
            *,
            provider: str,
            model: str | None = None,
            persisted_history: list[project_chat.ConversationTurn] | None = None,
        ) -> None:
            del persisted_history
            self.marker = f"{provider}:{model}:{working_dir}"
            created.append({"provider": provider, "model": model})

    monkeypatch.setattr(project_chat, "CodexAppServerChatSession", FakeCodexSession)
    monkeypatch.setattr(project_chat, "UnifiedAgentChatSession", FakeUnifiedSession)

    codex_default = service._build_session("conversation-provider-switch", str(tmp_path))
    openai = service._build_session("conversation-provider-switch", str(tmp_path), "openai", "gpt-5.4")
    anthropic = service._build_session("conversation-provider-switch", str(tmp_path), "anthropic", "claude-test")
    openrouter = service._build_session("conversation-provider-switch", str(tmp_path), "openrouter", "openai/gpt-test")
    litellm = service._build_session("conversation-provider-switch", str(tmp_path), "litellm", "team-model")
    codex_again = service._build_session("conversation-provider-switch", str(tmp_path), "codex", None)

    assert codex_default is codex_again
    assert openai is not codex_default
    assert anthropic is not openai
    assert openrouter is not openai
    assert litellm is not openrouter
    assert created == [
        {"provider": "codex", "model": None},
        {"provider": "openai", "model": "gpt-5.4"},
        {"provider": "anthropic", "model": "claude-test"},
        {"provider": "openrouter", "model": "openai/gpt-test"},
        {"provider": "litellm", "model": "team-model"},
    ]
    assert sorted(service._sessions) == [
        "conversation-provider-switch::anthropic::claude-test",
        "conversation-provider-switch::codex::",
        "conversation-provider-switch::litellm::team-model",
        "conversation-provider-switch::openai::gpt-5.4",
        "conversation-provider-switch::openrouter::openai/gpt-test",
    ]


def test_project_chat_starts_profile_session_with_explicit_model(tmp_path: Path, monkeypatch) -> None:
    service = project_chat.ProjectChatService(tmp_path)
    (tmp_path / "config").mkdir(exist_ok=True)
    (tmp_path / "config" / "llm-profiles.toml").write_text(
        """
        [profiles.local]
        provider = "openai_compatible"
        base_url = "http://127.0.0.1:1234/v1"
        models = ["local-model"]
        """,
        encoding="utf-8",
    )
    created: list[dict[str, object | None]] = []

    class FakeUnifiedSession:
        def __init__(
            self,
            working_dir: str,
            *,
            provider: str,
            model: str | None = None,
            llm_profile: str | None = None,
            config_dir: Path | None = None,
            persisted_history: list[project_chat.ConversationTurn] | None = None,
        ) -> None:
            del persisted_history
            created.append(
                {
                    "working_dir": working_dir,
                    "provider": provider,
                    "model": model,
                    "llm_profile": llm_profile,
                    "config_dir": config_dir,
                }
            )

    monkeypatch.setattr(project_chat, "UnifiedAgentChatSession", FakeUnifiedSession)

    session = service._build_session(
        "conversation-profile",
        str(tmp_path),
        provider="codex",
        model="local-model",
        llm_profile="local",
    )
    same_session = service._build_session(
        "conversation-profile",
        str(tmp_path),
        provider="codex",
        model="local-model",
        llm_profile="local",
    )

    assert session is same_session
    assert created == [
        {
            "working_dir": str(tmp_path),
            "provider": "codex",
            "model": "local-model",
            "llm_profile": "local",
            "config_dir": tmp_path / "config",
        }
    ]
    assert sorted(service._sessions) == ["conversation-profile::codex::local::local-model"]


def test_delete_conversation_closes_all_provider_model_keyed_sessions(tmp_path: Path) -> None:
    service = project_chat.ProjectChatService(tmp_path)
    project_path = str(tmp_path)
    conversation_id = "conversation-delete-provider-keyed"
    other_conversation_id = "conversation-delete-provider-keyed-other"
    closed: list[str] = []

    class FakeSession:
        def __init__(self, label: str) -> None:
            self.label = label

        def close(self) -> None:
            closed.append(self.label)

    service._write_state(
        project_chat.ConversationState(
            conversation_id=conversation_id,
            project_path=project_path,
        )
    )

    with service._sessions_lock:
        service._sessions[conversation_id] = FakeSession("legacy")
        service._sessions[service._session_key(conversation_id, "codex", None)] = FakeSession("codex-default")
        service._sessions[service._session_key(conversation_id, "openai", "gpt-5.4")] = FakeSession("openai")
        service._sessions[service._session_key(conversation_id, "anthropic", "claude-test")] = FakeSession(
            "anthropic"
        )
        service._sessions[service._session_key(other_conversation_id, "openai", "gpt-5.4")] = FakeSession("other")

    snapshot = service.delete_conversation(conversation_id, project_path)

    assert snapshot["status"] == "deleted"
    assert sorted(closed) == ["anthropic", "codex-default", "legacy", "openai"]
    with service._sessions_lock:
        assert sorted(service._sessions) == [
            "conversation-delete-provider-keyed-other::openai::gpt-5.4",
        ]


def test_build_session_does_not_restore_codex_thread_from_different_model(tmp_path: Path, monkeypatch) -> None:
    service = project_chat.ProjectChatService(tmp_path)
    service._write_session_state(
        project_chat.ConversationSessionState(
            conversation_id="conversation-model-isolated",
            updated_at="2026-03-06T23:59:00Z",
            project_path=str(tmp_path),
            runtime_project_path=str(tmp_path),
            provider="codex",
            thread_id="thread-gpt-5-4",
            model="gpt-5.4",
        )
    )
    captured: dict[str, object] = {}

    class FakeCodexSession:
        def __init__(
            self,
            working_dir: str,
            *,
            persisted_thread_id=None,
            persisted_model=None,
            on_thread_id_updated=None,
            on_model_updated=None,
        ) -> None:
            del working_dir, on_thread_id_updated, on_model_updated
            captured["persisted_thread_id"] = persisted_thread_id
            captured["persisted_model"] = persisted_model

    monkeypatch.setattr(project_chat, "CodexAppServerChatSession", FakeCodexSession)

    service._build_session("conversation-model-isolated", str(tmp_path), "codex", "gpt-5.5")

    assert captured["persisted_thread_id"] is None
    assert captured["persisted_model"] is None


def test_unified_chat_session_applies_reasoning_effort_changes_between_turns(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    observed_reasoning_efforts: list[str | None] = []

    class FakeSession:
        def __init__(self, *, provider_profile, execution_environment, client, config) -> None:
            del execution_environment, client
            self.provider_profile = provider_profile
            self.config = config
            self.state = project_chat_session.SessionState.IDLE
            self.history: list[project_chat_session.AssistantTurn] = []
            self.event_queue = asyncio.Queue()

        async def process_input(self, prompt: str) -> None:
            observed_reasoning_efforts.append(self.config.reasoning_effort)
            self.history.append(project_chat_session.AssistantTurn(f"ack {prompt}"))

        async def close(self) -> None:
            return None

    monkeypatch.setattr(
        project_chat_session,
        "_profile_for_provider",
        lambda provider, model: SimpleNamespace(model=str(model or f"{provider}-default")),
    )
    monkeypatch.setattr(project_chat_session, "Session", FakeSession)

    session = project_chat_session.UnifiedAgentChatSession(
        str(tmp_path),
        provider="openai",
        model="gpt-test",
        client_factory=lambda provider: SimpleNamespace(provider=provider),
    )

    first = session.turn("first", None, reasoning_effort="high")
    second = session.turn("second", None, reasoning_effort="low")

    assert first.assistant_message == "ack first"
    assert second.assistant_message == "ack second"
    assert observed_reasoning_efforts == ["high", "low"]


def test_project_chat_unified_session_hydrates_persisted_history_without_current_turn(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    captured_histories: list[list[project_chat.ConversationTurn]] = []

    class FakeUnifiedSession:
        def __init__(
            self,
            working_dir: str,
            *,
            provider: str,
            model: str | None = None,
            persisted_history: list[project_chat.ConversationTurn] | None = None,
        ) -> None:
            del working_dir, provider, model
            captured_histories.append(list(persisted_history or []))

        def turn(
            self,
            prompt: str,
            model: str | None,
            *,
            chat_mode: str = "chat",
            reasoning_effort: str | None = None,
            on_event=None,
        ) -> project_chat.ChatTurnResult:
            del prompt, model, chat_mode, reasoning_effort
            if on_event is not None:
                on_event(
                    project_chat.TurnStreamEvent(
                        kind="content_completed",
                        channel="assistant",
                        content_delta="assistant reply",
                    )
                )
            return project_chat.ChatTurnResult(assistant_message="assistant reply")

    monkeypatch.setattr(project_chat, "UnifiedAgentChatSession", FakeUnifiedSession)

    first_service = project_chat.ProjectChatService(tmp_path)
    first_service.send_turn("conversation-history", str(tmp_path), "first question", provider="openai")

    second_service = project_chat.ProjectChatService(tmp_path)
    second_service.send_turn("conversation-history", str(tmp_path), "second question", provider="openai")

    assert [[turn.role, turn.content] for turn in captured_histories[0]] == []
    assert [[turn.role, turn.content] for turn in captured_histories[1]] == [
        ["user", "first question"],
        ["assistant", "assistant reply"],
    ]
    assert "second question" not in [turn.content for turn in captured_histories[1]]


def test_unified_chat_session_maps_session_events_to_turn_stream_source(tmp_path: Path) -> None:
    session = project_chat_session.UnifiedAgentChatSession(
        str(tmp_path),
        provider="openai",
        model="gpt-test",
        client_factory=lambda provider: SimpleNamespace(provider=provider),
    )
    events: list[TurnStreamEvent] = []

    session._forward_session_event(
        project_chat_session.SessionEvent(
            project_chat_session.EventKind.ASSISTANT_REASONING_DELTA,
            data={"delta": "Thinking", "response_id": "resp-1"},
        ),
        on_event=events.append,
    )
    session._forward_session_event(
        project_chat_session.SessionEvent(
            project_chat_session.EventKind.MODEL_USAGE_UPDATE,
            data={"usage": {"input_tokens": 2, "output_tokens": 3, "total_tokens": 5}},
        ),
        on_event=events.append,
    )
    session._forward_session_event(
        project_chat_session.SessionEvent(
            project_chat_session.EventKind.MODEL_TOOL_CALL_START,
            data={"tool_call": {"id": "call-model", "name": "shell"}},
        ),
        on_event=events.append,
    )

    assert [event.kind for event in events] == ["content_delta", "token_usage_updated"]
    assert events[0].channel == "reasoning"
    assert events[0].content_delta == "Thinking"
    assert events[0].source.backend == "agent_session"
    assert events[0].source.response_id == "resp-1"
    assert events[1].token_usage == {
        "total": {
            "inputTokens": 2,
            "cachedInputTokens": 0,
            "outputTokens": 3,
            "totalTokens": 5,
        }
    }

def test_unified_chat_session_close_closes_agent_session_and_client(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    closed_sessions: list[str] = []
    closed_clients: list[str] = []

    class FakeUnifiedClient:
        def __init__(self, provider: str) -> None:
            self.provider = provider

        async def close(self) -> None:
            closed_clients.append(self.provider)

    class FakeSession:
        def __init__(self, *, provider_profile, execution_environment, client, config) -> None:
            del execution_environment, config
            self.provider_profile = provider_profile
            self.client = client
            self.state = project_chat_session.SessionState.IDLE
            self.history: list[project_chat_session.AssistantTurn] = []
            self.event_queue = asyncio.Queue()

        async def process_input(self, prompt: str) -> None:
            self.history.append(project_chat_session.AssistantTurn(f"ack {prompt}"))

        async def close(self) -> None:
            closed_sessions.append(self.provider_profile.model)

    monkeypatch.setattr(
        project_chat_session,
        "_profile_for_provider",
        lambda provider, model: SimpleNamespace(model=str(model or f"{provider}-default")),
    )
    monkeypatch.setattr(project_chat_session, "Session", FakeSession)

    session = project_chat_session.UnifiedAgentChatSession(
        str(tmp_path),
        provider="openai",
        model="gpt-test",
        client_factory=FakeUnifiedClient,
    )

    assert session.turn("hello", None).assistant_message == "ack hello"
    session.close()

    assert closed_sessions == ["gpt-test"]
    assert closed_clients == ["openai"]


def test_unified_chat_session_model_switch_closes_replaced_session_and_client(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    closed_sessions: list[str] = []
    closed_clients: list[str] = []

    class FakeUnifiedClient:
        def __init__(self, provider: str) -> None:
            self.provider = provider

        async def close(self) -> None:
            closed_clients.append(self.provider)

    class FakeSession:
        def __init__(self, *, provider_profile, execution_environment, client, config) -> None:
            del execution_environment, client, config
            self.provider_profile = provider_profile
            self.state = project_chat_session.SessionState.IDLE
            self.history: list[project_chat_session.AssistantTurn] = []
            self.event_queue = asyncio.Queue()

        async def process_input(self, prompt: str) -> None:
            self.history.append(project_chat_session.AssistantTurn(f"{self.provider_profile.model}:{prompt}"))

        async def close(self) -> None:
            closed_sessions.append(self.provider_profile.model)

    monkeypatch.setattr(
        project_chat_session,
        "_profile_for_provider",
        lambda provider, model: SimpleNamespace(model=str(model or f"{provider}-default")),
    )
    monkeypatch.setattr(project_chat_session, "Session", FakeSession)

    session = project_chat_session.UnifiedAgentChatSession(
        str(tmp_path),
        provider="openai",
        model="gpt-old",
        client_factory=FakeUnifiedClient,
    )

    assert session.turn("first", None).assistant_message == "gpt-old:first"
    assert session.turn("second", "gpt-new").assistant_message == "gpt-new:second"

    assert closed_sessions == ["gpt-old"]
    assert closed_clients == ["openai"]


def test_list_chat_models_combines_codex_and_unified_provider_metadata(
    tmp_path: Path,
    monkeypatch,
) -> None:
    service = project_chat.ProjectChatService(tmp_path)

    class FakeCodexModel:
        def to_dict(self) -> dict[str, Any]:
            return {
                "id": "gpt-codex",
                "display": "GPT Codex",
                "supported_reasoning_efforts": ["low", "high"],
                "default_reasoning_effort": "high",
            }

    class FakeClient:
        def __init__(self, *args, **kwargs) -> None:
            return None

        def ensure_process(self, *, popen_factory) -> None:
            return None

        def list_models(self) -> list[FakeCodexModel]:
            return [FakeCodexModel()]

        def close(self) -> None:
            return None

    monkeypatch.setattr(project_chat, "CodexAppServerClient", FakeClient)
    monkeypatch.setattr(
        project_chat,
        "list_unified_models",
        lambda: [
            SimpleNamespace(
                provider="openai",
                id="gpt-5.4",
                display_name="GPT 5.4",
                supports_reasoning=True,
            ),
            SimpleNamespace(
                provider="local",
                id="ignored-local",
                display_name="Ignored",
                supports_reasoning=False,
            ),
        ],
    )

    payload = service.list_chat_models(str(tmp_path))

    assert payload["models"] == [
        {
            "provider": "codex",
            "id": "gpt-codex",
            "display": "GPT Codex",
            "supported_reasoning_efforts": ["low", "high"],
            "default_reasoning_effort": "high",
        },
        {
            "provider": "openai",
            "id": "gpt-5.4",
            "display": "GPT 5.4",
            "supported_reasoning_efforts": ["low", "medium", "high", "xhigh"],
            "default_reasoning_effort": "medium",
        },
    ]


def test_list_chat_models_returns_unified_models_when_codex_discovery_fails(
    tmp_path: Path,
    monkeypatch,
) -> None:
    service = project_chat.ProjectChatService(tmp_path)

    class FakeClient:
        def __init__(self, *args, **kwargs) -> None:
            return None

        def ensure_process(self, *, popen_factory) -> None:
            raise RuntimeError("codex unavailable")

        def close(self) -> None:
            return None

    monkeypatch.setattr(project_chat, "CodexAppServerClient", FakeClient)
    monkeypatch.setattr(
        project_chat,
        "list_unified_models",
        lambda: [
            SimpleNamespace(
                provider="openai",
                id="gpt-5.4",
                display_name="GPT 5.4",
                supports_reasoning=True,
            ),
        ],
    )

    payload = service.list_chat_models(str(tmp_path))

    assert payload["models"] == [
        {
            "provider": "openai",
            "id": "gpt-5.4",
            "display": "GPT 5.4",
            "supported_reasoning_efforts": ["low", "medium", "high", "xhigh"],
            "default_reasoning_effort": "medium",
        }
    ]


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
            reasoning_effort: str | None = None,
            on_event=None,
            on_dynamic_tool_call=None,
        ) -> project_chat.ChatTurnResult:
            captured_chat_modes.append(chat_mode)
            if on_event is not None:
                on_event(
                    project_chat.TurnStreamEvent(
                        kind="content_completed",
                channel="assistant",
                        source=TurnStreamSource(app_turn_id="app-turn-1", item_id="msg-1"),
                        content_delta="Plan mode acknowledged.",
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
            reasoning_effort: str | None = None,
            on_event=None,
            on_dynamic_tool_call=None,
        ) -> project_chat.ChatTurnResult:
            captured_chat_modes.append(chat_mode)
            if on_event is not None:
                on_event(
                    project_chat.TurnStreamEvent(
                        kind="content_completed",
                channel="assistant",
                        source=TurnStreamSource(app_turn_id="app-turn-1", item_id="msg-1"),
                        content_delta="Still in plan mode.",
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
            TurnStreamEvent(
                kind="content_delta",
                source=TurnStreamSource(item_id="plan-1"),
                channel="plan",
                content_delta="1. Patch the real session path.\n",
            )
        )
        on_event(
            TurnStreamEvent(
                kind="content_completed",
                source=TurnStreamSource(item_id="plan-1"),
                channel="plan",
                content_delta="1. Patch the real session path.\n2. Add the regression coverage.",
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
    assert snapshot["segments"][0]["artifact_id"].startswith("proposed-plan-")
    proposed_plan = snapshot["proposed_plans"][0]
    assert proposed_plan["conversation_id"] == conversation_id
    assert proposed_plan["source_turn_id"] == assistant_turn["id"]
    assert proposed_plan["source_segment_id"] == snapshot["segments"][0]["id"]
    assert proposed_plan["status"] == "pending_review"
    assert proposed_plan["content"] == "1. Patch the real session path.\n2. Add the regression coverage."
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
            reasoning_effort: str | None = None,
            on_event=None,
            on_dynamic_tool_call=None,
        ) -> project_chat.ChatTurnResult:
            assert chat_mode == "plan"
            if on_event is not None:
                on_event(
                    project_chat.TurnStreamEvent(
                        kind="content_delta",
                        channel="reasoning",
                        content_delta="Checking the active session path.",
                    )
                )
                on_event(
                    project_chat.TurnStreamEvent(
                        kind="content_completed",
                channel="assistant",
                        source=TurnStreamSource(app_turn_id="app-turn-1", item_id="msg-1"),
                        content_delta=raw_plan_markup,
                        phase="final_answer",
                    )
                )
                on_event(
                    project_chat.TurnStreamEvent(
                        kind="content_delta",
                        source=TurnStreamSource(app_turn_id="app-turn-1", item_id="plan-1"),
                        channel="plan",
                        content_delta="1. Patch the real session path.\n",
                    )
                )
                on_event(
                    project_chat.TurnStreamEvent(
                        kind="content_completed",
                        source=TurnStreamSource(app_turn_id="app-turn-1", item_id="plan-1"),
                        channel="plan",
                        content_delta="1. Patch the real session path.\n2. Add the regression coverage.",
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
            reasoning_effort: str | None = None,
            on_event=None,
            on_dynamic_tool_call=None,
        ) -> project_chat.ChatTurnResult:
            if on_event is not None:
                on_event(
                    project_chat.TurnStreamEvent(
                        kind="context_compaction_started",
                        source=TurnStreamSource(app_turn_id="app-turn-1", item_id="compact-1"),
                    )
                )
                on_event(
                    project_chat.TurnStreamEvent(
                        kind="context_compaction_completed",
                        source=TurnStreamSource(app_turn_id="app-turn-1", item_id="compact-1"),
                    )
                )
                on_event(
                    project_chat.TurnStreamEvent(
                        kind="content_completed",
                channel="assistant",
                        source=TurnStreamSource(app_turn_id="app-turn-1", item_id="msg-1"),
                        content_delta="Ack",
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
            reasoning_effort: str | None = None,
            on_event=None,
            on_dynamic_tool_call=None,
        ) -> project_chat.ChatTurnResult:
            if on_event is not None:
                on_event(
                    project_chat.TurnStreamEvent(
                        kind="context_compaction_completed",
                        source=TurnStreamSource(app_turn_id="app-turn-1"),
                    )
                )
                on_event(
                    project_chat.TurnStreamEvent(
                        kind="content_completed",
                channel="assistant",
                        source=TurnStreamSource(app_turn_id="app-turn-1", item_id="msg-1"),
                        content_delta="Ack",
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
            reasoning_effort: str | None = None,
            on_event=None,
            on_dynamic_tool_call=None,
        ) -> project_chat.ChatTurnResult:
            if on_event is not None:
                on_event(
                    project_chat.TurnStreamEvent(
                        kind="context_compaction_started",
                        source=TurnStreamSource(app_turn_id="app-turn-1", item_id="compact-1"),
                    )
                )
                on_event(
                    project_chat.TurnStreamEvent(
                        kind="context_compaction_completed",
                        source=TurnStreamSource(app_turn_id="app-turn-1", item_id="compact-1"),
                    )
                )
                on_event(
                    project_chat.TurnStreamEvent(
                        kind="context_compaction_completed",
                        source=TurnStreamSource(app_turn_id="app-turn-1"),
                    )
                )
                on_event(
                    project_chat.TurnStreamEvent(
                        kind="content_completed",
                channel="assistant",
                        source=TurnStreamSource(app_turn_id="app-turn-1", item_id="msg-1"),
                        content_delta="Ack",
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


def test_request_user_input_segments_persist_and_answer_in_place(
    tmp_path: Path,
    monkeypatch,
) -> None:
    service = project_chat.ProjectChatService(tmp_path)
    progress_updates: list[dict[str, Any]] = []

    class WaitingRequestSession:
        def __init__(self) -> None:
            self._answer_event = threading.Event()
            self._answers: dict[str, str] = {}

        def has_pending_request_user_input(self, request_id: str) -> bool:
            return request_id == "request-1" and not self._answer_event.is_set()

        def submit_request_user_input_answers(self, request_id: str, answers: dict[str, str]) -> bool:
            if request_id != "request-1":
                return False
            self._answers = dict(answers)
            self._answer_event.set()
            return True

        def turn(
            self,
            prompt: str,
            model: str | None,
            *,
            chat_mode: str = "chat",
            reasoning_effort: str | None = None,
            on_event=None,
            on_dynamic_tool_call=None,
        ) -> project_chat.ChatTurnResult:
            if on_event is not None:
                on_event(
                    project_chat.TurnStreamEvent(
                        kind="request_user_input_requested",
                        source=TurnStreamSource(app_turn_id="app-turn-1", item_id="request-1"),
                        request_user_input=_request_user_input_record(),
                    )
                )
            assert self._answer_event.wait(timeout=2)
            assistant_message = (
                f"ANSWER={self._answers['path_choice']} / {self._answers['constraints']}"
            )
            if on_event is not None:
                on_event(
                    project_chat.TurnStreamEvent(
                        kind="content_completed",
                channel="assistant",
                        source=TurnStreamSource(app_turn_id="app-turn-1", item_id="msg-1"),
                        content_delta=assistant_message,
                        phase="final_answer",
                    )
                )
            return project_chat.ChatTurnResult(assistant_message=assistant_message)

    waiting_session = WaitingRequestSession()
    monkeypatch.setattr(service, "_build_session", lambda conversation_id, project_path: waiting_session)
    with service._sessions_lock:
        service._sessions["conversation-request-user-input"] = waiting_session

    service.start_turn(
        "conversation-request-user-input",
        str(tmp_path),
        "Ask for the missing decision.",
        None,
        "plan",
        progress_callback=progress_updates.append,
    )

    deadline = time.time() + 2.0
    pending_snapshot: dict[str, Any] | None = None
    while time.time() < deadline:
        candidate = service.get_snapshot("conversation-request-user-input", str(tmp_path))
        request_segments = [segment for segment in candidate["segments"] if segment["kind"] == "request_user_input"]
        if request_segments:
            pending_snapshot = candidate
            break
        time.sleep(0.02)

    assert pending_snapshot is not None
    pending_revision = pending_snapshot["revision"]
    request_segments = [segment for segment in pending_snapshot["segments"] if segment["kind"] == "request_user_input"]
    assert len(request_segments) == 1
    assert request_segments[0]["status"] == "pending"
    assert request_segments[0]["request_user_input"]["status"] == "pending"

    answered_snapshot = service.submit_request_user_input_answer(
        "conversation-request-user-input",
        str(tmp_path),
        "path_choice",
        {
            "path_choice": "Inline card",
            "constraints": "Preserve the inline timeline.",
        },
        progress_callback=progress_updates.append,
    )

    answered_request_segments = [
        segment for segment in answered_snapshot["segments"] if segment["kind"] == "request_user_input"
    ]
    assert answered_snapshot["revision"] == pending_revision + 1
    assert len(answered_request_segments) == 1
    assert answered_request_segments[0]["status"] == "complete"
    assert answered_request_segments[0]["request_user_input"]["status"] == "answered"
    assert answered_request_segments[0]["request_user_input"]["answers"] == {
        "path_choice": "Inline card",
        "constraints": "Preserve the inline timeline.",
    }
    assert "delivery_status" not in answered_request_segments[0]["request_user_input"]
    assert "delivered_at" not in answered_request_segments[0]["request_user_input"]

    deadline = time.time() + 2.0
    final_snapshot: dict[str, Any] | None = None
    while time.time() < deadline:
        candidate = service.get_snapshot("conversation-request-user-input", str(tmp_path))
        if candidate["turns"][-1]["status"] == "complete":
            final_snapshot = candidate
            break
        time.sleep(0.02)

    assert final_snapshot is not None
    assert final_snapshot["revision"] > answered_snapshot["revision"]
    assert final_snapshot["turns"][-1]["content"] == "ANSWER=Inline card / Preserve the inline timeline."
    assert [segment["kind"] for segment in final_snapshot["segments"]] == [
        "request_user_input",
        "assistant_message",
    ]
    request_payloads = [
        payload["segment"]
        for payload in progress_updates
        if payload.get("type") == "segment_upsert" and payload["segment"]["kind"] == "request_user_input"
    ]
    assert [payload["status"] for payload in request_payloads] == ["pending", "complete"]


def test_request_user_input_answers_find_provider_keyed_codex_session(tmp_path: Path) -> None:
    service = project_chat.ProjectChatService(tmp_path)
    project_path = str(tmp_path)
    conversation_id = "conversation-request-provider-keyed"

    class WaitingRequestSession:
        def __init__(self) -> None:
            self.answers: dict[str, str] = {}

        def has_pending_request_user_input(self, request_id: str) -> bool:
            return request_id in {"request-1", "path_choice"}

        def submit_request_user_input_answers(self, request_id: str, answers: dict[str, str]) -> bool:
            if request_id != "request-1":
                return False
            self.answers = dict(answers)
            return True

    waiting_session = WaitingRequestSession()
    with service._sessions_lock:
        service._sessions[service._session_key(conversation_id, "codex", None)] = waiting_session
    service._write_state(
        project_chat.ConversationState(
            conversation_id=conversation_id,
            project_path=project_path,
            turns=[
                project_chat.ConversationTurn(
                    id="turn-user-1",
                    role="user",
                    content="Ask me the missing question.",
                    timestamp="2026-04-17T12:00:00Z",
                ),
                project_chat.ConversationTurn(
                    id="turn-assistant-1",
                    role="assistant",
                    content="",
                    timestamp="2026-04-17T12:00:01Z",
                    status="streaming",
                    parent_turn_id="turn-user-1",
                ),
            ],
            segments=[
                project_chat.ConversationSegment(
                    id="segment-request-user-input-app-turn-1-request-1",
                    turn_id="turn-assistant-1",
                    order=1,
                    kind="request_user_input",
                    role="system",
                    status="pending",
                    timestamp="2026-04-17T12:00:02Z",
                    updated_at="2026-04-17T12:00:02Z",
                    content="Which path should I take?",
                    request_user_input=_request_user_input_record(),
                ),
            ],
        )
    )

    snapshot = service.submit_request_user_input_answer(
        conversation_id,
        project_path,
        "path_choice",
        {
            "path_choice": "Inline card",
            "constraints": "Preserve the inline timeline.",
        },
    )

    request_segment = next(segment for segment in snapshot["segments"] if segment["kind"] == "request_user_input")
    assert request_segment["status"] == "complete"
    assert request_segment["request_user_input"]["status"] == "answered"
    assert waiting_session.answers == {
        "path_choice": "Inline card",
        "constraints": "Preserve the inline timeline.",
    }


def test_request_user_input_answers_expire_without_live_session(tmp_path: Path) -> None:
    service = project_chat.ProjectChatService(tmp_path)
    project_path = str(tmp_path)
    conversation_id = "conversation-request-queued"
    service._write_state(
        project_chat.ConversationState(
            conversation_id=conversation_id,
            project_path=project_path,
            turns=[
                project_chat.ConversationTurn(
                    id="turn-user-1",
                    role="user",
                    content="Ask me the missing question.",
                    timestamp="2026-04-17T12:00:00Z",
                ),
                project_chat.ConversationTurn(
                    id="turn-assistant-1",
                    role="assistant",
                    content="",
                    timestamp="2026-04-17T12:00:01Z",
                    status="streaming",
                    parent_turn_id="turn-user-1",
                ),
            ],
            segments=[
                project_chat.ConversationSegment(
                    id="segment-request-user-input-app-turn-1-request-1",
                    turn_id="turn-assistant-1",
                    order=1,
                    kind="request_user_input",
                    role="system",
                    status="pending",
                    timestamp="2026-04-17T12:00:02Z",
                    updated_at="2026-04-17T12:00:02Z",
                    content="Which path should I take?",
                    request_user_input=_request_user_input_record(),
                ),
            ],
        )
    )

    snapshot = service.submit_request_user_input_answer(
        conversation_id,
        project_path,
        "request-1",
        {
            "path_choice": "Inline card",
            "constraints": "Preserve the inline timeline.",
        },
    )

    request_segment = next(segment for segment in snapshot["segments"] if segment["kind"] == "request_user_input")
    request_payload = request_segment["request_user_input"]

    assert request_segment["status"] == "failed"
    assert request_segment["content"] == (
        "Which path should I take?\n"
        "Answer: Inline card\n\n"
        "What constraints matter?\n"
        "Answer: Preserve the inline timeline."
    )
    assert request_segment["error"] == "The requested input expired before the answer could be used."
    assert request_payload["status"] == "expired"
    assert request_payload["submitted_at"] is not None
    assert request_payload["answers"] == {
        "path_choice": "Inline card",
        "constraints": "Preserve the inline timeline.",
    }
    assert snapshot["turns"][-1]["status"] == "failed"
    assert snapshot["turns"][-1]["error"] == "The requested input expired before the answer could be used."

def test_get_snapshot_does_not_build_or_resume_request_user_input_delivery(tmp_path: Path, monkeypatch) -> None:
    service = project_chat.ProjectChatService(tmp_path)
    project_path = str(tmp_path)
    conversation_id = "conversation-request-pure-read"
    service._write_state(
        project_chat.ConversationState(
            conversation_id=conversation_id,
            project_path=project_path,
            turns=[
                project_chat.ConversationTurn(
                    id="turn-user-1",
                    role="user",
                    content="Ask me the missing question.",
                    timestamp="2026-04-17T12:00:00Z",
                ),
                project_chat.ConversationTurn(
                    id="turn-assistant-1",
                    role="assistant",
                    content="",
                    timestamp="2026-04-17T12:00:01Z",
                    status="streaming",
                    parent_turn_id="turn-user-1",
                ),
            ],
            segments=[
                project_chat.ConversationSegment(
                    id="segment-request-user-input-app-turn-1-request-1",
                    turn_id="turn-assistant-1",
                    order=1,
                    kind="request_user_input",
                    role="system",
                    status="pending",
                    timestamp="2026-04-17T12:00:02Z",
                    updated_at="2026-04-17T12:00:02Z",
                    content="Which path should I take?",
                    request_user_input=_request_user_input_record(),
                ),
            ],
        )
    )

    monkeypatch.setattr(
        service,
        "_build_session",
        lambda conversation_id, project_path: pytest.fail("get_snapshot should not build chat sessions"),
    )

    snapshot = service.get_snapshot(conversation_id, project_path)

    assert snapshot["turns"][-1]["status"] == "streaming"
    request_segment = next(segment for segment in snapshot["segments"] if segment["kind"] == "request_user_input")
    assert request_segment["request_user_input"]["status"] == "pending"


def test_request_user_input_answer_submissions_are_idempotent_for_matching_duplicates(tmp_path: Path) -> None:
    service = project_chat.ProjectChatService(tmp_path)
    project_path = str(tmp_path)
    conversation_id = "conversation-request-idempotent"
    service._write_state(
        project_chat.ConversationState(
            conversation_id=conversation_id,
            project_path=project_path,
            turns=[
                project_chat.ConversationTurn(
                    id="turn-user-1",
                    role="user",
                    content="Ask me the missing question.",
                    timestamp="2026-04-17T12:00:00Z",
                ),
                project_chat.ConversationTurn(
                    id="turn-assistant-1",
                    role="assistant",
                    content="",
                    timestamp="2026-04-17T12:00:01Z",
                    status="streaming",
                    parent_turn_id="turn-user-1",
                ),
            ],
            segments=[
                project_chat.ConversationSegment(
                    id="segment-request-user-input-app-turn-1-request-1",
                    turn_id="turn-assistant-1",
                    order=1,
                    kind="request_user_input",
                    role="system",
                    status="pending",
                    timestamp="2026-04-17T12:00:02Z",
                    updated_at="2026-04-17T12:00:02Z",
                    content="Which path should I take?",
                    request_user_input=_request_user_input_record(),
                ),
            ],
        )
    )

    first_snapshot = service.submit_request_user_input_answer(
        conversation_id,
        project_path,
        "request-1",
        {
            "path_choice": "Inline card",
            "constraints": "Preserve the inline timeline.",
        },
    )
    second_snapshot = service.submit_request_user_input_answer(
        conversation_id,
        project_path,
        "path_choice",
        {
            "path_choice": "Inline card",
            "constraints": "Preserve the inline timeline.",
        },
    )

    first_request = next(
        segment["request_user_input"]
        for segment in first_snapshot["segments"]
        if segment["kind"] == "request_user_input"
    )
    second_request = next(
        segment["request_user_input"]
        for segment in second_snapshot["segments"]
        if segment["kind"] == "request_user_input"
    )

    assert second_request["answers"] == first_request["answers"]
    assert second_request["submitted_at"] == first_request["submitted_at"]
    assert "delivery_status" not in second_request

    with pytest.raises(ValueError, match="expired before the answer could be used"):
        service.submit_request_user_input_answer(
            conversation_id,
            project_path,
            "request-1",
            {
                "path_choice": "Composer takeover",
                "constraints": "Move it into the composer.",
            },
        )


def test_conversation_request_user_input_segments_remain_scoped_to_their_conversation(tmp_path: Path) -> None:
    service = project_chat.ProjectChatService(tmp_path)
    project_path = str(tmp_path)
    service._write_state(
        project_chat.ConversationState(
            conversation_id="conversation-with-request",
            project_path=project_path,
            turns=[
                project_chat.ConversationTurn(
                    id="turn-user-1",
                    role="user",
                    content="Ask me the missing question.",
                    timestamp="2026-04-17T12:00:00Z",
                ),
                project_chat.ConversationTurn(
                    id="turn-assistant-1",
                    role="assistant",
                    content="",
                    timestamp="2026-04-17T12:00:01Z",
                    status="streaming",
                    parent_turn_id="turn-user-1",
                ),
            ],
            segments=[
                project_chat.ConversationSegment(
                    id="segment-request-user-input-app-turn-1-request-1",
                    turn_id="turn-assistant-1",
                    order=1,
                    kind="request_user_input",
                    role="system",
                    status="pending",
                    timestamp="2026-04-17T12:00:02Z",
                    updated_at="2026-04-17T12:00:02Z",
                    content="Which path should I take?",
                    request_user_input=_request_user_input_record(),
                ),
            ],
        )
    )
    service._write_state(
        project_chat.ConversationState(
            conversation_id="conversation-clean",
            project_path=project_path,
            chat_mode="plan",
            turns=[
                project_chat.ConversationTurn(
                    id="turn-mode-1",
                    role="system",
                    content="plan",
                    timestamp="2026-04-17T12:05:00Z",
                    kind="mode_change",
                ),
            ],
        )
    )

    clean_snapshot = service.get_snapshot("conversation-clean", project_path)

    assert clean_snapshot["conversation_id"] == "conversation-clean"
    assert clean_snapshot["segments"] == []


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
            reasoning_effort: str | None = None,
            on_event=None,
            on_dynamic_tool_call=None,
        ) -> project_chat.ChatTurnResult:
            assert chat_mode == "plan"
            if on_event is not None:
                on_event(
                    project_chat.TurnStreamEvent(
                        kind="content_completed",
                channel="assistant",
                        source=TurnStreamSource(app_turn_id="app-turn-1", item_id="msg-1"),
                        content_delta=raw_response,
                        phase="final_answer",
                    )
                )
                on_event(
                    project_chat.TurnStreamEvent(
                        kind="content_completed",
                        source=TurnStreamSource(app_turn_id="app-turn-1", item_id="plan-1"),
                        channel="plan",
                        content_delta="1. Patch the real session path.\n2. Add the regression coverage.",
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
            reasoning_effort: str | None = None,
            on_event=None,
            on_dynamic_tool_call=None,
        ) -> project_chat.ChatTurnResult:
            captured_chat_modes.append(chat_mode)
            if on_event is not None:
                on_event(
                    project_chat.TurnStreamEvent(
                        kind="content_completed",
                channel="assistant",
                        source=TurnStreamSource(app_turn_id="app-turn-1", item_id="msg-1"),
                        content_delta="Default chat mode.",
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


def test_create_flow_run_request_drops_direct_execution_container_image(tmp_path: Path) -> None:
    project_dir = tmp_path.resolve()
    conversation_id = "conversation-flow-run-image-selection"
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

    result = service.create_flow_run_request(
        conversation_id,
        str(project_dir),
        {
            "flow_name": TEST_DISPATCH_FLOW,
            "summary": "Run implementation for the approved scope.",
            "execution_container_image": "spark-exec:legacy",
            "execution_profile_id": "local-dev",
        },
    )

    snapshot = service.get_snapshot(conversation_id, str(project_dir))
    request_payload = next(
        entry for entry in snapshot["flow_run_requests"] if entry["id"] == result["flow_run_request_id"]
    )
    assert request_payload["execution_profile_id"] == "local-dev"
    assert "execution_container_image" not in request_payload


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
        llm_provider: str | None = None,
        llm_profile: str | None = None,
        reasoning_effort: str | None = None,
        execution_profile_id: str | None = None,
        project_default_execution_profile_id: str | None = None,
        goal: str | None = None,
        launch_context: dict[str, object] | None = None,
        spec_id: str | None = None,
        plan_id: str | None = None,
        **unexpected: object,
    ) -> dict[str, object]:
        assert "backend" not in unexpected
        assert not unexpected
        start_calls.append(
            {
                "run_id": run_id,
                "flow_name": flow_name,
                "working_directory": working_directory,
                "model": model,
                "llm_provider": llm_provider,
                "llm_profile": llm_profile,
                "reasoning_effort": reasoning_effort,
                "execution_profile_id": execution_profile_id,
                "project_default_execution_profile_id": project_default_execution_profile_id,
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
            "llm_provider": "openai",
            "llm_profile": "implementation",
            "reasoning_effort": "high",
            "execution_profile_id": "local-dev",
        },
    )

    assert create_response.status_code == 200
    create_payload = create_response.json()
    assert create_payload["ok"] is True
    assert create_payload["conversation_id"] == conversation_id
    request_id = create_payload["flow_run_request_id"]
    created_snapshot = _project_chat_service().get_snapshot(conversation_id, str(project_dir))
    assert created_snapshot["revision"] == snapshot["revision"] + 1

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
    assert approved_snapshot["revision"] == created_snapshot["revision"] + 2
    request_payload = next(
        entry for entry in approved_snapshot["flow_run_requests"] if entry["id"] == request_id
    )
    assert request_payload["status"] == "launched"
    assert request_payload["run_id"] == "run-flow-123"
    assert request_payload["review_message"] == "Approved for launch."
    assert request_payload["llm_provider"] == "openai"
    assert request_payload["llm_profile"] == "implementation"
    assert request_payload["reasoning_effort"] == "high"
    assert request_payload["execution_profile_id"] == "local-dev"
    assert start_calls == [
        {
            "run_id": None,
            "flow_name": TEST_DISPATCH_FLOW,
            "working_directory": str(project_dir),
            "model": "gpt-5.4",
            "llm_provider": "openai",
            "llm_profile": "implementation",
            "reasoning_effort": "high",
            "execution_profile_id": "local-dev",
            "project_default_execution_profile_id": None,
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


def test_flow_run_request_approval_launches_with_project_default_execution_profile(
    product_api_client,
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    project_dir = tmp_path.resolve()
    update_project_record(
        product_app.get_settings().data_dir,
        str(project_dir),
        execution_profile_id="remote-build",
    )
    conversation_id = "conversation-flow-run-project-default"
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
        execution_profile_id: str | None = None,
        project_default_execution_profile_id: str | None = None,
        **kwargs: object,
    ) -> dict[str, object]:
        assert kwargs["goal"] == "Implement the approved scope."
        start_calls.append(
            {
                "run_id": run_id,
                "flow_name": flow_name,
                "working_directory": working_directory,
                "model": model,
                "execution_profile_id": execution_profile_id,
                "project_default_execution_profile_id": project_default_execution_profile_id,
            }
        )
        return {"status": "started", "run_id": "run-flow-default-profile"}

    monkeypatch.setattr(attractor_client.AttractorApiClient, "start_pipeline", fake_start_pipeline)

    create_response = product_api_client.post(
        f"/workspace/api/conversations/by-handle/{snapshot['conversation_handle']}/flow-run-requests",
        json={
            "flow_name": TEST_DISPATCH_FLOW,
            "summary": "Run implementation for the approved scope.",
            "goal": "Implement the approved scope.",
        },
    )

    assert create_response.status_code == 200
    request_id = create_response.json()["flow_run_request_id"]
    created_snapshot = _project_chat_service().get_snapshot(conversation_id, str(project_dir))
    persisted_request = next(
        entry for entry in created_snapshot["flow_run_requests"] if entry["id"] == request_id
    )
    assert "execution_profile_id" not in persisted_request

    review_response = product_api_client.post(
        f"/workspace/api/conversations/{conversation_id}/flow-run-requests/{request_id}/review",
        json={
            "project_path": str(project_dir),
            "disposition": "approved",
            "message": "Approved for launch.",
        },
    )

    assert review_response.status_code == 200
    request_payload = next(
        entry for entry in review_response.json()["flow_run_requests"] if entry["id"] == request_id
    )
    assert request_payload["status"] == "launched"
    assert request_payload["run_id"] == "run-flow-default-profile"
    assert "execution_profile_id" not in request_payload
    assert start_calls == [
        {
            "run_id": None,
            "flow_name": TEST_DISPATCH_FLOW,
            "working_directory": str(project_dir),
            "model": None,
            "execution_profile_id": None,
            "project_default_execution_profile_id": "remote-build",
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
        llm_provider: str | None = None,
        llm_profile: str | None = None,
        reasoning_effort: str | None = None,
        execution_profile_id: str | None = None,
        project_default_execution_profile_id: str | None = None,
        goal: str | None = None,
        launch_context: dict[str, object] | None = None,
        spec_id: str | None = None,
        plan_id: str | None = None,
        **unexpected: object,
    ) -> dict[str, object]:
        assert "backend" not in unexpected
        assert not unexpected
        start_calls.append(
            {
                "run_id": run_id,
                "flow_name": flow_name,
                "working_directory": working_directory,
                "model": model,
                "llm_provider": llm_provider,
                "llm_profile": llm_profile,
                "reasoning_effort": reasoning_effort,
                "execution_profile_id": execution_profile_id,
                "project_default_execution_profile_id": project_default_execution_profile_id,
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
            "llm_provider": "anthropic",
            "reasoning_effort": "low",
            "execution_container_image": "spark-exec:legacy",
            "execution_profile_id": "remote-review",
            "backend": "codex-app-server",
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
    assert updated_snapshot["revision"] == snapshot["revision"] + 2
    flow_launch = next(
        entry for entry in updated_snapshot["flow_launches"] if entry["id"] == launch_payload["flow_launch_id"]
    )
    assert flow_launch["status"] == "launched"
    assert flow_launch["run_id"] == "run-launch-123"
    assert flow_launch["goal"] == "Implement the approved scope."
    assert flow_launch["llm_provider"] == "anthropic"
    assert flow_launch["reasoning_effort"] == "low"
    assert flow_launch["execution_profile_id"] == "remote-review"
    assert "execution_container_image" not in flow_launch
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
            "llm_provider": "anthropic",
            "llm_profile": None,
            "reasoning_effort": "low",
            "execution_profile_id": "remote-review",
            "project_default_execution_profile_id": None,
            "goal": "Implement the approved scope.",
            "launch_context": {
                "context.request.summary": "Implement the approved scope.",
            },
            "spec_id": None,
            "plan_id": None,
        }
    ]


def test_review_proposed_plan_writes_change_request_and_creates_launch_artifact(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    service = project_chat.ProjectChatService(tmp_path)
    project_dir = tmp_path / "project"
    project_dir.mkdir(parents=True, exist_ok=True)
    existing_change_dir = project_dir / "changes" / "CR-2026-0001-reviewable-proposed-plan-artifacts-in-project-chat"
    existing_change_dir.mkdir(parents=True, exist_ok=True)
    (existing_change_dir / "request.md").write_text(
        "# Existing plan\n",
        encoding="utf-8",
    )
    monkeypatch.setattr("spark.workspace.conversations.artifacts.iso_now", lambda: "2026-03-13T10:02:00Z")

    assistant_turn = project_chat.ConversationTurn(
        id="turn-assistant-plan",
        role="assistant",
        content="Here is the proposed plan.",
        timestamp="2026-03-13T10:01:00Z",
        status="complete",
    )
    plan_segment = project_chat.ConversationSegment(
        id="segment-plan-inline",
        turn_id=assistant_turn.id,
        order=1,
        kind="plan",
        role="assistant",
        status="complete",
        timestamp="2026-03-13T10:01:05Z",
        updated_at="2026-03-13T10:01:05Z",
        completed_at="2026-03-13T10:01:05Z",
        content="# Reviewable Proposed Plan Artifacts in Project Chat\n\n1. Add the backend artifact.\n2. Wire the review UI.",
        artifact_id="proposed-plan-inline",
        source=project_chat.ConversationSegmentSource(),
    )
    state = project_chat.ConversationState(
        conversation_id="conversation-proposed-plan",
        project_path=str(project_dir),
        chat_mode="plan",
        title="Plan review",
        created_at="2026-03-13T10:00:00Z",
        updated_at="2026-03-13T10:01:05Z",
        turns=[
            project_chat.ConversationTurn(
                id="turn-user-plan",
                role="user",
                content="Draft the implementation plan.",
                timestamp="2026-03-13T10:00:00Z",
            ),
            assistant_turn,
        ],
        segments=[plan_segment],
        proposed_plans=[
            project_chat.ProposedPlanArtifact(
                id="proposed-plan-inline",
                created_at="2026-03-13T10:01:05Z",
                updated_at="2026-03-13T10:01:05Z",
                title="Reviewable Proposed Plan Artifacts in Project Chat",
                content=plan_segment.content,
                project_path=str(project_dir),
                conversation_id="conversation-proposed-plan",
                source_turn_id=assistant_turn.id,
                source_segment_id=plan_segment.id,
            ),
        ],
    )
    service._write_state(state)

    snapshot, proposed_plan, flow_launch = service.review_proposed_plan(
        "conversation-proposed-plan",
        str(project_dir),
        "proposed-plan-inline",
        "approved",
        "Ready to implement.",
    )

    assert snapshot["chat_mode"] == "plan"
    assert snapshot["revision"] == state.revision + 1
    assert proposed_plan.status == "approved"
    assert proposed_plan.review_note == "Ready to implement."
    assert proposed_plan.written_change_request_path is not None
    assert proposed_plan.written_change_request_path.endswith(
        "changes/CR-2026-0002-reviewable-proposed-plan-artifacts-in-project-chat/request.md"
    )
    request_path = Path(proposed_plan.written_change_request_path)
    assert request_path.read_text(encoding="utf-8") == (
        "# Reviewable Proposed Plan Artifacts in Project Chat\n\n"
        "1. Add the backend artifact.\n2. Wire the review UI.\n"
    )
    assert flow_launch is not None
    assert flow_launch.flow_name == "software-development/implement-change-request.dot"
    assert flow_launch.launch_context == {
        "context.request.change_request_id": "CR-2026-0002-reviewable-proposed-plan-artifacts-in-project-chat",
        "context.request.change_request_path": (
            "changes/CR-2026-0002-reviewable-proposed-plan-artifacts-in-project-chat/request.md"
        ),
    }
    launch_segment = next(
        entry for entry in snapshot["segments"] if entry["artifact_id"] == flow_launch.id
    )
    assert launch_segment["kind"] == "flow_launch"
    assert launch_segment["turn_id"] == assistant_turn.id


def test_review_proposed_plan_rejects_and_locks_the_artifact(tmp_path: Path) -> None:
    service = project_chat.ProjectChatService(tmp_path)
    project_dir = tmp_path / "project"
    project_dir.mkdir(parents=True, exist_ok=True)

    assistant_turn = project_chat.ConversationTurn(
        id="turn-assistant-plan",
        role="assistant",
        content="Here is the proposed plan.",
        timestamp="2026-03-13T10:01:00Z",
        status="complete",
    )
    plan_segment = project_chat.ConversationSegment(
        id="segment-plan-inline",
        turn_id=assistant_turn.id,
        order=1,
        kind="plan",
        role="assistant",
        status="complete",
        timestamp="2026-03-13T10:01:05Z",
        updated_at="2026-03-13T10:01:05Z",
        completed_at="2026-03-13T10:01:05Z",
        content="# Reviewable Proposed Plan Artifacts in Project Chat\n\n1. Add the backend artifact.",
        artifact_id="proposed-plan-inline",
        source=project_chat.ConversationSegmentSource(),
    )
    service._write_state(
        project_chat.ConversationState(
            conversation_id="conversation-proposed-plan",
            project_path=str(project_dir),
            title="Plan review",
            created_at="2026-03-13T10:00:00Z",
            updated_at="2026-03-13T10:01:05Z",
            turns=[
                project_chat.ConversationTurn(
                    id="turn-user-plan",
                    role="user",
                    content="Draft the implementation plan.",
                    timestamp="2026-03-13T10:00:00Z",
                ),
                assistant_turn,
            ],
            segments=[plan_segment],
            proposed_plans=[
                project_chat.ProposedPlanArtifact(
                    id="proposed-plan-inline",
                    created_at="2026-03-13T10:01:05Z",
                    updated_at="2026-03-13T10:01:05Z",
                    title="Reviewable Proposed Plan Artifacts in Project Chat",
                    content=plan_segment.content,
                    project_path=str(project_dir),
                    conversation_id="conversation-proposed-plan",
                    source_turn_id=assistant_turn.id,
                    source_segment_id=plan_segment.id,
                ),
            ],
        )
    )

    snapshot, proposed_plan, flow_launch = service.review_proposed_plan(
        "conversation-proposed-plan",
        str(project_dir),
        "proposed-plan-inline",
        "rejected",
        "Needs acceptance criteria first.",
    )

    assert proposed_plan.status == "rejected"
    assert snapshot["revision"] == 1
    assert proposed_plan.review_note == "Needs acceptance criteria first."
    assert flow_launch is None
    assert snapshot["flow_launches"] == []
    with pytest.raises(ValueError, match="not reviewable"):
        service.review_proposed_plan(
            "conversation-proposed-plan",
            str(project_dir),
            "proposed-plan-inline",
            "approved",
            None,
        )


def test_proposed_plan_review_route_launches_in_owner_conversation_and_records_run_metadata(
    product_api_client,
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    project_dir = tmp_path / "project"
    project_dir.mkdir(parents=True, exist_ok=True)
    owner_conversation_id = "conversation-proposed-plan"
    other_conversation_id = "conversation-other"
    owner_assistant_turn = project_chat.ConversationTurn(
        id="turn-assistant-plan",
        role="assistant",
        content="Here is the proposed plan.",
        timestamp="2026-03-13T10:01:00Z",
        status="complete",
    )
    owner_plan_segment = project_chat.ConversationSegment(
        id="segment-plan-inline",
        turn_id=owner_assistant_turn.id,
        order=1,
        kind="plan",
        role="assistant",
        status="complete",
        timestamp="2026-03-13T10:01:05Z",
        updated_at="2026-03-13T10:01:05Z",
        completed_at="2026-03-13T10:01:05Z",
        content="# Reviewable Proposed Plan Artifacts in Project Chat\n\n1. Add the backend artifact.\n2. Wire the review UI.",
        artifact_id="proposed-plan-inline",
        source=project_chat.ConversationSegmentSource(),
    )
    service = _project_chat_service()
    monkeypatch.setattr("spark.workspace.conversations.artifacts.iso_now", lambda: "2026-03-13T10:02:00Z")
    service._write_state(
        project_chat.ConversationState(
            conversation_id=owner_conversation_id,
            project_path=str(project_dir),
            chat_mode="plan",
            title="Plan review",
            created_at="2026-03-13T10:00:00Z",
            updated_at="2026-03-13T10:01:05Z",
            turns=[
                project_chat.ConversationTurn(
                    id="turn-user-plan",
                    role="user",
                    content="Draft the implementation plan.",
                    timestamp="2026-03-13T10:00:00Z",
                ),
                owner_assistant_turn,
            ],
            segments=[owner_plan_segment],
            proposed_plans=[
                project_chat.ProposedPlanArtifact(
                    id="proposed-plan-inline",
                    created_at="2026-03-13T10:01:05Z",
                    updated_at="2026-03-13T10:01:05Z",
                    title="Reviewable Proposed Plan Artifacts in Project Chat",
                    content=owner_plan_segment.content,
                    project_path=str(project_dir),
                    conversation_id=owner_conversation_id,
                    source_turn_id=owner_assistant_turn.id,
                    source_segment_id=owner_plan_segment.id,
                ),
            ],
        )
    )
    service._write_state(
        project_chat.ConversationState(
            conversation_id=other_conversation_id,
            project_path=str(project_dir),
            title="Other thread",
            created_at="2026-03-13T10:00:00Z",
            updated_at="2026-03-13T10:02:00Z",
            turns=[
                project_chat.ConversationTurn(
                    id="turn-user-other",
                    role="user",
                    content="Something else.",
                    timestamp="2026-03-13T10:00:00Z",
                ),
                project_chat.ConversationTurn(
                    id="turn-assistant-other",
                    role="assistant",
                    content="Other reply.",
                    timestamp="2026-03-13T10:02:00Z",
                    status="complete",
                ),
            ],
        )
    )

    _seed_flow("software-development/implement-change-request.dot")
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
        **unexpected: object,
    ) -> dict[str, object]:
        assert "backend" not in unexpected
        assert not unexpected
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
        return {"status": "started", "run_id": "run-plan-123"}

    monkeypatch.setattr(attractor_client.AttractorApiClient, "start_pipeline", fake_start_pipeline)

    review_response = product_api_client.post(
        f"/workspace/api/conversations/{owner_conversation_id}/proposed-plans/proposed-plan-inline/review",
        json={
            "project_path": str(project_dir),
            "disposition": "approved",
            "review_note": "Ready to implement.",
        },
    )

    assert review_response.status_code == 200
    approved_snapshot = review_response.json()
    assert approved_snapshot["revision"] == 2
    approved_plan = next(
        entry for entry in approved_snapshot["proposed_plans"] if entry["id"] == "proposed-plan-inline"
    )
    assert approved_snapshot["chat_mode"] == "plan"
    assert approved_plan["status"] == "approved"
    assert approved_plan["review_note"] == "Ready to implement."
    assert approved_plan["run_id"] == "run-plan-123"
    assert approved_plan["written_change_request_path"].endswith(
        "changes/CR-2026-0001-reviewable-proposed-plan-artifacts-in-project-chat/request.md"
    )
    flow_launch = next(
        entry for entry in approved_snapshot["flow_launches"] if entry["id"] == approved_plan["flow_launch_id"]
    )
    assert flow_launch["conversation_id"] == owner_conversation_id
    assert flow_launch["run_id"] == "run-plan-123"
    assert next(
        entry for entry in approved_snapshot["segments"] if entry["artifact_id"] == flow_launch["id"]
    )["turn_id"] == owner_assistant_turn.id
    assert _project_chat_service().get_snapshot(other_conversation_id, str(project_dir))["flow_launches"] == []
    assert start_calls == [
        {
            "run_id": None,
            "flow_name": "software-development/implement-change-request.dot",
            "working_directory": str(project_dir),
            "model": None,
            "goal": (
                "Implement the approved change request written to "
                "changes/CR-2026-0001-reviewable-proposed-plan-artifacts-in-project-chat/request.md."
            ),
            "launch_context": {
                "context.request.change_request_id": (
                    "CR-2026-0001-reviewable-proposed-plan-artifacts-in-project-chat"
                ),
                "context.request.change_request_path": (
                    "changes/CR-2026-0001-reviewable-proposed-plan-artifacts-in-project-chat/request.md"
                ),
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
    captured_events: list[project_chat.TurnStreamEvent] = []
    client = StubChatClient()
    session._client = client

    def run_turn(**kwargs) -> CodexAppServerTurnResult:
        on_event = kwargs["on_event"]
        on_event(TurnStreamEvent(kind="content_delta", channel="assistant", content_delta="Ack"))
        on_event(
            TurnStreamEvent(
                kind="content_completed",
                channel="assistant",
                source=TurnStreamSource(item_id="msg-1"),
                content_delta="Ack",
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
    assert [event.kind for event in captured_events] == ["content_delta", "content_completed"]
    assert captured_events[1].source.item_id == "msg-1"
    assert captured_events[1].phase == "final_answer"
    assert captured_events[-1].content_delta == "Ack"


def test_chat_session_preserves_normalized_app_turn_id_when_local_turn_id_missing() -> None:
    session = project_chat.CodexAppServerChatSession("/tmp/project")
    captured_events: list[project_chat.TurnStreamEvent] = []

    session._forward_normalized_turn_event(
        TurnStreamEvent(
            kind="content_completed",
            channel="assistant",
            source=TurnStreamSource(app_turn_id="normalized-turn", item_id="msg-1"),
            content_delta="Ack",
            phase="final_answer",
        ),
        on_event=captured_events.append,
        tool_calls_by_id={},
        current_app_turn_id=None,
    )

    assert len(captured_events) == 1
    assert captured_events[0].source.app_turn_id == "normalized-turn"
    assert captured_events[0].source.item_id == "msg-1"


def test_chat_session_handles_command_output_update_without_reasoning_fallback_helper(monkeypatch) -> None:
    session = project_chat.CodexAppServerChatSession("/tmp/project")
    captured_events: list[project_chat.TurnStreamEvent] = []
    client = StubChatClient()
    session._client = client

    def run_turn(**kwargs) -> CodexAppServerTurnResult:
        on_event = kwargs["on_event"]
        on_event(
            TurnStreamEvent(
                kind="content_delta",
                source=TurnStreamSource(item_id="rs-1", summary_index=0),
                channel="reasoning",
                content_delta="Thinking...",
            )
        )
        on_event(
            TurnStreamEvent(
                kind="tool_call_updated",
                source=TurnStreamSource(item_id="cmd-1", raw_kind="command_output_delta"),
                content_delta="output",
            )
        )
        on_event(
            TurnStreamEvent(
                kind="content_delta",
                channel="assistant",
                source=TurnStreamSource(item_id="msg-1"),
                content_delta="Ack",
            )
        )
        on_event(
            TurnStreamEvent(
                kind="content_completed",
                channel="assistant",
                source=TurnStreamSource(item_id="msg-1"),
                content_delta="Ack",
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
    assert [event.kind for event in captured_events] == ["content_delta", "content_delta", "content_completed"]


def test_chat_session_emits_assistant_completed_for_commentary_item(monkeypatch) -> None:
    session = project_chat.CodexAppServerChatSession("/tmp/project")
    captured_events: list[project_chat.TurnStreamEvent] = []
    client = StubChatClient()
    session._client = client

    def run_turn(**kwargs) -> CodexAppServerTurnResult:
        on_event = kwargs["on_event"]
        on_event(
            TurnStreamEvent(
                kind="content_delta",
                channel="assistant",
                source=TurnStreamSource(item_id="msg-1"),
                content_delta="I’m drafting the proposal now.",
            )
        )
        on_event(
            TurnStreamEvent(
                kind="content_completed",
                channel="assistant",
                source=TurnStreamSource(item_id="msg-1"),
                content_delta="I’m drafting the proposal now.",
                phase="commentary",
            )
        )
        on_event(
            TurnStreamEvent(
                kind="content_delta",
                channel="assistant",
                source=TurnStreamSource(item_id="msg-2"),
                content_delta="Done.",
            )
        )
        on_event(
            TurnStreamEvent(
                kind="content_completed",
                channel="assistant",
                source=TurnStreamSource(item_id="msg-2"),
                content_delta="Done.",
                phase="final_answer",
            )
        )
        return _completed_turn_result(thread_id=kwargs["thread_id"], assistant_message="Done.")

    client.run_turn_handler = run_turn

    result = session.turn("hello", None, on_event=captured_events.append)

    assert result.assistant_message == "Done."
    assert [event.kind for event in captured_events] == [
        "content_delta",
        "content_completed",
        "content_delta",
        "content_completed",
    ]
    assert captured_events[0].source.item_id == "msg-1"
    assert captured_events[1].phase == "commentary"
    assert captured_events[2].source.item_id == "msg-2"
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
            reasoning_effort: str | None = None,
            on_event=None,
            on_dynamic_tool_call=None,
        ) -> project_chat.ChatTurnResult:
            assert self.raw_logger is not None
            self.raw_logger("outgoing", '{"jsonrpc":"2.0","id":1,"method":"turn/start"}')
            self.raw_logger("incoming", '{"jsonrpc":"2.0","method":"turn/completed","params":{"turn":{"status":"completed"}}}')
            if on_event is not None:
                on_event(
                    project_chat.TurnStreamEvent(
                        kind="content_completed",
                channel="assistant",
                        source=TurnStreamSource(app_turn_id="app-turn-1", item_id="msg-1"),
                        content_delta="Logged.",
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
            reasoning_effort: str | None = None,
            on_event=None,
            on_dynamic_tool_call=None,
        ) -> project_chat.ChatTurnResult:
            entered_turn.set()
            assert finish_turn.wait(timeout=2)
            if on_event is not None:
                on_event(
                        project_chat.TurnStreamEvent(
                            kind="content_delta",
                            channel="reasoning",
                            content_delta="Checking the repository.",
                        )
                )
                on_event(
                    project_chat.TurnStreamEvent(
                        kind="content_completed",
                channel="assistant",
                        source=TurnStreamSource(app_turn_id="app-turn-1", item_id="msg-1"),
                        content_delta="ACK",
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
            reasoning_effort: str | None = None,
            on_event=None,
            on_dynamic_tool_call=None,
        ) -> project_chat.ChatTurnResult:
            entered_turn.set()
            assert finish_turn.wait(timeout=2)
            if on_event is not None:
                on_event(
                        project_chat.TurnStreamEvent(
                            kind="content_delta",
                            channel="reasoning",
                            content_delta="Checking whether a flow run request makes sense.",
                        )
                )
                on_event(
                    project_chat.TurnStreamEvent(
                        kind="content_delta",
                channel="assistant",
                        source=TurnStreamSource(app_turn_id="app-turn-1", item_id="msg-1"),
                        content_delta="Working on it",
                        phase="final_answer",
                    )
                )
                on_event(
                    project_chat.TurnStreamEvent(
                        kind="content_completed",
                channel="assistant",
                        source=TurnStreamSource(app_turn_id="app-turn-1", item_id="msg-1"),
                        content_delta="Working on it",
                        phase="final_answer",
                    )
                )
                on_event(
                    project_chat.TurnStreamEvent(
                        kind="tool_call_started",
                        source=TurnStreamSource(item_id="call-pwd"),
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


def test_send_project_conversation_turn_endpoint_persists_token_usage_in_snapshot(
    product_api_client,
    monkeypatch,
    tmp_path: Path,
) -> None:
    service = _project_chat_service()
    entered_turn = threading.Event()
    finish_turn = threading.Event()
    token_usage = {
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

    class FakeSession:
        def turn(
            self,
            prompt: str,
            model: str | None,
            *,
            chat_mode: str = "chat",
            reasoning_effort: str | None = None,
            on_event=None,
            on_dynamic_tool_call=None,
        ) -> project_chat.ChatTurnResult:
            entered_turn.set()
            assert finish_turn.wait(timeout=2)
            if on_event is not None:
                on_event(
                    project_chat.TurnStreamEvent(
                        kind="token_usage_updated",
                        token_usage=token_usage,
                    )
                )
                on_event(
                    project_chat.TurnStreamEvent(
                        kind="content_completed",
                channel="assistant",
                        source=TurnStreamSource(app_turn_id="app-turn-usage", item_id="msg-usage"),
                        content_delta="Usage captured.",
                        phase="final_answer",
                    )
                )
            return project_chat.ChatTurnResult(
                assistant_message='{"assistant_message":"Usage captured."}',
                token_usage=token_usage,
            )

    monkeypatch.setattr(service, "_build_session", lambda *args: FakeSession())

    response = product_api_client.post(
        "/workspace/api/conversations/conversation-token-usage/turns",
        json={
            "project_path": str(tmp_path),
            "message": "show token usage",
            "model": "gpt-test",
        },
    )

    assert response.status_code == 200
    assert entered_turn.wait(timeout=2)

    finish_turn.set()
    deadline = time.time() + 2.0
    final_snapshot: dict[str, Any] | None = None
    while time.time() < deadline:
        candidate = service.get_snapshot("conversation-token-usage", str(tmp_path))
        if candidate["turns"][-1]["status"] == "complete":
            final_snapshot = candidate
            break
        time.sleep(0.02)

    assert final_snapshot is not None
    assistant_turn = final_snapshot["turns"][-1]
    assert assistant_turn["content"] == "Usage captured."
    assert assistant_turn["token_usage"] == token_usage


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


def test_update_project_conversation_settings_endpoint_updates_model_without_message(
    product_api_client,
    tmp_path: Path,
) -> None:
    project_path = str(tmp_path.resolve())
    response = product_api_client.put(
        "/workspace/api/conversations/conversation-settings/settings",
        json={
            "project_path": project_path,
            "model": "gpt-5.4",
            "reasoning_effort": "high",
        },
    )

    assert response.status_code == 200
    payload = response.json()
    assert payload["conversation_id"] == "conversation-settings"
    assert payload["chat_mode"] == "chat"
    assert payload["model"] == "gpt-5.4"
    assert payload["reasoning_effort"] == "high"
    assert payload["turns"] == []


def test_mutation_endpoints_return_bounded_historical_tool_output(
    product_api_client,
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    large_output = "api-output" * 1000
    state = _seed_tool_output_conversation(
        conversation_id="conversation-api-tool-output",
        project_path=str(tmp_path),
        output=large_output,
    )

    settings_response = product_api_client.put(
        f"/workspace/api/conversations/{state.conversation_id}/settings",
        json={
            "project_path": state.project_path,
            "chat_mode": "plan",
        },
    )
    assert settings_response.status_code == 200
    _assert_ui_tool_output_preview(settings_response.json(), large_output)

    finish_turn = threading.Event()

    class BlockingSession:
        def turn(self, *args, **kwargs) -> project_chat.ChatTurnResult:
            assert finish_turn.wait(timeout=2)
            on_event = kwargs.get("on_event")
            if on_event is not None:
                on_event(
                    project_chat.TurnStreamEvent(
                        kind="content_completed",
                        channel="assistant",
                        source=TurnStreamSource(app_turn_id="app-turn-1", item_id="msg-1"),
                        content_delta="Done.",
                        phase="final_answer",
                    )
                )
            return project_chat.ChatTurnResult(assistant_message="Done.")

    monkeypatch.setattr(_project_chat_service(), "_build_session", lambda *args, **kwargs: BlockingSession())
    turn_response = product_api_client.post(
        f"/workspace/api/conversations/{state.conversation_id}/turns",
        json={
            "project_path": state.project_path,
            "message": "Continue.",
        },
    )
    assert turn_response.status_code == 200
    _assert_ui_tool_output_preview(turn_response.json(), large_output)
    finish_turn.set()


def test_review_endpoints_return_bounded_historical_tool_output(
    product_api_client,
    tmp_path: Path,
) -> None:
    service = _project_chat_service()
    flow_output = "api-flow-review-output" * 600
    flow_state = _seed_tool_output_conversation(
        conversation_id="conversation-api-flow-review-output",
        project_path=str(tmp_path),
        output=flow_output,
    )
    flow_state.flow_run_requests.append(
        project_chat.FlowRunRequest(
            id="flow-request-1",
            created_at=TEST_TIMESTAMP,
            updated_at=TEST_TIMESTAMP,
            flow_name=TEST_DISPATCH_FLOW,
            summary="Run implementation.",
            project_path=flow_state.project_path,
            conversation_id=flow_state.conversation_id,
            source_turn_id="turn-assistant",
        )
    )
    service._write_state(flow_state)

    flow_response = product_api_client.post(
        f"/workspace/api/conversations/{flow_state.conversation_id}/flow-run-requests/flow-request-1/review",
        json={
            "project_path": flow_state.project_path,
            "disposition": "rejected",
            "message": "Not now.",
        },
    )
    assert flow_response.status_code == 200
    _assert_ui_tool_output_preview(flow_response.json(), flow_output)

    plan_output = "api-plan-review-output" * 600
    project_dir = tmp_path / "plan-project"
    project_dir.mkdir(parents=True, exist_ok=True)
    plan_state = _seed_tool_output_conversation(
        conversation_id="conversation-api-plan-review-output",
        project_path=str(project_dir),
        output=plan_output,
    )
    plan_segment = project_chat.ConversationSegment(
        id="segment-plan-inline",
        turn_id="turn-assistant",
        order=2,
        kind="plan",
        role="assistant",
        status="complete",
        timestamp=TEST_TIMESTAMP,
        updated_at=TEST_TIMESTAMP,
        completed_at=TEST_TIMESTAMP,
        content="# Proposed Plan\n\nDo the work.",
        artifact_id="proposed-plan-inline",
        source=project_chat.ConversationSegmentSource(),
    )
    plan_state.segments.append(plan_segment)
    plan_state.proposed_plans.append(
        project_chat.ProposedPlanArtifact(
            id="proposed-plan-inline",
            created_at=TEST_TIMESTAMP,
            updated_at=TEST_TIMESTAMP,
            title="Proposed Plan",
            content=plan_segment.content,
            project_path=str(project_dir),
            conversation_id=plan_state.conversation_id,
            source_turn_id="turn-assistant",
            source_segment_id=plan_segment.id,
        )
    )
    service._write_state(plan_state)

    plan_response = product_api_client.post(
        f"/workspace/api/conversations/{plan_state.conversation_id}/proposed-plans/proposed-plan-inline/review",
        json={
            "project_path": plan_state.project_path,
            "disposition": "rejected",
            "review_note": "Needs work.",
        },
    )
    assert plan_response.status_code == 200
    _assert_ui_tool_output_preview(plan_response.json(), plan_output)


def test_project_chat_models_endpoint_returns_normalized_model_metadata(
    product_api_client,
    monkeypatch,
    tmp_path: Path,
) -> None:
    service = _project_chat_service()

    def fake_list_chat_models(project_path: str) -> dict[str, Any]:
        assert project_path == str(tmp_path.resolve())
        return {
            "models": [
                {
                    "id": "gpt-5.4",
                    "display": "GPT-5.4",
                    "is_default": True,
                    "supported_reasoning_efforts": ["low", "medium", "high", "xhigh"],
                    "default_reasoning_effort": "medium",
                }
            ]
        }

    monkeypatch.setattr(service, "list_chat_models", fake_list_chat_models)

    response = product_api_client.get(
        "/workspace/api/projects/chat-models",
        params={"project_path": str(tmp_path)},
    )

    assert response.status_code == 200
    assert response.json() == {
        "models": [
            {
                "id": "gpt-5.4",
                "display": "GPT-5.4",
                "is_default": True,
                "supported_reasoning_efforts": ["low", "medium", "high", "xhigh"],
                "default_reasoning_effort": "medium",
            }
        ]
    }


def test_send_project_conversation_turn_endpoint_switches_chat_mode_atomically(
    product_api_client,
    monkeypatch,
    tmp_path: Path,
) -> None:
    service = _project_chat_service()
    captured_calls: list[dict[str, str | None]] = []

    class PlainTextSession:
        def turn(
            self,
            prompt: str,
            model: str | None,
            *,
            chat_mode: str = "chat",
            reasoning_effort: str | None = None,
            on_event=None,
            on_dynamic_tool_call=None,
        ) -> project_chat.ChatTurnResult:
            captured_calls.append(
                {
                    "model": model,
                    "reasoning_effort": reasoning_effort,
                    "chat_mode": chat_mode,
                }
            )
            if on_event is not None:
                on_event(
                    project_chat.TurnStreamEvent(
                        kind="content_completed",
                channel="assistant",
                        source=TurnStreamSource(app_turn_id="app-turn-1", item_id="msg-1"),
                        content_delta="Mode switch acknowledged.",
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
            "model": "gpt-5.4",
            "reasoning_effort": "low",
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
    assert final_snapshot["model"] == "gpt-5.4"
    assert final_snapshot["reasoning_effort"] == "low"
    assert captured_calls == [
        {
            "model": "gpt-5.4",
            "reasoning_effort": "low",
            "chat_mode": "plan",
        }
    ]


def test_submit_project_conversation_request_user_input_endpoint_updates_existing_segment(
    product_api_client,
    monkeypatch,
    tmp_path: Path,
) -> None:
    service = _project_chat_service()

    class WaitingRequestSession:
        def __init__(self) -> None:
            self._answer_event = threading.Event()
            self._answers: dict[str, str] = {}

        def has_pending_request_user_input(self, request_id: str) -> bool:
            return request_id == "request-1" and not self._answer_event.is_set()

        def submit_request_user_input_answers(self, request_id: str, answers: dict[str, str]) -> bool:
            if request_id != "request-1":
                return False
            self._answers = dict(answers)
            self._answer_event.set()
            return True

        def turn(
            self,
            prompt: str,
            model: str | None,
            *,
            chat_mode: str = "chat",
            reasoning_effort: str | None = None,
            on_event=None,
            on_dynamic_tool_call=None,
        ) -> project_chat.ChatTurnResult:
            if on_event is not None:
                on_event(
                    project_chat.TurnStreamEvent(
                        kind="request_user_input_requested",
                        source=TurnStreamSource(app_turn_id="app-turn-1", item_id="request-1"),
                        request_user_input=_request_user_input_record(),
                    )
                )
            assert self._answer_event.wait(timeout=2)
            assistant_message = f"ANSWER={self._answers['path_choice']}"
            if on_event is not None:
                on_event(
                    project_chat.TurnStreamEvent(
                        kind="content_completed",
                channel="assistant",
                        source=TurnStreamSource(app_turn_id="app-turn-1", item_id="msg-1"),
                        content_delta=assistant_message,
                        phase="final_answer",
                    )
                )
            return project_chat.ChatTurnResult(assistant_message=assistant_message)

    waiting_session = WaitingRequestSession()
    monkeypatch.setattr(service, "_build_session", lambda conversation_id, project_path: waiting_session)
    with service._sessions_lock:
        service._sessions["conversation-request-answer"] = waiting_session

    start_response = product_api_client.post(
        "/workspace/api/conversations/conversation-request-answer/turns",
        json={
            "project_path": str(tmp_path),
            "message": "Ask for a decision.",
            "chat_mode": "plan",
        },
    )

    assert start_response.status_code == 200

    deadline = time.time() + 2.0
    while time.time() < deadline:
        candidate = service.get_snapshot("conversation-request-answer", str(tmp_path))
        if any(segment["kind"] == "request_user_input" for segment in candidate["segments"]):
            break
        time.sleep(0.02)

    response = product_api_client.post(
        "/workspace/api/conversations/conversation-request-answer/request-user-input/request-1/answer",
        json={
            "project_path": str(tmp_path),
            "answers": {
                "path_choice": "Inline card",
                "constraints": "Keep the request inline.",
            },
        },
    )

    assert response.status_code == 200
    payload = response.json()
    request_segments = [segment for segment in payload["segments"] if segment["kind"] == "request_user_input"]
    assert len(request_segments) == 1
    assert request_segments[0]["request_user_input"]["status"] == "answered"
    assert request_segments[0]["request_user_input"]["answers"]["path_choice"] == "Inline card"
    assert "delivery_status" not in request_segments[0]["request_user_input"]


def test_submit_project_conversation_request_user_input_endpoint_expires_without_live_session(
    product_api_client,
    tmp_path: Path,
) -> None:
    service = _project_chat_service()
    project_path = str(tmp_path)
    conversation_id = "conversation-request-answer-queued"
    service._write_state(
        project_chat.ConversationState(
            conversation_id=conversation_id,
            project_path=project_path,
            turns=[
                project_chat.ConversationTurn(
                    id="turn-user-1",
                    role="user",
                    content="Ask me the missing question.",
                    timestamp="2026-04-17T12:00:00Z",
                ),
                project_chat.ConversationTurn(
                    id="turn-assistant-1",
                    role="assistant",
                    content="",
                    timestamp="2026-04-17T12:00:01Z",
                    status="streaming",
                    parent_turn_id="turn-user-1",
                ),
            ],
            segments=[
                project_chat.ConversationSegment(
                    id="segment-request-user-input-app-turn-1-request-1",
                    turn_id="turn-assistant-1",
                    order=1,
                    kind="request_user_input",
                    role="system",
                    status="pending",
                    timestamp="2026-04-17T12:00:02Z",
                    updated_at="2026-04-17T12:00:02Z",
                    content="Which path should I take?",
                    request_user_input=_request_user_input_record(),
                ),
            ],
        )
    )

    response = product_api_client.post(
        f"/workspace/api/conversations/{conversation_id}/request-user-input/request-1/answer",
        json={
            "project_path": project_path,
            "answers": {
                "path_choice": "Inline card",
                "constraints": "Keep the request inline.",
            },
        },
    )

    assert response.status_code == 200
    payload = response.json()
    request_segment = next(segment for segment in payload["segments"] if segment["kind"] == "request_user_input")
    assert request_segment["status"] == "failed"
    assert request_segment["request_user_input"]["status"] == "expired"
    assert request_segment["request_user_input"]["answers"]["path_choice"] == "Inline card"
    assert request_segment["error"] == "The requested input expired before the answer could be used."
    assert payload["turns"][-1]["status"] == "failed"
    assert payload["turns"][-1]["error"] == "The requested input expired before the answer could be used."


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
                "kind": "content_delta",
                "content_delta": "hel",
            },
            {
                "id": "event-reasoning-1",
                "turn_id": "turn-assistant-1",
                "sequence": 2,
                "timestamp": "2026-03-07T18:00:01Z",
                "kind": "content_delta",
                "content_delta": "Thinking about the repository structure.",
            },
            {
                "id": "event-assistant-delta-2",
                "turn_id": "turn-assistant-1",
                "sequence": 3,
                "timestamp": "2026-03-07T18:00:02Z",
                "kind": "content_delta",
                "content_delta": "lo",
            },
            {
                "id": "event-assistant-completed-1",
                "turn_id": "turn-assistant-1",
                "sequence": 4,
                "timestamp": "2026-03-07T18:00:03Z",
                "kind": "content_completed",
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
