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
    _RuntimeCaptureHandler,
    _SharedRefSeedHandler,
    _SharedRefIsolationChecker,
    _MaxParallelProbeHandler,
    _CustomConcurrencyProbeHandler,
    _AlwaysSuccessHandler,
    _AlwaysFailHandler,
)

class TestParallelHandler:
    def test_parallel_branches_keep_context_updates_isolated_per_branch(self):
        graph = parse_dot(
            """
            digraph G {
                fan [shape=component]
                a_seed [shape=box, type="custom.seed_a"]
                b_seed [shape=box, type="custom.seed_b"]
                a_check [shape=box, type="custom.check_a"]
                b_check [shape=box, type="custom.check_b"]
                a_stop [shape=tripleoctagon]
                b_stop [shape=tripleoctagon]

                fan -> a_seed
                fan -> b_seed
                a_seed -> a_check
                b_seed -> b_check
                a_check -> a_stop [condition="outcome=success"]
                b_check -> b_stop [condition="outcome=success"]
            }
            """
        )
        shared_ref = {"markers": []}
        barrier = threading.Barrier(2)
        registry = build_default_registry(
            codergen_backend=_StubBackend(),
            extra_handlers={
                "custom.seed_a": _SharedRefSeedHandler(shared_ref),
                "custom.seed_b": _SharedRefSeedHandler(shared_ref),
                "custom.check_a": _SharedRefIsolationChecker("a", barrier),
                "custom.check_b": _SharedRefIsolationChecker("b", barrier),
            },
        )
        runner = HandlerRunner(graph, registry)
        context = Context(values={"base": "kept"})

        outcome = runner("fan", "", context)
        context.merge_updates(outcome.context_updates)

        assert outcome.status == OutcomeStatus.SUCCESS
        assert context.get("base") == "kept"
        assert context.get("shared_ref", "") == ""
        branch_results = context.get("parallel.results", [])
        assert len(branch_results) == 2
        assert all(item.get("status") == "success" for item in branch_results)

    def test_parallel_handler_respects_max_parallel_bound(self):
        graph = parse_dot(
            """
            digraph G {
                fan [shape=component, max_parallel=2]
                a [shape=box, type="custom.probe"]
                b [shape=box, type="custom.probe"]
                c [shape=box, type="custom.probe"]
                d [shape=box, type="custom.probe"]
                a_stop [shape=tripleoctagon]
                b_stop [shape=tripleoctagon]
                c_stop [shape=tripleoctagon]
                d_stop [shape=tripleoctagon]

                fan -> a
                fan -> b
                fan -> c
                fan -> d
                a -> a_stop [condition="outcome=success"]
                b -> b_stop [condition="outcome=success"]
                c -> c_stop [condition="outcome=success"]
                d -> d_stop [condition="outcome=success"]
            }
            """
        )
        state = {"lock": threading.Lock(), "in_flight": 0, "peak": 0}
        registry = build_default_registry(
            codergen_backend=_StubBackend(),
            extra_handlers={"custom.probe": _MaxParallelProbeHandler(state)},
        )
        runner = HandlerRunner(graph, registry)

        outcome = runner("fan", "", Context())
        assert outcome.status == OutcomeStatus.SUCCESS
        assert state["peak"] <= 2
        branch_results = outcome.context_updates.get("parallel.results", [])
        assert isinstance(branch_results, list)
        assert len(branch_results) == 4

    def test_custom_handler_without_thread_safe_marker_is_serialized_under_parallel_execution(self):
        graph = parse_dot(
            """
            digraph G {
                fan [shape=component, max_parallel=4]
                a [shape=box, type="custom.probe"]
                b [shape=box, type="custom.probe"]
                c [shape=box, type="custom.probe"]
                d [shape=box, type="custom.probe"]
                a_stop [shape=tripleoctagon]
                b_stop [shape=tripleoctagon]
                c_stop [shape=tripleoctagon]
                d_stop [shape=tripleoctagon]

                fan -> a
                fan -> b
                fan -> c
                fan -> d
                a -> a_stop [condition="outcome=success"]
                b -> b_stop [condition="outcome=success"]
                c -> c_stop [condition="outcome=success"]
                d -> d_stop [condition="outcome=success"]
            }
            """
        )
        state = {"lock": threading.Lock(), "in_flight": 0, "peak": 0}
        registry = build_default_registry(
            codergen_backend=_StubBackend(),
            extra_handlers={"custom.probe": _CustomConcurrencyProbeHandler(state)},
        )
        runner = HandlerRunner(graph, registry)

        outcome = runner("fan", "", Context())

        assert outcome.status == OutcomeStatus.SUCCESS
        assert state["peak"] == 1

    def test_parallel_handler_rejects_non_positive_max_parallel(self):
        graph = parse_dot(
            """
            digraph G {
                fan [shape=component, max_parallel=0]
                a [shape=box]
                b [shape=box]
                a_stop [shape=tripleoctagon]
                b_stop [shape=tripleoctagon]

                fan -> a
                fan -> b
                a -> a_stop [condition="outcome=success"]
                b -> b_stop [condition="outcome=success"]
            }
            """
        )
        registry = build_default_registry(codergen_backend=_StubBackend())
        runner = HandlerRunner(graph, registry)

        outcome = runner("fan", "", Context())
        assert outcome.status == OutcomeStatus.FAIL
        assert outcome.failure_reason == "max_parallel must be >= 1"

    def test_parallel_handler_preserves_logs_root_for_branch_and_followup_calls(self, tmp_path):
        graph = parse_dot(
            """
            digraph G {
                fan [shape=component]
                a [shape=box, type="custom.capture"]
                b [shape=box, type="custom.capture"]
                join [shape=tripleoctagon]
                post [shape=box, type="custom.capture"]
                fan -> a
                fan -> b
                a -> join [condition="outcome=success"]
                b -> join [condition="outcome=success"]
            }
            """
        )
        capture_handler = _RuntimeCaptureHandler()
        registry = build_default_registry(
            codergen_backend=_StubBackend(),
            extra_handlers={"custom.capture": capture_handler},
        )
        runner = HandlerRunner(graph, registry, logs_root=tmp_path)

        parallel_outcome = runner("fan", "", Context())
        followup_outcome = runner("post", "", Context())

        assert parallel_outcome.status == OutcomeStatus.SUCCESS
        assert followup_outcome.status == OutcomeStatus.SUCCESS

        captures = [runtime for runtime in capture_handler.calls if runtime.node_id in {"a", "b", "post"}]
        assert len(captures) == 3
        assert all(runtime.logs_root == tmp_path for runtime in captures)

    @pytest.mark.parametrize(
        ("join_policy", "join_attrs", "expected_status"),
        [
            ("wait_all", "", OutcomeStatus.PARTIAL_SUCCESS),
            ("first_success", "", OutcomeStatus.SUCCESS),
            ("k_of_n", ", join_k=2", OutcomeStatus.SUCCESS),
            ("quorum", ", join_quorum=0.66", OutcomeStatus.SUCCESS),
        ],
    )
    def test_parallel_handler_supports_configured_join_policies(
        self,
        join_policy: str,
        join_attrs: str,
        expected_status: OutcomeStatus,
    ):
        graph = parse_dot(
            f"""
            digraph G {{
                fan [shape=component, join_policy={join_policy}{join_attrs}]
                good_a [shape=box, type="custom.success"]
                bad [shape=box, type="custom.fail"]
                good_b [shape=box, type="custom.success"]
                stop_a [shape=tripleoctagon]
                stop_b [shape=tripleoctagon]

                fan -> good_a
                fan -> bad
                fan -> good_b
                good_a -> stop_a [condition="outcome=success"]
                good_b -> stop_b [condition="outcome=success"]
            }}
            """
        )
        registry = build_default_registry(
            codergen_backend=_StubBackend(),
            extra_handlers={
                "custom.success": _AlwaysSuccessHandler(),
                "custom.fail": _AlwaysFailHandler(),
            },
        )
        runner = HandlerRunner(graph, registry)

        outcome = runner("fan", "", Context())

        assert outcome.status == expected_status
        branch_results = outcome.context_updates.get("parallel.results", [])
        assert isinstance(branch_results, list)
        assert len(branch_results) >= 1

    def test_parallel_handler_rejects_unknown_join_policy(self):
        graph = parse_dot(
            """
            digraph G {
                fan [shape=component, join_policy=not_a_policy]
                a [shape=box, type="custom.success"]
                stop_a [shape=tripleoctagon]
                fan -> a
                a -> stop_a [condition="outcome=success"]
            }
            """
        )
        registry = build_default_registry(
            codergen_backend=_StubBackend(),
            extra_handlers={"custom.success": _AlwaysSuccessHandler()},
        )
        runner = HandlerRunner(graph, registry)

        outcome = runner("fan", "", Context())

        assert outcome.status == OutcomeStatus.FAIL
        assert outcome.failure_reason == "unsupported join_policy: not_a_policy"

    def test_parallel_handler_rejects_unknown_error_policy(self):
        graph = parse_dot(
            """
            digraph G {
                fan [shape=component, error_policy=not_a_policy]
                a [shape=box, type="custom.success"]
                stop_a [shape=tripleoctagon]
                fan -> a
                a -> stop_a [condition="outcome=success"]
            }
            """
        )
        registry = build_default_registry(
            codergen_backend=_StubBackend(),
            extra_handlers={"custom.success": _AlwaysSuccessHandler()},
        )
        runner = HandlerRunner(graph, registry)

        outcome = runner("fan", "", Context())

        assert outcome.status == OutcomeStatus.FAIL
        assert outcome.failure_reason == "unsupported error_policy: not_a_policy"

    @pytest.mark.parametrize(
        ("error_policy", "expected_status", "expected_result_count", "expected_failures"),
        [
            ("continue", OutcomeStatus.PARTIAL_SUCCESS, 3, 1),
            ("ignore", OutcomeStatus.SUCCESS, 2, 0),
            ("fail_fast", OutcomeStatus.PARTIAL_SUCCESS, 1, 1),
        ],
    )
    def test_parallel_handler_supports_error_policies(
        self,
        error_policy: str,
        expected_status: OutcomeStatus,
        expected_result_count: int,
        expected_failures: int,
    ):
        graph = parse_dot(
            f"""
            digraph G {{
                fan [shape=component, join_policy=wait_all, error_policy={error_policy}, max_parallel=1]
                bad [shape=box, type="custom.fail"]
                good_a [shape=box, type="custom.success"]
                good_b [shape=box, type="custom.success"]
                stop_a [shape=tripleoctagon]
                stop_b [shape=tripleoctagon]

                fan -> bad
                fan -> good_a
                fan -> good_b
                good_a -> stop_a [condition="outcome=success"]
                good_b -> stop_b [condition="outcome=success"]
            }}
            """
        )
        registry = build_default_registry(
            codergen_backend=_StubBackend(),
            extra_handlers={
                "custom.success": _AlwaysSuccessHandler(),
                "custom.fail": _AlwaysFailHandler(),
            },
        )
        runner = HandlerRunner(graph, registry)

        outcome = runner("fan", "", Context())

        assert outcome.status == expected_status
        branch_results = outcome.context_updates.get("parallel.results", [])
        assert isinstance(branch_results, list)
        assert len(branch_results) == expected_result_count
        fail_count = sum(1 for result in branch_results if result.get("status") == "fail")
        assert fail_count == expected_failures
