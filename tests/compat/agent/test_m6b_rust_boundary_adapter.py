from __future__ import annotations

from contextlib import contextmanager
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import sys
import threading

import pytest

import spark.chat.service as project_chat
from tests.compat import harness
from attractor.api.codex_backends import ProviderRouterBackend, UnifiedAgentBackend
from attractor.dsl import parse_dot
from attractor.engine.context import Context
from attractor.engine.context_contracts import ContextWriteContract
from attractor.engine.outcome import FailureKind, Outcome, OutcomeStatus
from attractor.handlers import HandlerRunner, build_default_registry
from attractor.handlers.base import ChildInterventionRequest
from spark.chat.session import (
    RustBoundaryError,
    SerializedRustAgentBoundary,
    UnifiedAgentChatSession,
    default_rust_agent_boundary_command,
    normalize_boundary_provider_selector,
)
from spark.workspace.storage import ensure_project_paths
from spark.workspace.conversations.utils import normalize_project_path_value

M6_ITEM_ID = "M6-I3"
M6_REQUIREMENTS = ("REQ-001", "REQ-004", "REQ-014", "REQ-015")
M6_DECISIONS = (
    "CD-CODING-AGENT-RUST-001",
    "CD-CODING-AGENT-RUST-003",
    "CD-CODING-AGENT-RUST-004",
    "CD-CODING-AGENT-RUST-011",
    "CD-CODING-AGENT-RUST-012",
)


class FakeRustBoundary:
    def __init__(self, *, agent_outputs=None, codergen_outputs=None, steer_outputs=None, before_codergen_output=None):
        self.agent_requests = []
        self.codergen_requests = []
        self.steer_requests = []
        self._agent_outputs = list(agent_outputs or [])
        self._codergen_outputs = list(codergen_outputs or [])
        self._steer_outputs = list(steer_outputs or [{"status": "delivered", "delivery_mode": "rust_boundary_codergen_turn"}])
        self.before_codergen_output = before_codergen_output

    def run_agent_turn(self, payload):
        self.agent_requests.append(payload)
        output = self._agent_outputs.pop(0)
        if isinstance(output, BaseException):
            raise output
        return output

    def run_codergen(self, payload):
        self.codergen_requests.append(payload)
        if self.before_codergen_output is not None:
            self.before_codergen_output()
        output = self._codergen_outputs.pop(0)
        if isinstance(output, BaseException):
            raise output
        return output

    def steer_codergen_turn(self, payload):
        self.steer_requests.append(payload)
        output = self._steer_outputs.pop(0)
        if isinstance(output, BaseException):
            raise output
        return output


@contextmanager
def _openai_compatible_server(responses):
    records = []
    pending_responses = list(responses)

    class Handler(BaseHTTPRequestHandler):
        def do_POST(self):
            length = int(self.headers.get("content-length", "0") or 0)
            raw_body = self.rfile.read(length).decode("utf-8")
            body = json.loads(raw_body) if raw_body else {}
            records.append({"path": self.path, "headers": dict(self.headers), "body": body})
            response = pending_responses.pop(0) if pending_responses else {"status": 500, "payload": {"error": "unexpected request"}}
            payload = json.dumps(response["payload"]).encode("utf-8")
            self.send_response(int(response.get("status", 200)))
            self.send_header("content-type", "application/json")
            self.send_header("content-length", str(len(payload)))
            self.end_headers()
            self.wfile.write(payload)

        def log_message(self, format, *args):  # noqa: A002
            return

    server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        yield f"http://127.0.0.1:{server.server_port}/v1", records
    finally:
        server.shutdown()
        server.server_close()
        thread.join(2)


def _chat_completion_response(text: str, *, model: str = "profile-model"):
    return {
        "id": "chatcmpl-boundary-test",
        "object": "chat.completion",
        "created": 0,
        "model": model,
        "choices": [
            {
                "index": 0,
                "message": {"role": "assistant", "content": text},
                "finish_reason": "stop",
            }
        ],
        "usage": {"prompt_tokens": 8, "completion_tokens": 5, "total_tokens": 13},
    }


def _write_test_profile(config_dir, base_url: str) -> None:
    config_dir.mkdir(parents=True, exist_ok=True)
    config_dir.joinpath("llm-profiles.toml").write_text(
        f'''
[profiles.team-profile]
label = "Boundary Test"
provider = "openai_compatible"
base_url = "{base_url}"
models = ["profile-model"]
default_model = "profile-model"
'''.strip(),
        encoding="utf-8",
    )


def _committed_boundary_command_or_skip(monkeypatch):
    monkeypatch.delenv("SPARK_RUST_AGENT_BOUNDARY_COMMAND", raising=False)
    command = default_rust_agent_boundary_command()
    if command is None:
        pytest.skip("the committed Rust boundary binary or cargo is required for this compatibility check")
    return command


def _successful_status_envelope(label: str = "done") -> str:
    return json.dumps(
        {
            "outcome": "success",
            "preferred_label": label,
            "suggested_next_ids": [],
            "context_updates": {"context.allowed": label},
            "notes": "completed through committed boundary",
            "failure_reason": "",
        }
    )


