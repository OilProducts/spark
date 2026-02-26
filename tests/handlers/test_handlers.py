import threading
import time
from pathlib import Path

import pytest

from attractor.dsl import parse_dot
from attractor.engine.context import Context
from attractor.engine.executor import PipelineExecutor
from attractor.engine.outcome import Outcome
from attractor.engine.outcome import OutcomeStatus
from attractor.handlers.base import CodergenBackend
from attractor.handlers import HandlerRunner, build_default_registry
from attractor.handlers.registry import SHAPE_TO_TYPE
from attractor.interviewer import Answer, Interviewer, Question


class _StubBackend:
    def __init__(self, ok: bool = True):
        self.ok = ok
        self.calls = []

    def run(self, node_id: str, prompt: str, context: Context, *, timeout=None) -> bool:
        self.calls.append((node_id, prompt, dict(context.values)))
        return self.ok


class _PluginHandler:
    def run(self, runtime):
        return Outcome(status=OutcomeStatus.SUCCESS, notes=f"plugin:{runtime.node_id}")


class _ExecuteOnlyHandler:
    def __init__(self):
        self.calls = []

    def execute(self, runtime):
        self.calls.append(runtime)
        return Outcome(status=OutcomeStatus.SUCCESS, notes=f"execute:{runtime.node_id}")


class _RuntimeCaptureHandler:
    def __init__(self):
        self.calls = []

    def execute(self, runtime):
        self.calls.append(runtime)
        return Outcome(status=OutcomeStatus.SUCCESS, notes="captured")


class _SlowHandler:
    def run(self, runtime):
        time.sleep(0.2)
        return Outcome(status=OutcomeStatus.SUCCESS, notes="slow handler completed")


class _ConcurrentOutsideParallelHandler:
    def run(self, runtime):
        targets = [edge.target for edge in runtime.outgoing_edges]
        if len(targets) < 2:
            return Outcome(status=OutcomeStatus.FAIL, failure_reason="need at least two branches")

        barrier = threading.Barrier(2)
        errors = []

        def invoke(target: str) -> None:
            try:
                barrier.wait(timeout=1.0)
                runtime.runner(target, "", runtime.context.clone())
            except Exception as exc:
                errors.append(str(exc))

        threads = [
            threading.Thread(target=invoke, args=(targets[0],), daemon=True),
            threading.Thread(target=invoke, args=(targets[1],), daemon=True),
        ]
        for thread in threads:
            thread.start()
        for thread in threads:
            thread.join(timeout=2.0)

        if any("parallel handlers" in message for message in errors):
            return Outcome(status=OutcomeStatus.SUCCESS, notes="concurrency gate enforced")
        return Outcome(
            status=OutcomeStatus.FAIL,
            failure_reason=f"unexpectedly allowed concurrent non-parallel handler execution: {errors}",
        )


class _SharedRefSeedHandler:
    def __init__(self, shared_ref):
        self.shared_ref = shared_ref

    def run(self, runtime):
        return Outcome(
            status=OutcomeStatus.SUCCESS,
            context_updates={"shared_ref": self.shared_ref},
        )


class _SharedRefIsolationChecker:
    def __init__(self, marker: str, barrier: threading.Barrier):
        self.marker = marker
        self.barrier = barrier

    def run(self, runtime):
        shared_ref = runtime.context.get("shared_ref", {})
        if not isinstance(shared_ref, dict):
            return Outcome(status=OutcomeStatus.FAIL, failure_reason="missing shared_ref dict")
        markers = shared_ref.setdefault("markers", [])
        if not isinstance(markers, list):
            return Outcome(status=OutcomeStatus.FAIL, failure_reason="shared_ref.markers must be list")
        markers.append(self.marker)
        try:
            self.barrier.wait(timeout=1.0)
        except threading.BrokenBarrierError:
            return Outcome(status=OutcomeStatus.FAIL, failure_reason="checker synchronization failed")

        if markers == [self.marker]:
            return Outcome(status=OutcomeStatus.SUCCESS, notes=f"isolated:{self.marker}")
        return Outcome(
            status=OutcomeStatus.FAIL,
            failure_reason=f"context leaked markers for {self.marker}: {markers}",
        )


class _FalseyInterviewer(Interviewer):
    def __bool__(self) -> bool:
        return False

    def ask(self, question: Question) -> Answer:
        return Answer(selected_values=["Fix"])


class TestBuiltInHandlers:
    def test_codergen_backend_protocol_is_structural(self):
        class _AltBackend:
            def run(self, node_id: str, prompt: str, context: Context, *, timeout=None) -> bool:
                return True

        assert isinstance(_StubBackend(), CodergenBackend)
        assert isinstance(_AltBackend(), CodergenBackend)

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

    def test_house_shape_resolves_and_executes_with_default_registry(self):
        graph = parse_dot(
            """
            digraph G {
                manager [shape=house]
            }
            """
        )
        registry = build_default_registry(codergen_backend=_StubBackend())
        runner = HandlerRunner(graph, registry)

        assert registry.resolve_handler_type(graph.nodes["manager"]) == "stack.manager_loop"
        outcome = runner("manager", "", Context())
        assert outcome.status == OutcomeStatus.FAIL
        assert "not implemented" in outcome.failure_reason

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
