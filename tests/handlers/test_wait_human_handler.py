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
    _FalseyInterviewer,
)

class TestWaitHumanHandler:
    def test_wait_human_builds_options_with_label_fallback_to_target_id(self):
        graph = parse_dot(
            """
            digraph G {
                gate [shape=hexagon, prompt="Choose"]
                ship [shape=box]
                fix [shape=box]
                gate -> ship [label=""]
                gate -> fix
            }
            """
        )

        seen = {}

        def _capture(question):
            seen["question"] = question
            return Answer(selected_values=[question.options[0].value])

        registry = build_default_registry(
            codergen_backend=_StubBackend(),
            interviewer=CallbackInterviewer(_capture),
        )
        runner = HandlerRunner(graph, registry)

        outcome = runner("gate", "Choose", Context())

        assert outcome.status == OutcomeStatus.SUCCESS
        assert outcome.preferred_label == "ship"
        assert [(option.label, option.value) for option in seen["question"].options] == [
            ("ship", "ship"),
            ("fix", "fix"),
        ]

    def test_wait_human_honors_answer_selected_option_without_value(self):
        graph = parse_dot(
            """
            digraph G {
                gate [shape=hexagon, prompt="Choose"]
                ship [shape=box]
                fix [shape=box]
                gate -> ship [label="Approve"]
                gate -> fix [label="Fix"]
            }
            """
        )

        def _capture(question):
            return Answer(selected_option=question.options[1])

        registry = build_default_registry(
            codergen_backend=_StubBackend(),
            interviewer=CallbackInterviewer(_capture),
        )
        runner = HandlerRunner(graph, registry)
        outcome = runner("gate", "Choose", Context())

        assert outcome.status == OutcomeStatus.SUCCESS
        assert outcome.preferred_label == "Fix"
        assert outcome.suggested_next_ids == ["fix"]
        assert outcome.context_updates == {
            "human.gate.selected": "F",
            "human.gate.label": "Fix",
        }

    def test_wait_human_maps_target_answer_back_to_selected_edge_label(self):
        graph = parse_dot(
            """
            digraph G {
                gate [shape=hexagon, prompt="Choose"]
                ship [shape=box]
                fix [shape=box]
                gate -> ship [label="Approve"]
                gate -> fix [label="Fix"]
            }
            """
        )

        registry = build_default_registry(
            codergen_backend=_StubBackend(),
            interviewer=QueueInterviewer([Answer(selected_values=["ship"])]),
        )
        runner = HandlerRunner(graph, registry)
        outcome = runner("gate", "Choose", Context())

        assert outcome.status == OutcomeStatus.SUCCESS
        assert outcome.preferred_label == "Approve"

    @pytest.mark.parametrize(
        ("label", "expected_key"),
        [
            ("[Y] Yes, deploy", "Y"),
            ("Y) Yes, deploy", "Y"),
            ("Y - Yes, deploy", "Y"),
            ("Yes, deploy", "Y"),
        ],
    )
    def test_wait_human_parses_accelerator_keys_from_supported_label_patterns(self, label, expected_key):
        graph = parse_dot(
            f"""
            digraph G {{
                gate [shape=hexagon, prompt="Choose"]
                yes_path [shape=box]
                no_path [shape=box]
                gate -> yes_path [label="{label}"]
                gate -> no_path [label="[N] No, cancel"]
            }}
            """
        )

        seen = {}

        def _capture(question):
            seen["question"] = question
            return Answer(selected_values=[question.options[0].value])

        registry = build_default_registry(
            codergen_backend=_StubBackend(),
            interviewer=CallbackInterviewer(_capture),
        )
        runner = HandlerRunner(graph, registry)
        outcome = runner("gate", "Choose", Context())

        assert outcome.status == OutcomeStatus.SUCCESS
        assert seen["question"].options[0].key == expected_key
        assert seen["question"].options[1].key == "N"

    def test_wait_human_skipped_answer_returns_fail(self):
        graph = parse_dot(
            """
            digraph G {
                gate [shape=hexagon, prompt="Choose"]
                ship [shape=box]
                fix [shape=box]
                gate -> ship [label="Approve"]
                gate -> fix [label="Fix"]
            }
            """
        )

        registry = build_default_registry(
            codergen_backend=_StubBackend(),
            interviewer=QueueInterviewer([]),
        )
        runner = HandlerRunner(graph, registry)
        outcome = runner("gate", "Choose", Context())

        assert outcome.status == OutcomeStatus.FAIL
        assert outcome.failure_reason == "human skipped interaction"

    def test_wait_human_timeout_default_choice_requires_exact_target_match(self):
        graph = parse_dot(
            """
            digraph G {
                gate [shape=hexagon, prompt="Choose", human.default_choice="FIX"]
                ship [shape=box]
                fix [shape=box]
                gate -> ship [label="Approve"]
                gate -> fix [label="Fix"]
            }
            """
        )

        registry = build_default_registry(
            codergen_backend=_StubBackend(),
            interviewer=QueueInterviewer([Answer(value="TIMEOUT")]),
        )
        runner = HandlerRunner(graph, registry)
        outcome = runner("gate", "Choose", Context())

        assert outcome.status == OutcomeStatus.RETRY
        assert outcome.failure_reason == "human gate timeout, no default"

    def test_wait_human_timeout_uses_human_default_choice_target(self):
        graph = parse_dot(
            """
            digraph G {
                gate [shape=hexagon, prompt="Choose", human.default_choice="fix"]
                ship [shape=box]
                fix [shape=box]
                gate -> ship [label="Approve"]
                gate -> fix [label="Fix"]
            }
            """
        )

        registry = build_default_registry(
            codergen_backend=_StubBackend(),
            interviewer=QueueInterviewer([Answer(value="TIMEOUT")]),
        )
        runner = HandlerRunner(graph, registry)
        outcome = runner("gate", "Choose", Context())

        assert outcome.status == OutcomeStatus.SUCCESS
        assert outcome.preferred_label == "Fix"
        assert outcome.suggested_next_ids == ["fix"]
        assert outcome.context_updates == {
            "human.gate.selected": "F",
            "human.gate.label": "Fix",
        }

    def test_wait_human_timeout_without_default_returns_retry(self):
        graph = parse_dot(
            """
            digraph G {
                gate [shape=hexagon, prompt="Choose"]
                ship [shape=box]
                fix [shape=box]
                gate -> ship [label="Approve"]
                gate -> fix [label="Fix"]
            }
            """
        )

        registry = build_default_registry(
            codergen_backend=_StubBackend(),
            interviewer=QueueInterviewer([Answer(value="TIMEOUT")]),
        )
        runner = HandlerRunner(graph, registry)
        outcome = runner("gate", "Choose", Context())

        assert outcome.status == OutcomeStatus.RETRY
        assert outcome.failure_reason == "human gate timeout, no default"

    def test_wait_human_uses_falsey_external_interviewer(self):
        graph = parse_dot(
            """
            digraph G {
                gate [shape=hexagon, prompt="Choose"]
                pass [shape=box]
                fail [shape=box]
                gate -> pass [label="Approve"]
                gate -> fail [label="Fix"]
            }
            """
        )

        registry = build_default_registry(
            codergen_backend=_StubBackend(),
            interviewer=_FalseyInterviewer(),
        )
        runner = HandlerRunner(graph, registry)

        outcome = runner("gate", "Choose", Context())
        assert outcome.status == OutcomeStatus.SUCCESS
        assert outcome.preferred_label == "Fix"

    def test_wait_human_uses_interviewer_and_sets_preferred_label(self):
        graph = parse_dot(
            """
            digraph G {
                gate [shape=hexagon, prompt="Choose"]
                pass [shape=box]
                fail [shape=box]
                gate -> pass [label="Approve"]
                gate -> fail [label="Fix"]
            }
            """
        )

        registry = build_default_registry(codergen_backend=_StubBackend())
        runner = HandlerRunner(graph, registry)

        outcome = runner("gate", "Choose", Context())
        assert outcome.status == OutcomeStatus.SUCCESS
        assert outcome.preferred_label == "Approve"
        assert outcome.suggested_next_ids == ["pass"]
        assert outcome.context_updates == {
            "human.gate.selected": "A",
            "human.gate.label": "Approve",
        }