def test_m6_integration_boundary_snapshot_fixture_matches_golden(
    compat_fixture_root,
    compat_update_goldens,
    tmp_path,
):
    manifest = _m6_integration_boundary_manifest(tmp_path)
    harness.validate_manifest_coverage(
        manifest,
        requirement_ids=M6_REQUIREMENTS,
        decision_ids=M6_DECISIONS,
    )
    fixture_path = compat_fixture_root / "agent/m6-integration-boundary-snapshot.json"
    if compat_update_goldens:
        harness.write_manifest(fixture_path, manifest)
    expected = harness.load_manifest(fixture_path)
    harness.assert_agent_manifest_matches_golden(manifest, expected)


def test_project_chat_service_uses_committed_rust_boundary_for_profiled_chat_snapshot(
    tmp_path,
    monkeypatch,
):
    _committed_boundary_command_or_skip(monkeypatch)
    with _openai_compatible_server(
        [
            {
                "payload": _chat_completion_response(
                    "Rust-backed chat answer.",
                    model="profile-model",
                )
            }
        ]
    ) as (base_url, records):
        spark_home = tmp_path / "spark-home"
        _write_test_profile(spark_home / "config", base_url)
        service = project_chat.ProjectChatService(spark_home)
        snapshot = service.send_turn(
            "conversation-rust-chat",
            str(tmp_path / "project"),
            "Please answer from the Rust boundary.",
            model="profile-model",
            provider="codex",
            llm_profile="team-profile",
            reasoning_effort="HIGH",
        )

    assert snapshot["provider"] == "codex"
    assert snapshot["model"] == "profile-model"
    assert snapshot["llm_profile"] == "team-profile"
    assert snapshot["reasoning_effort"] == "high"
    assert snapshot["turns"][-1]["status"] == "complete"
    assert snapshot["turns"][-1]["content"] == "Rust-backed chat answer."
    assert snapshot["turns"][-1]["token_usage"]["total"]["totalTokens"] == 13
    assert records and records[0]["path"] == "/v1/chat/completions"
    request_body = records[0]["body"]
    assert request_body["model"] == "profile-model"
    assert request_body["metadata"]["spark.runtime.provider_selector"] == "codex"
    assert request_body["metadata"]["spark.runtime.provider"] == "openai_compatible"
    assert request_body["metadata"]["spark.runtime.llm_profile"] == "team-profile"
    assert request_body["metadata"]["spark.runtime.reasoning_effort"] == "high"


def _m6_integration_boundary_manifest(tmp_path):
    return {
        "schema_version": "compat-agent-v1",
        "fixture_id": "agent/m6-integration-boundary-snapshot",
        "item_id": M6_ITEM_ID,
        "requirements": list(M6_REQUIREMENTS),
        "decisions": list(M6_DECISIONS),
        "provenance": {
            "oracle": "m6-public-python-facade-workspace-codergen-boundary",
            "interfaces": [
                "spark.chat.session.UnifiedAgentChatSession",
                "spark.chat.service.ProjectChatService",
                "attractor.api.codex_backends.ProviderRouterBackend",
                "attractor.handlers.HandlerRunner",
            ],
        },
        "scenario": "m6_integration_boundary_snapshot",
        "input": {
            "selectors": ["codex", "openai-compatible", "openai", "anthropic", "gemini"],
            "chat_provider": "codex",
            "profile": "team-profile",
            "codergen_provider": "openai-compatible",
        },
        "observation": {
            "facade_events": _m6_facade_event_observation(tmp_path / "facade"),
            "workspace_snapshot": _m6_workspace_snapshot_observation(tmp_path / "workspace"),
            "codergen_runtime": _m6_codergen_observation(tmp_path / "codergen"),
            "selectors": {
                selector: normalize_boundary_provider_selector(selector)
                for selector in ["codex", "openai-compatible", "openai", "anthropic", "gemini"]
            },
            "failures": _m6_failure_observation(tmp_path / "failures"),
        },
    }


