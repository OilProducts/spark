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
    _ArtifactProbeBackend,
    _TextBackend,
    _OutcomeBackend,
)

class TestCodergenHandler:
    def test_codergen_handler_calls_backend(self):
        graph = parse_dot(
            """
            digraph G {
                start [shape=Mdiamond]
                task [shape=box, prompt="Plan for $goal"]
                done [shape=Msquare]
                start -> task
                task -> done
            }
            """
        )

        backend = _StubBackend(ok=True)
        registry = build_default_registry(codergen_backend=backend)
        runner = HandlerRunner(graph, registry)
        ctx = Context(values={"graph.goal": "ship"})

        outcome = runner("task", "Plan for $goal", ctx)
        assert outcome.status == OutcomeStatus.SUCCESS
        assert backend.calls[0][1] == "Plan for ship"

    def test_codergen_handler_expands_goal_from_graph_attr_when_context_missing(self):
        graph = parse_dot(
            """
            digraph G {
                graph [goal="Ship docs"]
                task [shape=box, prompt="Plan for $goal"]
            }
            """
        )

        backend = _StubBackend(ok=True)
        registry = build_default_registry(codergen_backend=backend)
        runner = HandlerRunner(graph, registry)

        outcome = runner("task", "Plan for $goal", Context())
        assert outcome.status == OutcomeStatus.SUCCESS
        assert backend.calls[0][1] == "Plan for Ship docs"

    def test_codergen_handler_falls_back_to_label_when_prompt_is_empty(self):
        graph = parse_dot(
            """
            digraph G {
                task [shape=box, label="Label Prompt"]
            }
            """
        )

        backend = _StubBackend(ok=True)
        registry = build_default_registry(codergen_backend=backend)
        runner = HandlerRunner(graph, registry)

        outcome = runner("task", "", Context())
        assert outcome.status == OutcomeStatus.SUCCESS
        assert backend.calls[0][1] == "Label Prompt"

    def test_codergen_handler_returns_simulation_response_when_backend_absent(self):
        graph = parse_dot(
            """
            digraph G {
                task [shape=box, prompt="Plan for $goal"]
            }
            """
        )

        with tempfile.TemporaryDirectory() as tmp:
            logs_root = Path(tmp) / "logs"
            registry = build_default_registry(codergen_backend=None)
            runner = HandlerRunner(graph, registry, logs_root=logs_root)

            outcome = runner("task", "Plan for $goal", Context(values={"graph.goal": "ship"}))

            assert outcome.status == OutcomeStatus.SUCCESS
            assert outcome.notes == "Stage completed: task"
            assert outcome.context_updates == {
                "last_response": "[Simulated] Response for stage: task",
                "last_stage": "task",
            }
            response_path = logs_root / "task" / "response.md"
            assert response_path.exists()
            assert response_path.read_text(encoding="utf-8").strip() == "[Simulated] Response for stage: task"

    def test_codergen_handler_supports_backend_outcome_response(self):
        graph = parse_dot(
            """
            digraph G {
                task [shape=box, prompt="Plan for $goal"]
            }
            """
        )

        backend_outcome = Outcome(
            status=OutcomeStatus.RETRY,
            notes="please retry",
            suggested_next_ids=["fallback_stage"],
            context_updates={"work.last": "task"},
        )
        registry = build_default_registry(codergen_backend=_OutcomeBackend(backend_outcome))
        runner = HandlerRunner(graph, registry)

        outcome = runner("task", "Plan for $goal", Context(values={"graph.goal": "ship"}))

        assert outcome is backend_outcome
        assert outcome.status == OutcomeStatus.RETRY
        assert outcome.notes == "please retry"
        assert outcome.suggested_next_ids == ["fallback_stage"]
        assert outcome.context_updates == {"work.last": "task"}

    def test_codergen_handler_supports_backend_text_response(self):
        graph = parse_dot(
            """
            digraph G {
                task [shape=box, prompt="Plan for $goal"]
            }
            """
        )

        with tempfile.TemporaryDirectory() as tmp:
            logs_root = Path(tmp) / "logs"
            backend = _TextBackend("backend text response")
            registry = build_default_registry(codergen_backend=backend)
            runner = HandlerRunner(graph, registry, logs_root=logs_root)

            outcome = runner("task", "Plan for $goal", Context(values={"graph.goal": "ship"}))

            assert outcome.status == OutcomeStatus.SUCCESS
            response_path = logs_root / "task" / "response.md"
            assert response_path.exists()
            assert response_path.read_text(encoding="utf-8").strip() == "backend text response"

    def test_codergen_handler_writes_prompt_before_backend_and_response_afterward(self):
        graph = parse_dot(
            """
            digraph G {
                task [shape=box, prompt="Plan for $goal"]
            }
            """
        )

        with tempfile.TemporaryDirectory() as tmp:
            logs_root = Path(tmp) / "logs"
            backend = _ArtifactProbeBackend(logs_root)
            registry = build_default_registry(codergen_backend=backend)
            runner = HandlerRunner(graph, registry, logs_root=logs_root)
            ctx = Context(values={"graph.goal": "ship"})

            outcome = runner("task", "Plan for $goal", ctx)

            assert outcome.status == OutcomeStatus.SUCCESS
            assert backend.prompt_exists_during_call is True
            assert backend.prompt_text_during_call == "Plan for ship"
            assert backend.response_exists_during_call is False
            response_path = logs_root / "task" / "response.md"
            assert response_path.exists()
            assert response_path.read_text(encoding="utf-8").strip() == "codergen backend success"

    def test_codergen_handler_writes_status_json_from_final_outcome(self):
        graph = parse_dot(
            """
            digraph G {
                task [shape=box, prompt="Plan for $goal"]
            }
            """
        )
        backend_outcome = Outcome(
            status=OutcomeStatus.PARTIAL_SUCCESS,
            preferred_label="continue",
            suggested_next_ids=["followup"],
            context_updates={"work.last": "task"},
            notes="backend returned partial",
        )

        with tempfile.TemporaryDirectory() as tmp:
            logs_root = Path(tmp) / "logs"
            registry = build_default_registry(codergen_backend=_OutcomeBackend(backend_outcome))
            runner = HandlerRunner(graph, registry, logs_root=logs_root)

            outcome = runner("task", "Plan for $goal", Context(values={"graph.goal": "ship"}))

            assert outcome is backend_outcome
            status_path = logs_root / "task" / "status.json"
            assert status_path.exists()
            assert json.loads(status_path.read_text(encoding="utf-8")) == {
                "context_updates": {"work.last": "task"},
                "notes": "backend returned partial",
                "outcome": "partial_success",
                "preferred_next_label": "continue",
                "suggested_next_ids": ["followup"],
            }
