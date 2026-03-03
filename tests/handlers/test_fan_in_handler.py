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
    _FanInRankingBackend,
)

class TestFanInHandler:
    def test_fan_in_uses_backend_ranking_when_prompt_present(self):
        graph = parse_dot(
            """
            digraph G {
                fan_in [shape=tripleoctagon, prompt="Rank the branch results"]
            }
            """
        )
        backend = _FanInRankingBackend('{"best_id":"branch_b"}')
        registry = build_default_registry(codergen_backend=backend)
        runner = HandlerRunner(graph, registry)
        context = Context(
            values={
                "parallel.results": [
                    {"id": "branch_a", "status": "success"},
                    {"id": "branch_b", "status": "success"},
                ]
            }
        )

        outcome = runner("fan_in", "Rank the branch results", context)

        assert outcome.status == OutcomeStatus.SUCCESS
        assert outcome.context_updates["parallel.fan_in.best_id"] == "branch_b"
        assert outcome.context_updates["parallel.fan_in.best_outcome"] == "success"
        assert len(backend.calls) == 1
        assert "Rank the branch results" in backend.calls[0]["prompt"]

    def test_fan_in_uses_heuristic_score_fallback_without_prompt(self):
        graph = parse_dot(
            """
            digraph G {
                fan_in [shape=tripleoctagon]
            }
            """
        )
        registry = build_default_registry(codergen_backend=_StubBackend())
        runner = HandlerRunner(graph, registry)
        context = Context(
            values={
                "parallel.results": [
                    {"id": "branch_a", "status": "success", "score": 0.2},
                    {"id": "branch_b", "status": "success", "score": 0.9},
                    {"id": "branch_c", "status": "partial_success", "score": 1.0},
                ]
            }
        )

        outcome = runner("fan_in", "", context)

        assert outcome.status == OutcomeStatus.SUCCESS
        assert outcome.context_updates["parallel.fan_in.best_id"] == "branch_b"
        assert outcome.context_updates["parallel.fan_in.best_outcome"] == "success"