def _m6_facade_event_observation(project_path):
    requested = threading.Event()
    events = []
    boundary = FakeRustBoundary(
        agent_outputs=[
            {
                "events": [
                    {"kind": "session_start", "source": {"backend": "rust", "raw_kind": "SESSION_START"}},
                    {
                        "kind": "content_delta",
                        "channel": "reasoning",
                        "content_delta": "checking",
                        "source": {"backend": "rust", "raw_kind": "assistant_reasoning_delta"},
                    },
                    {
                        "kind": "model_tool_call_start",
                        "tool_call": {
                            "id": "model-call-1",
                            "kind": "model_tool_call",
                            "status": "proposed",
                            "title": "search",
                        },
                        "source": {"backend": "rust", "raw_kind": "model_tool_call_start"},
                    },
                    {
                        "kind": "tool_call_started",
                        "tool_call": {
                            "id": "exec-call-1",
                            "kind": "command_execution",
                            "status": "running",
                            "title": "Run command",
                            "command": "printf ok",
                        },
                    },
                    {
                        "kind": "tool_call_failed",
                        "tool_call": {
                            "id": "exec-call-1",
                            "kind": "command_execution",
                            "status": "failed",
                            "title": "Run command",
                            "command": "printf ok",
                            "output": "recoverable tool failure visible to model",
                        },
                    },
                    {
                        "kind": "token_usage_updated",
                        "usage": {"inputTokens": 5, "outputTokens": 7, "totalTokens": 12},
                    },
                    {"kind": "warning", "message": "bounded warning"},
                    {
                        "kind": "request_user_input_requested",
                        "request_user_input": {
                            "request_id": "request-1",
                            "status": "pending",
                            "questions": [
                                {
                                    "id": "decision",
                                    "header": "Decision",
                                    "question": "Continue?",
                                    "question_type": "MULTIPLE_CHOICE",
                                    "options": [{"label": "Continue", "description": "Proceed"}],
                                }
                            ],
                        },
                    },
                    {
                        "kind": "content_completed",
                        "channel": "assistant",
                        "content_delta": "Facade answer.",
                        "phase": "final_answer",
                    },
                    {"kind": "turn_limit", "message": "turn budget reached"},
                    {"kind": "processing_end", "status": "idle"},
                    {"kind": "session_end", "status": "closed"},
                ],
                "final_assistant_text": "Facade answer.",
            }
        ]
    )
    session = UnifiedAgentChatSession(
        str(project_path),
        provider="codex",
        model="profile-model",
        llm_profile="team-profile",
        conversation_id="conversation-m6-facade",
        boundary=boundary,
    )
    result_holder = {}

    def on_event(event):
        events.append(event)
        if event.kind == "request_user_input_requested":
            requested.set()

    thread = threading.Thread(
        target=lambda: result_holder.setdefault("result", session.turn("hello", None, on_event=on_event))
    )
    thread.start()
    assert requested.wait(2)
    assert session.submit_request_user_input_answers("request-1", {"decision": "Continue"})
    thread.join(2)
    assert not thread.is_alive()

    return {
        "assistant_message": result_holder["result"].assistant_message,
        "event_kinds": [event.kind for event in events],
        "channels": [event.channel for event in events if event.channel],
        "model_tool_events": [
            {
                "kind": event.kind,
                "tool_kind": event.tool_call.kind,
                "status": event.tool_call.status,
            }
            for event in events
            if event.kind.startswith("model_tool_call") and event.tool_call is not None
        ],
        "execution_tool_events": [
            {
                "kind": event.kind,
                "tool_kind": event.tool_call.kind,
                "status": event.tool_call.status,
                "output": event.tool_call.output,
            }
            for event in events
            if event.kind.startswith("tool_call") and event.tool_call is not None
        ],
        "request_user_input": [
            event.request_user_input.to_dict()
            for event in events
            if event.kind == "request_user_input_requested" and event.request_user_input is not None
        ],
        "usage_totals": [
            event.token_usage["total"]["totalTokens"]
            for event in events
            if event.kind == "token_usage_updated" and isinstance(event.token_usage, dict)
        ],
        "warnings": [event.message for event in events if event.kind == "warning"],
        "lifecycle": [
            event.kind
            for event in events
            if event.kind in {"session_start", "turn_limit", "processing_end", "session_end"}
        ],
    }


def _m6_workspace_snapshot_observation(project_path):
    boundary = FakeRustBoundary(
        agent_outputs=[
            {
                "events": [
                    {
                        "kind": "content_delta",
                        "channel": "reasoning",
                        "content_delta": "reasoning through public service",
                    },
                    {
                        "kind": "tool_call_completed",
                        "tool_call": {
                            "id": "workspace-tool-1",
                            "kind": "command_execution",
                            "status": "completed",
                            "title": "Run command",
                            "command": "pwd",
                            "output": "workspace output",
                        },
                    },
                    {
                        "kind": "token_usage_updated",
                        "usage": {"inputTokens": 3, "outputTokens": 4, "totalTokens": 7},
                    },
                    {
                        "kind": "content_completed",
                        "channel": "assistant",
                        "content_delta": "Workspace answer.",
                        "phase": "final_answer",
                    },
                ],
                "final_assistant_text": "Workspace answer.",
                "token_usage": {
                    "total": {
                        "inputTokens": 3,
                        "cachedInputTokens": 0,
                        "outputTokens": 4,
                        "totalTokens": 7,
                    }
                },
                "raw_log_lines": [{"direction": "incoming", "line": '{"workspace":true}'}],
            }
        ]
    )
    service = project_chat.ProjectChatService(project_path / "spark-home")

    class BoundarySession(UnifiedAgentChatSession):
        def __init__(self, working_dir, **kwargs):
            super().__init__(working_dir, **kwargs, boundary=boundary)

    original = project_chat.UnifiedAgentChatSession
    project_chat.UnifiedAgentChatSession = BoundarySession
    try:
        snapshot = service.send_turn(
            "conversation-m6-workspace",
            str(project_path / "project"),
            "Persist this through the public service.",
            model="profile-model",
            provider="codex",
            llm_profile="team-profile",
            reasoning_effort="HIGH",
        )
    finally:
        project_chat.UnifiedAgentChatSession = original

    return {
        "conversation_id": snapshot["conversation_id"],
        "provider": snapshot["provider"],
        "model": snapshot["model"],
        "llm_profile": snapshot["llm_profile"],
        "reasoning_effort": snapshot["reasoning_effort"],
        "assistant_turn": {
            "role": snapshot["turns"][-1]["role"],
            "status": snapshot["turns"][-1]["status"],
            "content": snapshot["turns"][-1]["content"],
            "token_usage": snapshot["turns"][-1]["token_usage"],
        },
        "segments": [
            {
                "kind": segment["kind"],
                "status": segment["status"],
                "content": segment.get("content"),
                "tool_call": segment.get("tool_call"),
            }
            for segment in snapshot["segments"]
        ],
        "raw_log_directions": [
            json.loads(line)["direction"]
            for line in (
                ensure_project_paths(project_path / "spark-home", str(project_path / "project")).conversations_dir
                / "conversation-m6-workspace"
                / "raw-log.jsonl"
            ).read_text(encoding="utf-8").splitlines()
        ],
    }


