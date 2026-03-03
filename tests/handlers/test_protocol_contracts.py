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

class TestProtocolContracts:
    def test_codergen_backend_protocol_returns_text_or_outcome(self):
        hints = get_type_hints(CodergenBackend.run)
        return_type = hints["return"]
        assert set(get_args(return_type)) == {str, Outcome}

    def test_codergen_backend_protocol_is_structural(self):
        class _AltBackend:
            def run(self, node_id: str, prompt: str, context: Context, *, timeout=None) -> bool:
                return True

        assert isinstance(_StubBackend(), CodergenBackend)
        assert isinstance(_AltBackend(), CodergenBackend)

    def test_house_shape_resolves_and_executes_with_default_registry(self):
        graph = parse_dot(
            """
            digraph G {
                manager [shape=house, manager.max_cycles=1, manager.poll_interval=0ms]
            }
            """
        )
        registry = build_default_registry(codergen_backend=_StubBackend())
        runner = HandlerRunner(graph, registry)

        assert registry.resolve_handler_type(graph.nodes["manager"]) == "stack.manager_loop"
        outcome = runner("manager", "", Context())
        assert outcome.status == OutcomeStatus.FAIL
        assert "Max cycles exceeded" in outcome.failure_reason
