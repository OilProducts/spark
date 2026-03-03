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
    _ExecuteOnlyHandler,
    _RuntimeCaptureHandler,
    _SlowHandler,
    _ConcurrentOutsideParallelHandler,
    _AlwaysSuccessHandler,
    _SystemExitHandler,
)

class TestHandlerRunnerContracts:
    def test_handler_runner_dispatches_execute_contract(self):
        graph = parse_dot(
            """
            digraph G {
                execute_stage [shape=box, type="custom.execute"]
            }
            """
        )
        execute_handler = _ExecuteOnlyHandler()
        registry = build_default_registry(
            codergen_backend=_StubBackend(),
            extra_handlers={"custom.execute": execute_handler},
        )
        runner = HandlerRunner(graph, registry)
        context = Context(values={"seed": "ok"})

        outcome = runner("execute_stage", "ignored", context)

        assert outcome.status == OutcomeStatus.SUCCESS
        assert outcome.notes == "execute:execute_stage"
        assert len(execute_handler.calls) == 1
        assert execute_handler.calls[0].node_id == "execute_stage"

    def test_handler_contract_receives_node_context_graph_and_logs_root(self, tmp_path):
        graph = parse_dot(
            """
            digraph G {
                start [shape=Mdiamond]
                capture [shape=box, type="custom.capture"]
                done [shape=Msquare]
                start -> capture
                capture -> done
            }
            """
        )
        capture_handler = _RuntimeCaptureHandler()
        registry = build_default_registry(
            codergen_backend=_StubBackend(),
            extra_handlers={"custom.capture": capture_handler},
        )
        runner = HandlerRunner(graph, registry)
        context = Context(values={"seed": "ok"})

        result = PipelineExecutor(graph, runner, logs_root=str(tmp_path)).run(context)

        assert result.status == "success"
        assert len(capture_handler.calls) == 1
        runtime = capture_handler.calls[0]
        assert runtime.node is graph.nodes["capture"]
        assert runtime.context is context
        assert runtime.graph is graph
        assert runtime.logs_root == tmp_path

    def test_handler_contract_normalizes_string_logs_root_to_path(self, tmp_path):
        graph = parse_dot(
            """
            digraph G {
                capture [shape=box, type="custom.capture"]
            }
            """
        )
        capture_handler = _RuntimeCaptureHandler()
        registry = build_default_registry(
            codergen_backend=_StubBackend(),
            extra_handlers={"custom.capture": capture_handler},
        )
        runner = HandlerRunner(graph, registry, logs_root=str(tmp_path))

        outcome = runner("capture", "", Context())

        assert outcome.status == OutcomeStatus.SUCCESS
        assert len(capture_handler.calls) == 1
        runtime = capture_handler.calls[0]
        assert runtime.logs_root == tmp_path
        assert isinstance(runtime.logs_root, Path)

    def test_custom_handler_system_exit_is_converted_to_fail_outcome(self):
        graph = parse_dot(
            """
            digraph G {
                start [shape=Mdiamond]
                risky [shape=box, type="custom.system_exit"]
                recover [shape=box, type="custom.success"]
                done [shape=Msquare]
                start -> risky
                risky -> done [condition="outcome=success"]
                risky -> recover [condition="outcome=fail"]
                recover -> done
            }
            """
        )
        registry = build_default_registry(
            codergen_backend=_StubBackend(),
            extra_handlers={
                "custom.system_exit": _SystemExitHandler(),
                "custom.success": _AlwaysSuccessHandler(),
            },
        )
        runner = HandlerRunner(graph, registry)

        result = PipelineExecutor(graph, runner).run(Context())

        assert result.status == "success"
        assert result.route_trace == ["start", "risky", "recover", "done"]
        assert result.node_outcomes["risky"].status is OutcomeStatus.FAIL
        assert result.node_outcomes["risky"].failure_reason == "handler terminated abruptly"

    def test_handler_runner_enforces_node_timeout(self):
        graph = parse_dot(
            """
            digraph G {
                slow [shape=box, type="custom.slow", timeout=50ms]
            }
            """
        )
        registry = build_default_registry(
            codergen_backend=_StubBackend(),
            extra_handlers={"custom.slow": _SlowHandler()},
        )
        runner = HandlerRunner(graph, registry)

        outcome = runner("slow", "", Context())
        assert outcome.status == OutcomeStatus.FAIL
        assert outcome.failure_reason == "handler timed out after 0.05s"

    def test_handler_runner_rejects_concurrent_nested_calls_outside_parallel_handler(self):
        graph = parse_dot(
            """
            digraph G {
                fan [shape=box, type="custom.concurrent"]
                left [shape=box, type="custom.slow"]
                right [shape=box, type="custom.slow"]
                fan -> left
                fan -> right
            }
            """
        )
        registry = build_default_registry(
            codergen_backend=_StubBackend(),
            extra_handlers={
                "custom.concurrent": _ConcurrentOutsideParallelHandler(),
                "custom.slow": _SlowHandler(),
            },
        )
        runner = HandlerRunner(graph, registry)

        outcome = runner("fan", "", Context())
        assert outcome.status == OutcomeStatus.SUCCESS
        assert outcome.notes == "concurrency gate enforced"