def _m6_codergen_observation(project_path):
    boundary = FakeRustBoundary(
        codergen_outputs=[
            {
                "events": [
                    {
                        "kind": "content_delta",
                        "channel": "reasoning",
                        "content_delta": "codergen thinking",
                    }
                ],
                "response": {
                    "kind": "outcome",
                    "value": {
                        "status": "success",
                        "preferred_label": "done",
                        "context_updates": {"context.allowed": "done"},
                        "notes": "codergen completed",
                    },
                },
            }
        ]
    )
    backend = ProviderRouterBackend(
        str(project_path),
        lambda event: None,
        model="fallback-model",
        boundary=boundary,
    )
    graph = parse_dot(
        r'''
        digraph G {
          task [
            shape=box,
            prompt="Use runtime state",
            codergen.response_contract="status_envelope",
            spark.writes_context="[\"context.allowed\"]",
            llm_provider="openai-compatible",
            llm_model="profile-model",
            llm_profile="team-profile",
            reasoning_effort="HIGH"
          ];
        }
        '''
    )
    runner = HandlerRunner(graph, build_default_registry(codergen_backend=backend))
    outcome = runner("task", "", Context(values={"context.request": "ship"}))
    request = boundary.codergen_requests[0]
    return {
        "outcome": harness.outcome_payload(outcome),
        "request": {
            "provider": request["provider"],
            "model": request["model"],
            "llm_profile": request["llm_profile"],
            "reasoning_effort": request["reasoning_effort"],
            "response_contract": request["response_contract"],
            "write_contract": request["write_contract"],
        },
    }


def _m6_failure_observation(project_path):
    recoverable_boundary = FakeRustBoundary(
        codergen_outputs=[
            RustBoundaryError(
                "recoverable tool failure",
                retryable=True,
                raw={"kind": "tool_error"},
            )
        ]
    )
    recoverable = UnifiedAgentBackend(
        str(project_path / "recoverable"),
        lambda event: None,
        provider="openrouter",
        boundary=recoverable_boundary,
    ).run("plan", "Prompt", Context(), provider="openrouter", model="model-x")

    auth_session = UnifiedAgentChatSession(
        str(project_path / "auth"),
        provider="codex",
        model="profile-model",
        boundary=FakeRustBoundary(
            agent_outputs=[
                {
                    "error": {
                        "kind": "auth_error",
                        "message": "auth failed",
                        "retryable": False,
                    }
                }
            ]
        ),
    )
    try:
        auth_session.turn("hello", None)
    except RustBoundaryError as exc:
        auth_failure = {
            "message": str(exc),
            "retryable": exc.retryable,
            "kind": exc.raw.get("kind"),
        }
    else:
        raise AssertionError("auth failure did not surface as RustBoundaryError")

    return {
        "recoverable_codergen": harness.outcome_payload(recoverable),
        "unrecoverable_chat": auth_failure,
    }


def test_chat_session_builds_boundary_payload_and_maps_turn_output(tmp_path):
    requested = threading.Event()
    events = []
    raw_logs = []
    boundary = FakeRustBoundary(
        agent_outputs=[
            {
                "events": [
                    {
                        "kind": "content_delta",
                        "channel": "reasoning",
                        "content_delta": "checking",
                        "source": {"backend": "rust", "raw_kind": "assistant_reasoning_delta"},
                    },
                    {
                        "kind": "tool_call_updated",
                        "tool_call": {
                            "id": "tool-1",
                            "kind": "command_execution",
                            "status": "completed",
                            "title": "Run command",
                            "command": "pytest -q",
                        },
                    },
                    {
                        "kind": "request_user_input_requested",
                        "request_user_input": {
                            "request_id": "req-1",
                            "status": "pending",
                            "questions": [
                                {
                                    "id": "q1",
                                    "header": "Choice",
                                    "question": "Continue?",
                                    "question_type": "MULTIPLE_CHOICE",
                                    "options": [{"label": "Yes", "description": "Proceed"}],
                                }
                            ],
                        },
                    },
                ],
                "final_assistant_text": "Done",
                "token_usage": {
                    "total": {
                        "inputTokens": 7,
                        "cachedInputTokens": 2,
                        "outputTokens": 5,
                        "totalTokens": 12,
                    }
                },
                "raw_log_lines": [{"direction": "incoming", "line": '{"event":"ok"}'}],
            }
        ]
    )
    session = UnifiedAgentChatSession(
        str(tmp_path),
        provider="openai-compatible",
        model="model-a",
        llm_profile="team-profile",
        config_dir=tmp_path / "config",
        conversation_id="conv-123",
        metadata={"source": "test"},
        persisted_history=[
            {"role": "system", "content": "ignore", "timestamp": "2026-01-01T00:00:00Z"},
            {
                "role": "user",
                "content": "Earlier",
                "timestamp": "2026-01-01T00:00:01Z",
                "status": "complete",
                "kind": "message",
            },
            {
                "role": "assistant",
                "content": "Draft",
                "timestamp": "2026-01-01T00:00:02Z",
                "status": "in_progress",
                "kind": "message",
            },
            {
                "role": "assistant",
                "content": "Answer",
                "timestamp": "2026-01-01T00:00:03Z",
                "status": "complete",
                "kind": "message",
            },
        ],
        boundary=boundary,
    )
    session.set_raw_rpc_logger(lambda direction, line: raw_logs.append((direction, line)))

    result_holder = {}

    def on_event(event):
        events.append(event)
        if event.kind == "request_user_input_requested":
            requested.set()

    thread = threading.Thread(
        target=lambda: result_holder.setdefault(
            "result",
            session.turn("Please continue", "model-b", chat_mode="plan", reasoning_effort="HIGH", on_event=on_event),
        )
    )
    thread.start()
    assert requested.wait(2)
    assert session.has_pending_request_user_input("q1")
    assert session.submit_request_user_input_answers("q1", {"q1": "Yes"})
    thread.join(2)

    assert not thread.is_alive()
    result = result_holder["result"]
    assert result.assistant_message == "Done"
    assert result.token_usage["total"]["totalTokens"] == 12
    assert raw_logs == [("incoming", '{"event":"ok"}')]
    assert [event.kind for event in events] == [
        "content_delta",
        "tool_call_updated",
        "request_user_input_requested",
    ]
    assert events[1].tool_call.command == "pytest -q"
    assert events[2].request_user_input.request_id == "req-1"

    request = boundary.agent_requests[0]
    assert request["conversation_id"] == "conv-123"
    assert request["project_path"] == normalize_project_path_value(str(tmp_path))
    assert request["provider"] == "openai_compatible"
    assert request["model"] == "model-b"
    assert request["llm_profile"] == "team-profile"
    assert request["reasoning_effort"] == "high"
    assert request["chat_mode"] == "plan"
    assert request["metadata"]["source"] == "test"
    assert request["metadata"]["spark.config_dir"] == str(tmp_path / "config")
    assert request["history"] == [
        {"role": "user", "content": "Earlier", "timestamp": "2026-01-01T00:00:01Z"},
        {"role": "assistant", "content": "Answer", "timestamp": "2026-01-01T00:00:03Z"},
    ]


