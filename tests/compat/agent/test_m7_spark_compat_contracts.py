from __future__ import annotations

from contextlib import contextmanager
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import subprocess
import threading
import time
from typing import Any, Mapping

import httpx

import spark.chat.service as project_chat
from attractor.api.codex_backends import ProviderRouterBackend
from attractor.dsl import parse_dot
from attractor.engine.context import Context
from attractor.engine.context_contracts import ContextWriteContract
from attractor.engine.outcome import Outcome
from attractor.handlers import HandlerRunner, build_default_registry
from spark.chat.session import (
    PersistedThreadContinuityResetError,
    UnifiedAgentChatSession,
)
from spark.workspace.conversations.utils import normalize_project_path_value
from spark.workspace.storage import ensure_project_paths
from tests.compat import harness


ITEM_ID = "M7-I3"
ITEM_REQUIREMENTS = ("REQ-001", "REQ-016", "REQ-017")
ITEM_DECISIONS = (
    "CD-CODING-AGENT-RUST-001",
    "CD-CODING-AGENT-RUST-011",
    "CD-CODING-AGENT-RUST-013",
)


class FakeRustBoundary:
    def __init__(self, *, agent_outputs=None, codergen_outputs=None):
        self.agent_requests: list[dict[str, Any]] = []
        self.codergen_requests: list[dict[str, Any]] = []
        self._agent_outputs = list(agent_outputs or [])
        self._codergen_outputs = list(codergen_outputs or [])

    def run_agent_turn(self, payload):
        self.agent_requests.append(dict(payload))
        output = self._agent_outputs.pop(0)
        if isinstance(output, BaseException):
            raise output
        return output

    def run_codergen(self, payload):
        self.codergen_requests.append(dict(payload))
        output = self._codergen_outputs.pop(0)
        if isinstance(output, BaseException):
            raise output
        return output

    def steer_codergen_turn(self, payload):
        return {
            "status": "rejected",
            "delivery_mode": "rust_boundary",
            "reason": "backend_steering_unsupported",
            "turn_id": payload.get("turn_id"),
        }


def test_m7_spark_compatibility_contract_fixture_matches_observed_boundaries(
    compat_fixture_root: Path,
    compat_update_goldens: bool,
    tmp_path: Path,
    rust_spark_server_binary: Path,
) -> None:
    manifest = harness.normalize_path_tokens(
        _compat_manifest(tmp_path, rust_spark_server_binary),
        {"__TMP__": tmp_path},
    )
    _assert_observable_contract(manifest["observation"])
    _assert_fixture(
        manifest,
        compat_fixture_root / f"{manifest['fixture_id']}.json",
        compat_update_goldens,
    )


def test_m7_rust_workspace_and_http_event_contracts_are_active(
    rewrite_worktree_path: Path,
) -> None:
    commands = [
        [
            "cargo",
            "test",
            "-p",
            "spark-workspace",
            "--test",
            "conversation_event_normalization_contracts",
            "normalized_agent_events_update_segments_raw_logs_usage_and_resume_failures",
        ],
        [
            "cargo",
            "test",
            "-p",
            "spark-http",
            "--test",
            "workspace_conversation_turn_route_contracts",
            "conversation_turn_route_executes_injected_backend_and_preserves_validation",
        ],
        [
            "cargo",
            "test",
            "-p",
            "spark-http",
            "--test",
            "workspace_live_sse_contracts",
            "live_route_streams_full_backend_ingested_revision_range_for_turn_route",
        ],
    ]
    for command in commands:
        completed = subprocess.run(
            command,
            cwd=rewrite_worktree_path,
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            check=False,
            timeout=180,
        )
        assert completed.returncode == 0, {
            "command": command,
            "stdout": completed.stdout,
            "stderr": completed.stderr,
        }


