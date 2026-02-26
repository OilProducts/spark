import pytest

from attractor.dsl import parse_dot
from attractor.engine.context import Context
from attractor.engine.executor import PipelineExecutor
from attractor.engine.outcome import Outcome, OutcomeStatus
from attractor.transforms import AttributeDefaultsTransform


class TestRetryAndGoalGate:
    def test_graph_default_max_retry_applies_when_node_omits_max_retries(self):
        graph = parse_dot(
            """
            digraph G {
                graph [default_max_retry=2]
                start [shape=Mdiamond]
                task [shape=box]
                done [shape=Msquare]
                start -> task
                task -> done
            }
            """
        )
        graph = AttributeDefaultsTransform().apply(graph)

        calls = {"task": 0}

        def runner(node_id: str, prompt: str, context: Context) -> Outcome:
            if node_id == "task":
                calls["task"] += 1
                if calls["task"] < 3:
                    return Outcome(status=OutcomeStatus.RETRY, failure_reason="retryable")
            return Outcome(status=OutcomeStatus.SUCCESS)

        result = PipelineExecutor(graph, runner).run(Context())

        assert result.status == "success"
        assert calls["task"] == 3

    def test_retry_status_retries_same_node(self):
        graph = parse_dot(
            """
            digraph G {
                start [shape=Mdiamond]
                task [shape=box, max_retries=2]
                done [shape=Msquare]
                start -> task
                task -> done [condition="outcome=success"]
            }
            """
        )

        calls = {"task": 0}

        def runner(node_id: str, prompt: str, context: Context) -> Outcome:
            if node_id == "task":
                calls["task"] += 1
                if calls["task"] == 1:
                    return Outcome(status=OutcomeStatus.RETRY, failure_reason="transient")
            return Outcome(status=OutcomeStatus.SUCCESS)

        result = PipelineExecutor(graph, runner).run(Context())
        assert result.status == "success"
        assert calls["task"] == 2

    def test_fail_edge_after_retry_exhaustion(self):
        graph = parse_dot(
            """
            digraph G {
                start [shape=Mdiamond]
                task [shape=box, max_retries=1]
                fix [shape=box]
                done [shape=Msquare]
                start -> task
                task -> done [condition="outcome=success"]
                task -> fix [condition="outcome=fail"]
                fix -> done
            }
            """
        )

        calls = {"task": 0}

        def runner(node_id: str, prompt: str, context: Context) -> Outcome:
            if node_id == "task":
                calls["task"] += 1
                return Outcome(status=OutcomeStatus.FAIL, failure_reason="network timeout")
            return Outcome(status=OutcomeStatus.SUCCESS)

        result = PipelineExecutor(graph, runner).run(Context())
        assert result.status == "success"
        assert calls["task"] == 2
        assert "fix" in result.completed_nodes

    def test_fail_status_retries_for_max_retries_budget(self):
        graph = parse_dot(
            """
            digraph G {
                start [shape=Mdiamond]
                task [shape=box, max_retries=2]
                fix [shape=box]
                done [shape=Msquare]
                start -> task
                task -> done [condition="outcome=success"]
                task -> fix [condition="outcome=fail"]
                fix -> done
            }
            """
        )

        calls = {"task": 0}

        def runner(node_id: str, prompt: str, context: Context) -> Outcome:
            if node_id == "task":
                calls["task"] += 1
                return Outcome(status=OutcomeStatus.FAIL, failure_reason="permanent")
            return Outcome(status=OutcomeStatus.SUCCESS)

        result = PipelineExecutor(graph, runner).run(Context())
        assert result.status == "success"
        assert calls["task"] == 3
        assert "fix" in result.completed_nodes

    def test_failure_routing_uses_retry_target(self):
        graph = parse_dot(
            """
            digraph G {
                start [shape=Mdiamond]
                task [shape=box, max_retries=0, retry_target="fix"]
                fix [shape=box]
                done [shape=Msquare]
                start -> task
                task -> done [condition="outcome=success"]
                fix -> done
            }
            """
        )

        def runner(node_id: str, prompt: str, context: Context) -> Outcome:
            if node_id == "task":
                return Outcome(status=OutcomeStatus.FAIL, failure_reason="permanent")
            return Outcome(status=OutcomeStatus.SUCCESS)

        result = PipelineExecutor(graph, runner).run(Context())
        assert result.status == "success"
        assert "fix" in result.completed_nodes

    def test_goal_gate_enforced_at_exit(self):
        graph = parse_dot(
            """
            digraph G {
                start [shape=Mdiamond]
                implement [shape=box, goal_gate=true, retry_target="implement", max_retries=0]
                done [shape=Msquare]

                start -> implement
                implement -> done
            }
            """
        )

        calls = {"implement": 0}

        def runner(node_id: str, prompt: str, context: Context) -> Outcome:
            if node_id == "implement":
                calls["implement"] += 1
                if calls["implement"] == 1:
                    return Outcome(status=OutcomeStatus.FAIL, failure_reason="needs fix")
            return Outcome(status=OutcomeStatus.SUCCESS)

        result = PipelineExecutor(graph, runner).run(Context())
        assert result.status == "success"
        assert calls["implement"] == 2

    def test_goal_gate_allows_partial_success_at_exit(self):
        graph = parse_dot(
            """
            digraph G {
                start [shape=Mdiamond]
                implement [shape=box, goal_gate=true]
                done [shape=Msquare]

                start -> implement
                implement -> done
            }
            """
        )

        def runner(node_id: str, prompt: str, context: Context) -> Outcome:
            if node_id == "implement":
                return Outcome(status=OutcomeStatus.PARTIAL_SUCCESS, notes="good enough")
            return Outcome(status=OutcomeStatus.SUCCESS)

        result = PipelineExecutor(graph, runner).run(Context())
        assert result.status == "success"

    def test_goal_gate_recovery_uses_graph_level_retry_target(self):
        graph = parse_dot(
            """
            digraph G {
                graph [retry_target="implement"]
                start [shape=Mdiamond]
                implement [shape=box, goal_gate=true, max_retries=0]
                done [shape=Msquare]

                start -> implement
                implement -> done
            }
            """
        )

        calls = {"implement": 0}

        def runner(node_id: str, prompt: str, context: Context) -> Outcome:
            if node_id == "implement":
                calls["implement"] += 1
                if calls["implement"] == 1:
                    return Outcome(status=OutcomeStatus.FAIL, failure_reason="needs fix")
            return Outcome(status=OutcomeStatus.SUCCESS)

        result = PipelineExecutor(graph, runner).run(Context())
        assert result.status == "success"
        assert calls["implement"] == 2

    def test_goal_gate_recovery_uses_graph_level_fallback_retry_target(self):
        graph = parse_dot(
            """
            digraph G {
                graph [retry_target="missing", fallback_retry_target="implement"]
                start [shape=Mdiamond]
                implement [shape=box, goal_gate=true, max_retries=0]
                done [shape=Msquare]

                start -> implement
                implement -> done
            }
            """
        )

        calls = {"implement": 0}

        def runner(node_id: str, prompt: str, context: Context) -> Outcome:
            if node_id == "implement":
                calls["implement"] += 1
                if calls["implement"] == 1:
                    return Outcome(status=OutcomeStatus.FAIL, failure_reason="needs fix")
            return Outcome(status=OutcomeStatus.SUCCESS)

        result = PipelineExecutor(graph, runner).run(Context())
        assert result.status == "success"
        assert calls["implement"] == 2

    def test_non_goal_gate_fail_routing_ignores_graph_level_retry_target(self):
        graph = parse_dot(
            """
            digraph G {
                graph [retry_target="fix"]
                start [shape=Mdiamond]
                task [shape=box, max_retries=0]
                fix [shape=box]
                done [shape=Msquare]
                start -> task
                task -> done [condition="outcome=success"]
                fix -> done
            }
            """
        )

        def runner(node_id: str, prompt: str, context: Context) -> Outcome:
            if node_id == "task":
                return Outcome(status=OutcomeStatus.FAIL, failure_reason="permanent")
            return Outcome(status=OutcomeStatus.SUCCESS)

        with pytest.raises(RuntimeError, match="Stage 'task' has no eligible outgoing edge"):
            PipelineExecutor(graph, runner).run(Context())