def test_chat_session_normalizes_boundary_event_variants_and_errors(tmp_path):
    boundary = FakeRustBoundary(
        agent_outputs=[
            {
                "events": [
                    {
                        "kind": "content_delta",
                        "rawKind": "assistant_delta",
                        "responseId": "resp-1",
                        "delta": "hel",
                    },
                    {
                        "kind": "content_completed",
                        "rawKind": "assistant_message_completed",
                        "text": "hello",
                    },
                    {
                        "kind": "tool_call_started",
                        "id": "call-1",
                        "name": "shell",
                        "command": "echo hi",
                    },
                    {
                        "kind": "tool_call_failed",
                        "toolCall": {
                            "callId": "call-1",
                            "name": "shell",
                            "status": "failed",
                            "output": "boom",
                        },
                    },
                    {
                        "kind": "token_usage_updated",
                        "usage": {"inputTokens": 1, "outputTokens": 2, "totalTokens": 3},
                    },
                    {"kind": "warning", "message": "heads up"},
                ]
            },
            {
                "events": [
                    {
                        "kind": "error",
                        "error": {"message": "boundary exploded", "code": "boom"},
                    }
                ]
            },
        ]
    )
    session = UnifiedAgentChatSession(str(tmp_path), provider="gemini", model="gemini-test", boundary=boundary)
    events = []

    result = session.turn("hello", None, on_event=events.append)

    assert result.assistant_message == "hello"
    assert result.token_usage["total"]["totalTokens"] == 3
    assert [event.kind for event in events] == [
        "content_delta",
        "content_completed",
        "tool_call_started",
        "tool_call_failed",
        "token_usage_updated",
        "warning",
    ]
    assert events[0].channel == "assistant"
    assert events[0].source.raw_kind == "assistant_delta"
    assert events[0].source.response_id == "resp-1"
    assert events[2].tool_call.kind == "command_execution"
    assert events[2].tool_call.status == "running"
    assert events[3].tool_call.id == "call-1"
    assert events[3].tool_call.status == "failed"

    error_events = []
    with pytest.raises(RustBoundaryError, match="boundary exploded"):
        session.turn("again", None, on_event=error_events.append)

    assert len(error_events) == 1
    assert error_events[0].kind == "error"
    assert error_events[0].error == "boundary exploded"