def _compat_manifest(tmp_path: Path, rust_spark_server_binary: Path) -> dict[str, Any]:
    observation = {
        "python_facade": _python_facade_observation(tmp_path / "facade"),
        "workspace_service": _workspace_service_observation(tmp_path / "workspace"),
        "codergen_adapter": _codergen_observation(tmp_path / "codergen"),
        "thread_resume_failure": _thread_resume_failure_observation(tmp_path / "resume-failure"),
        "rust_http_route": _rust_http_route_observation(tmp_path / "rust-http", rust_spark_server_binary),
    }
    return {
        "schema_version": "compat-agent-v1",
        "fixture_id": "agent/m7-spark-compatibility-contracts",
        "item_id": ITEM_ID,
        "requirements": list(ITEM_REQUIREMENTS),
        "decisions": list(ITEM_DECISIONS),
        "provenance": {
            "oracle": "public-spark-python-facades-and-rust-http-route",
            "interfaces": [
                "spark.chat.session.UnifiedAgentChatSession",
                "spark.chat.service.ProjectChatService",
                "attractor.api.codex_backends.ProviderRouterBackend",
                "attractor.handlers.HandlerRunner",
                "spark-agent-adapter.AgentTurnBackend",
                "spark-http workspace conversation turn route",
            ],
        },
        "scenario": "m7_spark_payload_contracts_backed_by_rust_boundary",
        "input": {
            "providers": ["codex", "openai-compatible"],
            "profile": "team-profile",
            "chat_mode": "plan",
            "response_contract": "status_envelope",
        },
        "observation": observation,
    }


def _python_facade_observation(project_path: Path) -> dict[str, Any]:
    requested = threading.Event()
    raw_logs: list[tuple[str, str]] = []
    events = []
    boundary = FakeRustBoundary(
        agent_outputs=[
            {
                "events": [
                    {"kind": "session_start", "source": {"backend": "rust", "raw_kind": "SESSION_START"}},
                    {
                        "kind": "content_delta",
                        "channel": "reasoning",
                        "content_delta": "checking compatibility",
                        "source": {"backend": "rust", "raw_kind": "assistant_reasoning_delta"},
                    },
                    _model_tool_event("model_tool_call_start", "facade-model-tool", "proposed"),
                    _model_tool_event("model_tool_call_delta", "facade-model-tool", "streaming"),
                    _model_tool_event("model_tool_call_end", "facade-model-tool", "completed"),
                    {
                        "kind": "tool_call_started",
                        "tool_call": {
                            "id": "facade-exec-tool",
                            "kind": "command_execution",
                            "status": "running",
                            "title": "Run command",
                            "command": "printf facade",
                        },
                    },
                    {
                        "kind": "tool_call_completed",
                        "tool_call": {
                            "id": "facade-exec-tool",
                            "kind": "command_execution",
                            "status": "completed",
                            "title": "Run command",
                            "command": "printf facade",
                            "output": "facade",
                        },
                    },
                    {
                        "kind": "token_usage_updated",
                        "token_usage": {
                            "total": {
                                "inputTokens": 13,
                                "cachedInputTokens": 2,
                                "outputTokens": 8,
                                "totalTokens": 21,
                            }
                        },
                    },
                    {"kind": "warning", "message": "compat warning"},
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
                        "content_delta": "Facade compatibility answer.",
                        "phase": "final_answer",
                    },
                    {"kind": "processing_end", "status": "idle"},
                    {"kind": "session_end", "status": "closed"},
                ],
                "final_assistant_text": "Facade compatibility answer.",
                "token_usage": {
                    "total": {
                        "inputTokens": 13,
                        "cachedInputTokens": 2,
                        "outputTokens": 8,
                        "totalTokens": 21,
                    }
                },
                "raw_log_lines": [{"direction": "incoming", "line": '{"facade":true}'}],
            }
        ]
    )
    session = UnifiedAgentChatSession(
        str(project_path),
        provider="openai-compatible",
        model="initial-model",
        llm_profile="team-profile",
        config_dir=project_path / "config",
        conversation_id="conversation-m7-facade",
        metadata={"source": "m7-compat"},
        persisted_history=[
            {
                "role": "user",
                "content": "Earlier complete request",
                "timestamp": "2026-06-01T00:00:00Z",
                "status": "complete",
                "kind": "message",
            },
            {
                "role": "assistant",
                "content": "Earlier complete answer",
                "timestamp": "2026-06-01T00:00:01Z",
                "status": "complete",
                "kind": "message",
            },
            {
                "role": "assistant",
                "content": "Incomplete answer",
                "timestamp": "2026-06-01T00:00:02Z",
                "status": "in_progress",
                "kind": "message",
            },
        ],
        boundary=boundary,
    )
    session.set_raw_rpc_logger(lambda direction, line: raw_logs.append((direction, line)))
    result_holder: dict[str, Any] = {}

    def on_event(event):
        events.append(event)
        if event.kind == "request_user_input_requested":
            requested.set()

    worker = threading.Thread(
        target=lambda: result_holder.setdefault(
            "result",
            session.turn(
                "Continue through facade.",
                "profile-model",
                chat_mode="PLAN",
                reasoning_effort="HIGH",
                on_event=on_event,
            ),
        )
    )
    worker.start()
    assert requested.wait(2)
    assert session.has_pending_request_user_input("decision")
    assert session.submit_request_user_input_answers("decision", {"decision": "Continue"})
    worker.join(2)
    assert not worker.is_alive()
    request = boundary.agent_requests[0]
    result = result_holder["result"]
    return {
        "result": {
            "assistant_message": result.assistant_message,
            "token_total": result.token_usage["total"]["totalTokens"],
        },
        "request": _selected_request_fields(request),
        "history": request["history"],
        "event_kinds": [event.kind for event in events],
        "channels": [event.channel for event in events if event.channel],
        "model_tool_statuses": [
            event.tool_call.status
            for event in events
            if event.tool_call is not None and event.tool_call.kind == "model_tool_call"
        ],
        "execution_tool": [
            {
                "kind": event.kind,
                "status": event.tool_call.status,
                "command": event.tool_call.command,
                "output": event.tool_call.output,
            }
            for event in events
            if event.tool_call is not None and event.tool_call.kind == "command_execution"
        ],
        "request_user_input": [
            event.request_user_input.to_dict()
            for event in events
            if event.kind == "request_user_input_requested"
        ],
        "warnings": [event.message for event in events if event.kind == "warning"],
        "raw_logs": [{"direction": direction, "line": line} for direction, line in raw_logs],
    }


