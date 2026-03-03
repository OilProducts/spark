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
    _PluginHandler,
)

class TestRegistryResolution:
    def test_registry_resolution_by_shape_and_type(self):
        graph = parse_dot(
            """
            digraph G {
                start [shape=Mdiamond]
                human [shape=hexagon]
                custom [shape=box, type="tool", tool_command="printf hi"]
                done [shape=Msquare]
                start -> human [label="Approve"]
                human -> custom [label="Go"]
                custom -> done
            }
            """
        )

        registry = build_default_registry(codergen_backend=_StubBackend())
        assert registry.resolve_handler_type(graph.nodes["start"]) == "start"
        assert registry.resolve_handler_type(graph.nodes["human"]) == "wait.human"
        assert registry.resolve_handler_type(graph.nodes["custom"]) == "tool"

    def test_registry_falls_back_to_shape_when_explicit_type_is_unregistered(self):
        graph = parse_dot(
            """
            digraph G {
                gate [shape=hexagon, type="custom.missing", prompt="Choose"]
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

    def test_registry_falls_back_to_default_handler_when_shape_mapping_is_unregistered(self):
        graph = parse_dot(
            """
            digraph G {
                task [shape=hexagon, label="Use default", type="custom.missing"]
            }
            """
        )
        backend = _StubBackend(ok=True)
        registry = build_default_registry(codergen_backend=backend)
        del registry.handlers["wait.human"]
        runner = HandlerRunner(graph, registry)

        outcome = runner("task", "", Context())

        assert outcome.status == OutcomeStatus.SUCCESS
        assert backend.calls[0][0] == "task"
        assert backend.calls[0][1] == "Use default"

    @pytest.mark.parametrize(
        ("node_attrs", "expected_handler_type"),
        [
            ('shape=box, type="tool", tool_command="printf hi"', "tool"),
            ('shape=" hexagon "', "wait.human"),
            ('shape="unknown"', "codergen"),
        ],
    )
    def test_registry_resolution_precedence_levels(self, node_attrs, expected_handler_type):
        graph = parse_dot(
            f"""
            digraph G {{
                stage [{node_attrs}]
            }}
            """
        )
        registry = build_default_registry(codergen_backend=_StubBackend())

        assert registry.resolve_handler_type(graph.nodes["stage"]) == expected_handler_type

    @pytest.mark.parametrize(
        ("shape", "expected_handler_type"),
        [
            ("Mdiamond", "start"),
            ("Msquare", "exit"),
            ("box", "codergen"),
            ("hexagon", "wait.human"),
            ("diamond", "conditional"),
            ("component", "parallel"),
            ("tripleoctagon", "parallel.fan_in"),
            ("parallelogram", "tool"),
            ("house", "stack.manager_loop"),
        ],
    )
    def test_registry_shape_mapping_covers_all_spec_shapes(self, shape, expected_handler_type):
        graph = parse_dot(
            f"""
            digraph G {{
                stage [shape={shape}]
            }}
            """
        )
        registry = build_default_registry(codergen_backend=_StubBackend())

        assert SHAPE_TO_TYPE[shape] == expected_handler_type
        assert registry.resolve_handler_type(graph.nodes["stage"]) == expected_handler_type
        assert expected_handler_type in registry.handlers

    def test_registry_allows_plugin_injection_via_builder(self):
        graph = parse_dot(
            """
            digraph G {
                plugin_stage [shape=box, type="custom.plugin"]
            }
            """
        )
        registry = build_default_registry(
            codergen_backend=_StubBackend(),
            extra_handlers={"custom.plugin": _PluginHandler()},
        )
        runner = HandlerRunner(graph, registry)
        outcome = runner("plugin_stage", "", Context())
        assert outcome.status == OutcomeStatus.SUCCESS
        assert outcome.notes == "plugin:plugin_stage"