def test_codergen_provider_router_builds_boundary_payload_and_maps_output(tmp_path):
    emitted = []
    progress_events = []
    usage_snapshots = []
    boundary = FakeRustBoundary(
        codergen_outputs=[
            {
                "events": [
                    {
                        "kind": "content_delta",
                        "channel": "reasoning",
                        "content_delta": "thinking",
                        "source": {"backend": "rust", "raw_kind": "assistant_reasoning_delta"},
                    },
                    {
                        "kind": "token_usage_updated",
                        "token_usage": {
                            "total": {
                                "inputTokens": 3,
                                "cachedInputTokens": 1,
                                "outputTokens": 4,
                                "totalTokens": 7,
                            }
                        },
                    },
                    {
                        "kind": "token_usage_updated",
                        "token_usage": {
                            "total": {
                                "inputTokens": 5,
                                "cachedInputTokens": 1,
                                "outputTokens": 6,
                                "totalTokens": 11,
                            }
                        },
                    },
                    {"event_type": "tool", "payload": {"message": "tool completed"}},
                ],
                "response": {
                    "kind": "outcome",
                    "value": {
                        "status": "success",
                        "preferred_label": "done",
                        "context_updates": {"context.allowed": "value"},
                        "notes": "finished",
                    },
                },
                "raw_log_lines": [{"direction": "outgoing", "line": '{"request":true}'}],
            }
        ]
    )
    backend = ProviderRouterBackend(
        str(tmp_path),
        emitted.append,
        model="fallback-model",
        config_dir=tmp_path / "config",
        boundary=boundary,
        on_usage_update=usage_snapshots.append,
    )
    context = Context(values={"existing": "value"})

    with backend.bind_stage_raw_rpc_log("plan", tmp_path / "logs"):
        result = backend.run(
            "plan",
            "Prompt",
            context,
            provider="openai-compatible",
            model="model-x",
            llm_profile="team-profile",
            reasoning_effort="MEDIUM",
            response_contract="status_envelope",
            contract_repair_attempts=2,
            timeout=9.5,
            write_contract=ContextWriteContract(("context.allowed",), ""),
            emit_event=lambda *args, **kwargs: progress_events.append((args, kwargs)),
        )

    assert isinstance(result, Outcome)
    assert result.status == OutcomeStatus.SUCCESS
    assert result.preferred_label == "done"
    assert result.context_updates == {"context.allowed": "value"}
    assert progress_events[0][0] == ("LLMContent",)
    assert progress_events[0][1]["channel"] == "reasoning"
    assert usage_snapshots[-1].by_model["model-x"].total_tokens == 11
    assert emitted == [{"type": "log", "msg": "[plan] tool completed"}]
    raw_log_path = tmp_path / "logs" / "plan" / "raw-rpc.jsonl"
    assert json.loads(raw_log_path.read_text(encoding="utf-8").strip())["line"] == '{"request":true}'

    request = boundary.codergen_requests[0]
    assert request["node_id"] == "plan"
    assert request["prompt"] == "Prompt"
    assert request["context"] == {"existing": "value"}
    assert request["response_contract"] == "status_envelope"
    assert request["contract_repair_attempts"] == 2
    assert request["timeout_seconds"] == 9.5
    assert request["write_contract"] == {
        "allowed_keys": ["context.allowed"],
        "parse_error": "",
    }
    assert request["provider"] == "openai_compatible"
    assert request["model"] == "model-x"
    assert request["llm_profile"] == "team-profile"
    assert request["reasoning_effort"] == "medium"
    assert request["project_path"] == normalize_project_path_value(str(tmp_path))
    assert request["metadata"]["spark.config_dir"] == str(tmp_path / "config")


def test_codergen_provider_router_delivers_active_intervention_to_boundary(tmp_path):
    backend_box = {}
    intervention_results = []

    def request_intervention():
        intervention_results.append(
            backend_box["backend"].request_child_intervention(
                ChildInterventionRequest(
                    child_run_id="child-1",
                    message="Please keep the current change bounded.",
                    parent_run_id="parent-1",
                    parent_node_id="manager",
                    root_run_id="parent-1",
                    reason="scope check",
                    source="manager_loop",
                    cycle=3,
                    target_node_id="plan",
                )
            )
        )

    boundary = FakeRustBoundary(
        codergen_outputs=[{"response": {"kind": "text", "value": "done"}}],
        steer_outputs=[
            {
                "status": "delivered",
                "delivery_mode": "rust_boundary_codergen_turn",
                "message": "queued",
            }
        ],
        before_codergen_output=request_intervention,
    )
    backend = ProviderRouterBackend(str(tmp_path), lambda event: None, boundary=boundary)
    backend_box["backend"] = backend

    assert backend.run("plan", "Prompt", Context(), provider="openai", model="model-x") == "done"

    assert intervention_results[0].status == "delivered"
    assert intervention_results[0].delivery_mode == "rust_boundary_codergen_turn"
    assert intervention_results[0].reason == "scope check"
    request = boundary.codergen_requests[0]
    steer_request = boundary.steer_requests[0]
    assert steer_request["turn_id"] == request["turn_id"]
    assert steer_request["node_id"] == "plan"
    assert steer_request["message"] == "Please keep the current change bounded."
    assert steer_request["child_run_id"] == "child-1"
    assert steer_request["parent_node_id"] == "manager"
    assert steer_request["target_node_id"] == "plan"
    assert steer_request["provider"] == "openai"
    assert steer_request["model"] == "model-x"


def test_codergen_public_api_times_out_blocked_boundary(tmp_path):
    release = threading.Event()

    class BlockingBoundary(FakeRustBoundary):
        def run_codergen(self, payload):
            self.codergen_requests.append(payload)
            release.wait(2)
            return {"response": {"kind": "text", "value": "late"}}

    boundary = BlockingBoundary()
    backend = ProviderRouterBackend(str(tmp_path), lambda event: None, boundary=boundary)

    try:
        result = backend.run("plan", "Prompt", Context(), provider="openai", model="model-x", timeout=0.01)
    finally:
        release.set()

    assert isinstance(result, Outcome)
    assert result.status == OutcomeStatus.FAIL
    assert result.failure_kind == FailureKind.RUNTIME
    assert "timed out after 0.01s" in result.failure_reason
    assert boundary.codergen_requests[0]["timeout_seconds"] == 0.01