def _workspace_service_observation(project_path: Path) -> dict[str, Any]:
    boundary = FakeRustBoundary(
        agent_outputs=[
            {
                "events": [
                    {
                        "kind": "content_delta",
                        "channel": "reasoning",
                        "content_delta": "workspace reasoning",
                    },
                    _model_tool_event("model_tool_call_start", "workspace-model-tool", "proposed"),
                    _model_tool_event("model_tool_call_end", "workspace-model-tool", "completed"),
                    {
                        "kind": "tool_call_completed",
                        "tool_call": {
                            "id": "workspace-exec-tool",
                            "kind": "command_execution",
                            "status": "completed",
                            "title": "Run command",
                            "command": "pwd",
                            "output": "workspace-output",
                        },
                    },
                    {
                        "kind": "token_usage_updated",
                        "token_usage": {"total": {"inputTokens": 5, "outputTokens": 6, "totalTokens": 11}},
                    },
                    {
                        "kind": "content_completed",
                        "channel": "assistant",
                        "content_delta": "Workspace compatibility answer.",
                    },
                ],
                "final_assistant_text": "Workspace compatibility answer.",
                "token_usage": {"total": {"inputTokens": 5, "outputTokens": 6, "totalTokens": 11}},
                "raw_log_lines": [{"direction": "outgoing", "line": '{"workspace":true}'}],
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
            "conversation-m7-workspace",
            str(project_path / "project"),
            "Persist the compatibility payload.",
            model="profile-model",
            provider="codex",
            llm_profile="team-profile",
            reasoning_effort="MEDIUM",
            chat_mode="plan",
        )
    finally:
        project_chat.UnifiedAgentChatSession = original

    paths = ensure_project_paths(project_path / "spark-home", str(project_path / "project"))
    raw_log = (
        paths.conversations_dir / "conversation-m7-workspace" / "raw-log.jsonl"
    ).read_text(encoding="utf-8")
    return {
        "snapshot": {
            "conversation_id": snapshot["conversation_id"],
            "provider": snapshot["provider"],
            "model": snapshot["model"],
            "llm_profile": snapshot["llm_profile"],
            "reasoning_effort": snapshot["reasoning_effort"],
            "chat_mode": snapshot["chat_mode"],
            "assistant": {
                "role": snapshot["turns"][-1]["role"],
                "status": snapshot["turns"][-1]["status"],
                "content": snapshot["turns"][-1]["content"],
                "token_total": snapshot["turns"][-1]["token_usage"]["total"]["totalTokens"],
            },
            "segment_kinds": [segment["kind"] for segment in snapshot["segments"]],
            "model_tool_segments": [
                segment["tool_call"]
                for segment in snapshot["segments"]
                if segment["kind"] == "model_tool_call"
            ],
            "execution_tool_segments": [
                segment["tool_call"]
                for segment in snapshot["segments"]
                if segment["kind"] == "tool_call"
            ],
        },
        "request": _selected_request_fields(boundary.agent_requests[0]),
        "raw_log_directions": [json.loads(line)["direction"] for line in raw_log.splitlines()],
    }


def _codergen_observation(project_path: Path) -> dict[str, Any]:
    emitted: list[dict[str, Any]] = []
    progress_events: list[tuple[tuple[Any, ...], dict[str, Any]]] = []
    usage_snapshots = []
    boundary = FakeRustBoundary(
        codergen_outputs=[
            {
                "events": [
                    {
                        "kind": "content_delta",
                        "channel": "reasoning",
                        "content_delta": "codergen reasoning",
                    },
                    {
                        "kind": "token_usage_updated",
                        "token_usage": {"total": {"inputTokens": 7, "outputTokens": 3, "totalTokens": 10}},
                    },
                    {"event_type": "tool", "payload": {"message": "tool event visible"}},
                ],
                "response": {
                    "kind": "outcome",
                    "value": {
                        "status": "success",
                        "preferred_label": "done",
                        "suggested_next_ids": [],
                        "context_updates": {"context.allowed": "codergen"},
                        "notes": "codergen compatibility complete",
                        "failure_reason": "",
                    },
                },
                "raw_log_lines": [{"direction": "incoming", "line": '{"codergen":true}'}],
            }
        ]
    )
    backend = ProviderRouterBackend(
        str(project_path / "project"),
        emitted.append,
        model="fallback-model",
        config_dir=project_path / "config",
        boundary=boundary,
        on_usage_update=usage_snapshots.append,
    )
    graph = parse_dot(
        r'''
        digraph G {
          task [
            shape=box,
            prompt="Plan for $context.request",
            codergen.response_contract="status_envelope",
            codergen.contract_repair_attempts=1,
            spark.writes_context="[\"context.allowed\"]",
            llm_provider="openai-compatible",
            llm_model="profile-model",
            llm_profile="team-profile",
            reasoning_effort="HIGH"
          ];
        }
        '''
    )
    runner = HandlerRunner(
        graph,
        build_default_registry(codergen_backend=backend),
        logs_root=project_path / "logs",
    )
    outcome = runner(
        "task",
        "",
        Context(values={"context.request": "m7 compat"}),
        emit_event=lambda *args, **kwargs: progress_events.append((args, kwargs)),
    )
    assert isinstance(outcome, Outcome)
    raw_rpc = project_path / "logs" / "task" / "raw-rpc.jsonl"
    request = boundary.codergen_requests[0]
    return {
        "outcome": harness.outcome_payload(outcome),
        "request": {
            "node_id": request["node_id"],
            "context": request["context"],
            "response_contract": request["response_contract"],
            "contract_repair_attempts": request["contract_repair_attempts"],
            "write_contract": request["write_contract"],
            "provider": request["provider"],
            "model": request["model"],
            "llm_profile": request["llm_profile"],
            "reasoning_effort": request["reasoning_effort"],
            "project_path": request["project_path"],
            "metadata_keys": sorted(request["metadata"].keys()),
        },
        "progress": [
            {
                "event": args[0],
                "channel": kwargs.get("channel"),
                "content": kwargs.get("content"),
            }
            for args, kwargs in progress_events
        ],
        "usage_total": usage_snapshots[-1].total_tokens,
        "emitted": emitted,
        "raw_log_directions": [json.loads(line)["direction"] for line in raw_rpc.read_text(encoding="utf-8").splitlines()],
    }


def _thread_resume_failure_observation(project_path: Path) -> dict[str, Any]:
    session = UnifiedAgentChatSession(
        str(project_path),
        provider="codex",
        model="profile-model",
        boundary=FakeRustBoundary(
            agent_outputs=[
                {
                    "thread_resume_failure": {
                        "message": "persisted thread was unavailable",
                        "error_code": "thread_resume_failed",
                        "details": {"persisted_thread_id": "thread-old"},
                    }
                }
            ]
        ),
    )
    try:
        session.turn("resume", None)
    except PersistedThreadContinuityResetError as exc:
        return exc.to_debug_payload()
    raise AssertionError("thread resume failure did not surface")


def _rust_http_route_observation(root: Path, rust_spark_server_binary: Path) -> dict[str, Any]:
    spark_home = root / "spark-home"
    flows_dir = root / "flows"
    project_dir = root / "project"
    logs_dir = root / "logs"
    for path in (spark_home / "config", flows_dir, project_dir, logs_dir):
        path.mkdir(parents=True, exist_ok=True)

    with _openai_compatible_server(
        [
            {
                "payload": {
                    "id": "chatcmpl-m7-route",
                    "object": "chat.completion",
                    "created": 0,
                    "model": "profile-model",
                    "choices": [
                        {
                            "index": 0,
                            "message": {"role": "assistant", "content": "Rust HTTP route answer."},
                            "finish_reason": "stop",
                        }
                    ],
                    "usage": {"prompt_tokens": 9, "completion_tokens": 4, "total_tokens": 13},
                }
            }
        ]
    ) as (base_url, provider_records):
        (spark_home / "config" / "llm-profiles.toml").write_text(
            f'''
[profiles.team-profile]
label = "Team Profile"
provider = "openai_compatible"
base_url = "{base_url}"
models = ["profile-model"]
default_model = "profile-model"
'''.lstrip(),
            encoding="utf-8",
        )
        env = os.environ.copy()
        env.update(
            {
                "SPARK_HOME": str(spark_home),
                "SPARK_FLOWS_DIR": str(flows_dir),
                "CODEX_HOME": str(root / "codex-home"),
                "ATTRACTOR_CODEX_RUNTIME_ROOT": str(root / "runtime"),
            }
        )
        init = subprocess.run(
            [
                str(rust_spark_server_binary),
                "init",
                "--data-dir",
                str(spark_home),
                "--flows-dir",
                str(flows_dir),
            ],
            cwd=Path(__file__).resolve().parents[3],
            env=env,
            text=True,
            capture_output=True,
            check=False,
        )
        assert init.returncode == 0, init.stderr

        port = _free_tcp_port()
        stdout = (logs_dir / "stdout.log").open("w", encoding="utf-8")
        stderr = (logs_dir / "stderr.log").open("w", encoding="utf-8")
        process = subprocess.Popen(
            [
                str(rust_spark_server_binary),
                "serve",
                "--host",
                "127.0.0.1",
                "--port",
                str(port),
                "--data-dir",
                str(spark_home),
                "--flows-dir",
                str(flows_dir),
            ],
            cwd=Path(__file__).resolve().parents[3],
            env=env,
            text=True,
            stdout=stdout,
            stderr=stderr,
        )
        try:
            route_base_url = f"http://127.0.0.1:{port}"
            _wait_for_server(route_base_url, process)
            with httpx.Client(base_url=route_base_url, timeout=20.0) as client:
                response = client.post(
                    "/workspace/api/conversations/conversation-m7-http/turns",
                    json={
                        "project_path": str(project_dir),
                        "message": "Answer through the Rust HTTP route.",
                        "provider": "codex",
                        "model": "profile-model",
                        "llm_profile": "team-profile",
                        "reasoning_effort": "HIGH",
                        "chat_mode": "chat",
                    },
                )
            assert response.status_code == 200, response.text
            snapshot = response.json()
        finally:
            process.terminate()
            try:
                process.wait(timeout=10)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait(timeout=10)
            stdout.close()
            stderr.close()

    assistant = snapshot["turns"][-1]
    provider_request = provider_records[0]["body"]
    metadata = provider_request["metadata"]
    return {
        "http_status": response.status_code,
        "snapshot": {
            "conversation_id": snapshot["conversation_id"],
            "provider": snapshot["provider"],
            "model": snapshot["model"],
            "llm_profile": snapshot["llm_profile"],
            "reasoning_effort": snapshot["reasoning_effort"],
            "assistant": {
                "role": assistant["role"],
                "status": assistant["status"],
                "content": assistant["content"],
                "token_total": assistant["token_usage"]["total"]["totalTokens"],
            },
            "segment_kinds": [segment["kind"] for segment in snapshot["segments"]],
        },
        "provider_request": {
            "path": provider_records[0]["path"],
            "model": provider_request["model"],
            "message_count": len(provider_request["messages"]),
            "runtime_provider_selector": metadata["spark.runtime.provider_selector"],
            "runtime_provider": metadata["spark.runtime.provider"],
            "runtime_model": metadata["spark.runtime.model"],
            "runtime_llm_profile": metadata["spark.runtime.llm_profile"],
            "runtime_reasoning_effort": metadata["spark.runtime.reasoning_effort"],
        },
    }


def _assert_observable_contract(observation: Mapping[str, Any]) -> None:
    assert observation["python_facade"]["result"]["assistant_message"] == "Facade compatibility answer."
    assert observation["python_facade"]["request"]["provider"] == "openai_compatible"
    assert observation["python_facade"]["request"]["model"] == "profile-model"
    assert observation["python_facade"]["request"]["llm_profile"] == "team-profile"
    assert observation["python_facade"]["request"]["reasoning_effort"] == "high"
    assert "request_user_input_requested" in observation["python_facade"]["event_kinds"]
    assert observation["workspace_service"]["snapshot"]["assistant"]["content"] == "Workspace compatibility answer."
    assert observation["workspace_service"]["request"]["provider"] == "codex"
    assert observation["codergen_adapter"]["outcome"]["status"] == "success"
    assert observation["codergen_adapter"]["request"]["provider"] == "openai_compatible"
    assert observation["thread_resume_failure"]["event"] == "continuity_reset"
    assert observation["rust_http_route"]["http_status"] == 200
    assert observation["rust_http_route"]["provider_request"]["runtime_provider"] == "openai_compatible"
    assert observation["rust_http_route"]["snapshot"]["assistant"]["content"] == "Rust HTTP route answer."


def _selected_request_fields(request: Mapping[str, Any]) -> dict[str, Any]:
    prompt = str(request.get("prompt") or "")
    return {
        "conversation_id": request["conversation_id"],
        "project_path": request["project_path"],
        "prompt": {
            "non_empty": bool(prompt),
            "contains_latest_user_message": "compatibility payload" in prompt
            or "Continue through facade" in prompt,
        },
        "provider": request["provider"],
        "model": request["model"],
        "llm_profile": request["llm_profile"],
        "reasoning_effort": request["reasoning_effort"],
        "chat_mode": request["chat_mode"],
        "metadata": dict(request["metadata"]),
    }


def _model_tool_event(kind: str, call_id: str, status: str) -> dict[str, Any]:
    return {
        "kind": kind,
        "tool_call": {
            "id": call_id,
            "kind": "model_tool_call",
            "status": status,
            "title": "Search",
            "arguments": {"query": "m7 compatibility"},
        },
        "source": {"backend": "rust", "raw_kind": kind},
    }


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


def _free_tcp_port() -> int:
    import socket

    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as sock:
        sock.bind(("127.0.0.1", 0))
        return int(sock.getsockname()[1])


def _wait_for_server(base_url: str, process: subprocess.Popen[str]) -> None:
    deadline = time.monotonic() + 20
    last_error: Exception | None = None
    while time.monotonic() < deadline:
        if process.poll() is not None:
            raise AssertionError(f"server exited early with {process.returncode}")
        try:
            with httpx.Client(base_url=base_url, timeout=1.0) as client:
                response = client.get("/attractor/status")
            if response.status_code == 200:
                return
        except Exception as exc:  # noqa: BLE001
            last_error = exc
        time.sleep(0.1)
    raise AssertionError(f"server did not become ready: {last_error!r}")


def _assert_fixture(
    manifest: Mapping[str, Any],
    fixture_path: Path,
    update_goldens: bool,
) -> None:
    harness.validate_manifest_coverage(
        manifest,
        requirement_ids=ITEM_REQUIREMENTS,
        decision_ids=ITEM_DECISIONS,
    )
    if update_goldens:
        harness.write_manifest(fixture_path, manifest)
    expected = harness.load_manifest(fixture_path)
    harness.assert_agent_manifest_matches_golden(manifest, expected)
