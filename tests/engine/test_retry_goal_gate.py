import unittest

from attractor.dsl import parse_dot
from attractor.engine.context import Context
from attractor.engine.executor import PipelineExecutor
from attractor.engine.outcome import Outcome, OutcomeStatus


class TestRetryAndGoalGate(unittest.TestCase):
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
        self.assertEqual("success", result.status)
        self.assertEqual(2, calls["task"])

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
        self.assertEqual("success", result.status)
        self.assertEqual(2, calls["task"])
        self.assertIn("fix", result.completed_nodes)

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
        self.assertEqual("success", result.status)
        self.assertIn("fix", result.completed_nodes)

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
        self.assertEqual("success", result.status)
        self.assertEqual(2, calls["implement"])


if __name__ == "__main__":
    unittest.main()