def test_serialized_boundary_times_out_blocked_codergen_process(tmp_path):
    script = tmp_path / "blocked_boundary.py"
    script.write_text("import time\ntime.sleep(2)\n", encoding="utf-8")
    boundary = SerializedRustAgentBoundary(command=f"{sys.executable} {script}")

    with pytest.raises(RustBoundaryError, match="timed out after 0.01s"):
        boundary.run_codergen({"timeout_seconds": 0.01})


def test_serialized_rust_boundary_rejects_local_codergen_steer_without_active_turn(monkeypatch):
    _committed_boundary_command_or_skip(monkeypatch)
    boundary = SerializedRustAgentBoundary()

    output = boundary.steer_codergen_turn(
        {
            "turn_id": "turn-1",
            "node_id": "plan",
            "message": "Please adjust course.",
        }
    )

    assert output["status"] == "rejected"
    assert output["delivery_mode"] == "rust_boundary"
    assert output["reason"] == "backend_steering_unsupported"
    assert output["turn_id"] == "turn-1"


def test_provider_router_uses_committed_serialized_boundary_for_profiled_codergen(tmp_path, monkeypatch):
    _committed_boundary_command_or_skip(monkeypatch)
    response_text = _successful_status_envelope("done")
    with _openai_compatible_server(
        [
            {"payload": _chat_completion_response(response_text)},
            {"payload": _chat_completion_response(response_text)},
        ]
    ) as (base_url, records):
        config_dir = tmp_path / "config"
        _write_test_profile(config_dir, base_url)
        boundary = SerializedRustAgentBoundary()
        direct_output = boundary.run_codergen(
            {
                "node_id": "plan",
                "prompt": "Prompt through serialized boundary",
                "context": {"context.request": "preserve fields"},
                "response_contract": "status_envelope",
                "contract_repair_attempts": 2,
                "timeout_seconds": 120.0,
                "write_contract": {"allowed_keys": ["context.allowed"], "parse_error": ""},
                "provider": "codex",
                "model": "profile-model",
                "llm_profile": "team-profile",
                "reasoning_effort": "HIGH",
                "metadata": {"spark.config_dir": str(config_dir)},
            }
        )
        completion_event = next(
            event
            for event in direct_output["events"]
            if event["event_type"] == "rust_llm_adapter_request_completed"
        )
        payload = completion_event["payload"]
        assert payload["node_id"] == "plan"
        assert payload["provider_selector"] == "codex"
        assert payload["provider"] == "openai_compatible"
        assert payload["model"] == "profile-model"
        assert payload["llm_profile"] == "team-profile"
        assert payload["reasoning_effort"] == "high"
        assert payload["response_contract"] == "status_envelope"
        assert payload["contract_repair_attempts"] == 2
        assert payload["timeout_seconds"] == 120.0
        assert payload["write_contract"] == {"allowed_keys": ["context.allowed"], "parse_error": ""}

        backend = ProviderRouterBackend(
            str(tmp_path),
            lambda event: None,
            config_dir=config_dir,
            boundary=boundary,
        )
        result = backend.run(
            "plan",
            "Prompt through provider router",
            Context(values={"context.request": "preserve fields"}),
            provider="codex",
            model="profile-model",
            llm_profile="team-profile",
            reasoning_effort="HIGH",
            response_contract="status_envelope",
            contract_repair_attempts=2,
            timeout=120.0,
            write_contract=ContextWriteContract(("context.allowed",), ""),
        )

    assert isinstance(result, Outcome)
    assert result.status == OutcomeStatus.SUCCESS
    assert result.context_updates == {"context.allowed": "done"}
    assert [record["path"] for record in records] == ["/v1/chat/completions", "/v1/chat/completions"]
    first_metadata = records[0]["body"]["metadata"]
    assert records[0]["body"]["model"] == "profile-model"
    assert records[0]["body"]["messages"][0]["content"] == "Prompt through serialized boundary"
    assert first_metadata["spark.runtime.source"] == "codergen"
    assert first_metadata["spark.runtime.node_id"] == "plan"
    assert first_metadata["spark.runtime.response_contract"] == "status_envelope"
    assert first_metadata["spark.runtime.provider"] == "openai_compatible"
    assert first_metadata["spark.runtime.model"] == "profile-model"
    assert first_metadata["spark.runtime.llm_profile"] == "team-profile"
    assert first_metadata["spark.runtime.reasoning_effort"] == "high"
    second_metadata = records[1]["body"]["metadata"]
    assert records[1]["body"]["messages"][0]["content"] == "Prompt through provider router"
    assert second_metadata["spark.runtime.node_id"] == "plan"
    assert second_metadata["spark.runtime.response_contract"] == "status_envelope"
    assert second_metadata["spark.runtime.llm_profile"] == "team-profile"


def test_provider_router_maps_committed_serialized_boundary_errors_to_runtime_outcomes(tmp_path, monkeypatch):
    _committed_boundary_command_or_skip(monkeypatch)
    with _openai_compatible_server(
        [
            {
                "status": 500,
                "payload": {"error": {"message": "provider exploded"}},
            }
        ]
    ) as (base_url, _records):
        config_dir = tmp_path / "config"
        _write_test_profile(config_dir, base_url)
        backend = ProviderRouterBackend(
            str(tmp_path),
            lambda event: None,
            config_dir=config_dir,
            boundary=SerializedRustAgentBoundary(),
        )
        result = backend.run(
            "plan",
            "Prompt",
            Context(),
            provider="codex",
            model="profile-model",
            llm_profile="team-profile",
            timeout=120.0,
        )

    assert isinstance(result, Outcome)
    assert result.status == OutcomeStatus.FAIL
    assert result.failure_kind == FailureKind.RUNTIME
    assert result.retryable is False
    assert "provider_http_error" in result.failure_reason or "provider exploded" in result.failure_reason


