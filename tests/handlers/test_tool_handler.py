from __future__ import annotations

import json
import shlex
import subprocess
import tempfile
import threading
from pathlib import Path
from typing import get_args, get_type_hints

import pytest

from attractor.dsl import parse_dot
from attractor.engine.context import Context
from attractor.engine.executor import PipelineExecutor
from attractor.engine.outcome import Outcome, OutcomeStatus
from attractor.handlers import HandlerRunner, build_default_registry
from attractor.handlers.base import CodergenBackend
from attractor.handlers.registry import SHAPE_TO_TYPE
from attractor.interviewer import Answer, CallbackInterviewer, Interviewer, Question, QueueInterviewer

from tests.handlers._support.fakes import (
    _StubBackend,
)

class TestToolHandler:
    def test_tool_handler_executes_command(self):
        graph = parse_dot(
            """
            digraph G {
                tool_node [shape=parallelogram, tool_command="printf hello"]
            }
            """
        )
        registry = build_default_registry(codergen_backend=_StubBackend())
        runner = HandlerRunner(graph, registry)
        outcome = runner("tool_node", "", Context())
        assert outcome.status == OutcomeStatus.SUCCESS
        assert "hello" in outcome.notes
        assert outcome.context_updates["context.tool.output"] == "hello"
        assert outcome.context_updates["context.tool.exit_code"] == 0

    def test_tool_handler_fails_when_command_missing(self):
        graph = parse_dot(
            """
            digraph G {
                tool_node [shape=parallelogram]
            }
            """
        )
        registry = build_default_registry(codergen_backend=_StubBackend())
        runner = HandlerRunner(graph, registry)
        outcome = runner("tool_node", "", Context())
        assert outcome.status == OutcomeStatus.FAIL
        assert outcome.failure_reason == "No tool_command specified"

    def test_tool_handler_passes_hook_metadata_via_env_and_stdin_json(self, monkeypatch):
        graph = parse_dot(
            """
            digraph G {
                graph [tool_hooks.pre="pre-hook", tool_hooks.post="post-hook"]
                tool_node [shape=parallelogram, tool_command="run-tool"]
            }
            """
        )

        run_calls = []

        def _fake_run(command, **kwargs):
            run_calls.append({"command": command, "kwargs": kwargs})
            if command == "run-tool":
                return subprocess.CompletedProcess(command, 0, stdout="hello", stderr="")
            return subprocess.CompletedProcess(command, 0, stdout="", stderr="")

        monkeypatch.setattr("attractor.handlers.builtin.tool.subprocess.run", _fake_run)

        registry = build_default_registry(codergen_backend=_StubBackend())
        runner = HandlerRunner(graph, registry)
        outcome = runner("tool_node", "", Context())

        assert outcome.status == OutcomeStatus.SUCCESS

        hook_calls = [call for call in run_calls if call["command"] in {"pre-hook", "post-hook"}]
        assert [call["command"] for call in hook_calls] == ["pre-hook", "post-hook"]

        for expected_phase, call in zip(("pre", "post"), hook_calls):
            payload = json.loads(call["kwargs"]["input"])
            env = call["kwargs"]["env"]
            assert env["ATTRACTOR_TOOL_HOOK_PHASE"] == expected_phase
            assert env["ATTRACTOR_TOOL_NODE_ID"] == "tool_node"
            assert env["ATTRACTOR_TOOL_COMMAND"] == "run-tool"
            assert payload["hook_phase"] == expected_phase
            assert payload["node_id"] == "tool_node"
            assert payload["tool_command"] == "run-tool"

    def test_tool_handler_records_nonzero_hook_exits_without_blocking_tool_execution(self, monkeypatch, tmp_path):
        graph = parse_dot(
            """
            digraph G {
                graph [tool_hooks.pre="pre-hook", tool_hooks.post="post-hook"]
                tool_node [shape=parallelogram, tool_command="run-tool"]
            }
            """
        )
        logs_root = tmp_path / "logs"

        def _fake_run(command, **kwargs):
            del kwargs
            if command == "run-tool":
                return subprocess.CompletedProcess(command, 0, stdout="tool-output", stderr="")
            if command == "pre-hook":
                return subprocess.CompletedProcess(command, 7, stdout="", stderr="pre failed")
            if command == "post-hook":
                return subprocess.CompletedProcess(command, 9, stdout="", stderr="post failed")
            raise AssertionError(f"unexpected command: {command}")

        monkeypatch.setattr("attractor.handlers.builtin.tool.subprocess.run", _fake_run)

        registry = build_default_registry(codergen_backend=_StubBackend())
        runner = HandlerRunner(graph, registry, logs_root=logs_root)
        outcome = runner("tool_node", "", Context())

        assert outcome.status == OutcomeStatus.SUCCESS
        assert outcome.context_updates["context.tool.output"] == "tool-output"
        hook_failures_path = logs_root / "tool_node" / "tool_hook_failures.jsonl"
        assert hook_failures_path.exists()
        records = [
            json.loads(line)
            for line in hook_failures_path.read_text(encoding="utf-8").splitlines()
            if line.strip()
        ]
        assert [record["hook_phase"] for record in records] == ["pre", "post"]
        assert [record["exit_code"] for record in records] == [7, 9]

    def test_tool_handler_returns_fail_for_execution_errors(self, monkeypatch):
        graph = parse_dot(
            """
            digraph G {
                tool_node [shape=parallelogram, tool_command="echo hi"]
            }
            """
        )

        def _raise_exec_error(*args, **kwargs):
            del args, kwargs
            raise OSError("execution exploded")

        monkeypatch.setattr("attractor.handlers.builtin.tool.subprocess.run", _raise_exec_error)

        registry = build_default_registry(codergen_backend=_StubBackend())
        runner = HandlerRunner(graph, registry)
        outcome = runner("tool_node", "", Context())

        assert outcome.status == OutcomeStatus.FAIL
        assert outcome.failure_reason == "execution exploded"

    def test_tool_handler_runs_post_hook_after_tool_command(self, tmp_path):
        hook_file = tmp_path / "hook.log"
        tool_file = tmp_path / "tool.log"
        tool_command = f"printf ran >> {shlex.quote(str(tool_file))}"
        post_hook = f"test -f {shlex.quote(str(tool_file))} && printf post >> {shlex.quote(str(hook_file))}"
        graph = parse_dot(
            f"""
            digraph G {{
                graph [tool_hooks.post="{post_hook}"]
                tool_node [shape=parallelogram, tool_command="{tool_command}"]
            }}
            """
        )
        registry = build_default_registry(codergen_backend=_StubBackend())
        runner = HandlerRunner(graph, registry)

        outcome = runner("tool_node", "", Context())

        assert outcome.status == OutcomeStatus.SUCCESS
        assert tool_file.read_text(encoding="utf-8") == "ran"
        assert hook_file.read_text(encoding="utf-8") == "post"

    def test_tool_handler_runs_pre_hook_before_tool_command(self, tmp_path):
        hook_file = tmp_path / "hook.log"
        tool_file = tmp_path / "tool.log"
        pre_hook = f"printf pre >> {shlex.quote(str(hook_file))}"
        tool_command = f"test -f {shlex.quote(str(hook_file))} && printf ran >> {shlex.quote(str(tool_file))}"
        graph = parse_dot(
            f"""
            digraph G {{
                graph [tool_hooks.pre="{pre_hook}"]
                tool_node [shape=parallelogram, tool_command="{tool_command}"]
            }}
            """
        )
        registry = build_default_registry(codergen_backend=_StubBackend())
        runner = HandlerRunner(graph, registry)

        outcome = runner("tool_node", "", Context())

        assert outcome.status == OutcomeStatus.SUCCESS
        assert hook_file.read_text(encoding="utf-8") == "pre"
        assert tool_file.read_text(encoding="utf-8") == "ran"

    def test_tool_handler_timeout_surfaces_command_context(self, monkeypatch):
        graph = parse_dot(
            """
            digraph G {
                tool_node [shape=parallelogram, tool_command="sleep 5", timeout=50ms]
            }
            """
        )

        def _raise_timeout(command, **kwargs):
            raise subprocess.TimeoutExpired(
                cmd=command,
                timeout=kwargs["timeout"],
                output="partial output",
            )

        monkeypatch.setattr("attractor.handlers.builtin.tool.subprocess.run", _raise_timeout)

        registry = build_default_registry(codergen_backend=_StubBackend())
        runner = HandlerRunner(graph, registry)
        outcome = runner("tool_node", "", Context())

        assert outcome.status == OutcomeStatus.FAIL
        assert "timed out" in (outcome.failure_reason or "")
        assert "sleep 5" in (outcome.failure_reason or "")
        assert outcome.context_updates["context.tool.output"] == "partial output"
        assert outcome.context_updates["context.tool.exit_code"] == -1

    def test_tool_handler_writes_output_artifact(self, tmp_path):
        graph = parse_dot(
            """
            digraph G {
                tool_node [shape=parallelogram, tool_command="printf hello"]
            }
            """
        )
        logs_root = tmp_path / "logs"
        registry = build_default_registry(codergen_backend=_StubBackend())
        runner = HandlerRunner(graph, registry, logs_root=logs_root)

        outcome = runner("tool_node", "", Context())

        assert outcome.status == OutcomeStatus.SUCCESS
        artifact_path = logs_root / "tool_node" / "tool_output.txt"
        assert artifact_path.exists()
        assert artifact_path.read_text(encoding="utf-8") == "hello"
