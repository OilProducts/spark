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

class TestBuiltInNoopHandlers:
    def test_conditional_handler_is_noop_success(self):
        graph = parse_dot(
            """
            digraph G {
                gate [shape=diamond]
            }
            """
        )
        registry = build_default_registry(codergen_backend=_StubBackend())
        runner = HandlerRunner(graph, registry)

        outcome = runner("gate", "", Context(values={"outcome": "fail"}))
        assert outcome.status == OutcomeStatus.SUCCESS
        assert outcome.notes == ""
        assert outcome.context_updates == {}

    def test_start_handler_is_noop_success(self):
        graph = parse_dot(
            """
            digraph G {
                start [shape=Mdiamond]
            }
            """
        )
        registry = build_default_registry(codergen_backend=_StubBackend())
        runner = HandlerRunner(graph, registry)

        outcome = runner("start", "", Context(values={"goal": "ship"}))

        assert outcome.status == OutcomeStatus.SUCCESS
        assert outcome.notes == ""
        assert outcome.context_updates == {}

    def test_exit_handler_is_noop_success(self):
        graph = parse_dot(
            """
            digraph G {
                done [shape=Msquare]
            }
            """
        )
        registry = build_default_registry(codergen_backend=_StubBackend())
        runner = HandlerRunner(graph, registry)

        outcome = runner("done", "", Context(values={"outcome": "fail"}))

        assert outcome.status == OutcomeStatus.SUCCESS
        assert outcome.notes == ""
        assert outcome.context_updates == {}