def test_codergen_handler_public_api_routes_profiled_node_through_boundary(tmp_path):
    emitted = []
    progress_events = []
    boundary = FakeRustBoundary(
        codergen_outputs=[
            {
                "events": [
                    {
                        "kind": "content_delta",
                        "channel": "reasoning",
                        "content_delta": "checking handler route",
                    }
                ],
                "response": {
                    "kind": "text",
                    "value": json.dumps(
                        {
                            "outcome": "success",
                            "preferred_label": "done",
                            "suggested_next_ids": [],
                            "context_updates": {"context.allowed": "done"},
                            "notes": "handler completed",
                            "failure_reason": "",
                        }
                    ),
                },
                "raw_log_lines": [{"direction": "incoming", "line": '{"handler":true}'}],
            }
        ]
    )
    backend = ProviderRouterBackend(
        str(tmp_path),
        emitted.append,
        model="fallback-model",
        config_dir=tmp_path / "config",
        boundary=boundary,
    )
    graph = parse_dot(
        r'''
        digraph G {
          graph [goal="Ship boundary routing"];
          task [
            shape=box,
            prompt="Plan $goal",
            codergen.response_contract="status_envelope",
            codergen.contract_repair_attempts=1,
            spark.reads_context="[\"context.request\"]",
            spark.writes_context="[\"context.allowed\"]",
            llm_provider="codex",
            llm_profile="team-profile",
            llm_model="profile-model",
            reasoning_effort="HIGH"
          ];
        }
        '''
    )
    logs_root = tmp_path / "logs"
    runner = HandlerRunner(
        graph,
        build_default_registry(codergen_backend=backend),
        logs_root=logs_root,
    )

    outcome = runner(
        "task",
        "",
        Context(values={"context.request": "Please preserve public behavior."}),
        emit_event=lambda *args, **kwargs: progress_events.append((args, kwargs)),
    )

    assert outcome.status == OutcomeStatus.SUCCESS
    assert outcome.preferred_label == "done"
    assert outcome.context_updates["context.allowed"] == "done"
    assert progress_events[0][0] == ("LLMContent",)
    assert progress_events[0][1]["channel"] == "reasoning"
    assert (logs_root / "task" / "response.md").exists()
    raw_log_path = logs_root / "task" / "raw-rpc.jsonl"
    assert json.loads(raw_log_path.read_text(encoding="utf-8").strip())["line"] == '{"handler":true}'

    request = boundary.codergen_requests[0]
    assert request["node_id"] == "task"
    assert request["provider"] == "codex"
    assert request["llm_profile"] == "team-profile"
    assert request["model"] == "profile-model"
    assert request["reasoning_effort"] == "high"
    assert request["response_contract"] == "status_envelope"
    assert request["contract_repair_attempts"] == 1
    assert request["context"] == {"context.request": "Please preserve public behavior."}
    assert request["write_contract"] == {
        "allowed_keys": ["context.allowed"],
        "parse_error": "",
    }
    assert request["metadata"]["spark.config_dir"] == str(tmp_path / "config")


def test_codergen_boundary_repairs_contract_violations(tmp_path):
    boundary = FakeRustBoundary(
        codergen_outputs=[
            {"response": {"kind": "text", "value": "not json"}},
            {
                "response": {
                    "kind": "text",
                    "value": json.dumps(
                        {
                            "outcome": "success",
                            "preferred_label": "fixed",
                            "suggested_next_ids": [],
                            "context_updates": {},
                            "notes": "repaired",
                            "failure_reason": "",
                        }
                    ),
                }
            },
        ]
    )
    backend = UnifiedAgentBackend(str(tmp_path), lambda event: None, provider="openrouter", boundary=boundary)

    result = backend.run(
        "plan",
        "Prompt",
        Context(),
        provider="openrouter",
        model="model-x",
        response_contract="status_envelope",
        contract_repair_attempts=1,
    )

    assert isinstance(result, Outcome)
    assert result.status == OutcomeStatus.SUCCESS
    assert result.preferred_label == "fixed"
    assert len(boundary.codergen_requests) == 2
    assert boundary.codergen_requests[0]["prompt"] == "Prompt"
    assert boundary.codergen_requests[1]["repair_attempt"] == 1
    assert boundary.codergen_requests[1]["prompt"] != boundary.codergen_requests[0]["prompt"]


def test_codergen_boundary_errors_return_runtime_outcomes(tmp_path):
    boundary = FakeRustBoundary(
        codergen_outputs=[
            RustBoundaryError(
                "temporary Rust tool failure",
                retryable=True,
                raw={"kind": "tool_error"},
            )
        ]
    )
    backend = UnifiedAgentBackend(str(tmp_path), lambda event: None, provider="openrouter", boundary=boundary)

    result = backend.run("plan", "Prompt", Context(), provider="openrouter", model="model-x")

    assert isinstance(result, Outcome)
    assert result.status == OutcomeStatus.FAIL
    assert result.failure_reason == "temporary Rust tool failure"
    assert result.retryable is True
