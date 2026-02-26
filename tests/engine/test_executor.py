import pytest

from attractor.dsl import parse_dot
from attractor.engine.context import Context
from attractor.engine.executor import PipelineExecutor
from attractor.engine.outcome import Outcome, OutcomeStatus


class TestExecutor:
    def test_shape_start_takes_precedence_over_start_id_fallback(self):
        graph = parse_dot(
            """
            digraph G {
                entry [shape=Mdiamond]
                start [shape=box]
                work [shape=box]
                done [shape=Msquare]

                entry -> work
                start -> work
                work -> done
            }
            """
        )

        result = PipelineExecutor(graph, lambda *_: Outcome(status=OutcomeStatus.SUCCESS)).run(Context())

        assert result.status == "success"
        assert result.route_trace == ["entry", "work", "done"]

    def test_shape_exit_takes_precedence_over_end_id_fallback(self):
        graph = parse_dot(
            """
            digraph G {
                start [shape=Mdiamond]
                review [shape=box]
                end [shape=box]
                done [shape=Msquare]

                start -> review
                review -> end
                end -> done
            }
            """
        )

        result = PipelineExecutor(graph, lambda *_: Outcome(status=OutcomeStatus.SUCCESS)).run(Context())

        assert result.status == "success"
        assert result.current_node == "done"
        assert result.route_trace == ["start", "review", "end", "done"]

    def test_replay_with_identical_outcomes_and_context_has_identical_routing(self):
        graph = parse_dot(
            """
            digraph G {
                start [shape=Mdiamond]
                plan [shape=box]
                approve [shape=box]
                revise [shape=box]
                done [shape=Msquare]

                start -> plan
                plan -> approve [condition="outcome=success", label="Approve", weight=10]
                plan -> revise [condition="outcome=fail", label="Revise", weight=10]
                approve -> done
                revise -> done
            }
            """
        )

        calls = {"plan": 0}

        def runner(node_id: str, prompt: str, context: Context) -> Outcome:
            if node_id == "plan":
                calls["plan"] += 1
                return Outcome(status=OutcomeStatus.SUCCESS)
            return Outcome(status=OutcomeStatus.SUCCESS)

        first = PipelineExecutor(graph, runner).run(Context(values={"seed": "same"}))
        second = PipelineExecutor(graph, runner).run(Context(values={"seed": "same"}))

        assert first.route_trace == ["start", "plan", "approve", "done"]
        assert second.route_trace == first.route_trace

    def test_executor_resolves_start_and_branches(self):
        graph = parse_dot(
            """
            digraph G {
                start [shape=Mdiamond]
                plan [shape=box, prompt="plan"]
                fix [shape=box, prompt="fix"]
                done [shape=Msquare]

                start -> plan
                plan -> done [condition="outcome=success"]
                plan -> fix [condition="outcome=fail", label="Fix"]
                fix -> done
            }
            """
        )

        calls = []

        def runner(node_id: str, prompt: str, context: Context) -> Outcome:
            calls.append((node_id, prompt))
            if node_id == "start":
                return Outcome(status=OutcomeStatus.SUCCESS)
            if node_id == "plan":
                return Outcome(status=OutcomeStatus.FAIL, context_updates={"needs_fix": "true"})
            if node_id == "fix":
                return Outcome(status=OutcomeStatus.SUCCESS)
            return Outcome(status=OutcomeStatus.SUCCESS)

        result = PipelineExecutor(graph, runner).run(Context())

        assert result.status == "success"
        assert result.current_node == "done"
        assert result.completed_nodes == ["start", "plan", "fix"]
        assert result.context.get("needs_fix") == "true"
        assert calls[1][1] == "plan"

    def test_executor_requires_single_start(self):
        graph = parse_dot(
            """
            digraph G {
                start [shape=Mdiamond]
                start2 [shape=Mdiamond]
                done [shape=Msquare]
                start -> done
                start2 -> done
            }
            """
        )

        with pytest.raises(RuntimeError):
            PipelineExecutor(graph, lambda *_: Outcome(status=OutcomeStatus.SUCCESS)).run(Context())
