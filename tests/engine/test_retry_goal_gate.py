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

    def test_allow_partial_converts_retry_exhaustion_to_partial_success(self):
        graph = parse_dot(
            """
            digraph G {
                start [shape=Mdiamond]
                task [shape=box, max_retries=1, allow_partial=true]
                done [shape=Msquare]
                start -> task
                task -> done [condition="outcome=partial_success"]
            }
            """
        )

        calls = {"task": 0}

        def runner(node_id: str, prompt: str, context: Context) -> Outcome:
            if node_id == "task":
                calls["task"] += 1
                return Outcome(status=OutcomeStatus.RETRY, failure_reason="stuck")
            return Outcome(status=OutcomeStatus.SUCCESS)

        result = PipelineExecutor(graph, runner).run(Context())
        assert result.status == "success"
        assert calls["task"] == 2
        assert result.node_outcomes["task"].status is OutcomeStatus.PARTIAL_SUCCESS

    def test_allow_partial_converts_retryable_fail_exhaustion_to_partial_success(self):
        graph = parse_dot(
            """
            digraph G {
                start [shape=Mdiamond]
                task [shape=box, max_retries=1, allow_partial=true]
                done [shape=Msquare]
                start -> task
                task -> done [condition="outcome=partial_success"]
            }
            """
        )

        calls = {"task": 0}

        def runner(node_id: str, prompt: str, context: Context) -> Outcome:
            if node_id == "task":
                calls["task"] += 1
                return Outcome(
                    status=OutcomeStatus.FAIL,
                    failure_reason="transient dependency",
                    retryable=True,
                )
            return Outcome(status=OutcomeStatus.SUCCESS)

        result = PipelineExecutor(graph, runner).run(Context())

        assert result.status == "success"
        assert calls["task"] == 2
        assert result.node_outcomes["task"].status is OutcomeStatus.PARTIAL_SUCCESS

    def test_retry_counter_resets_after_success_before_revisiting_node(self):
        graph = parse_dot(
            """
            digraph G {
                start [shape=Mdiamond]
                task [shape=box, max_retries=1]
                requeue [shape=box]
                done [shape=Msquare]

                start -> task
                task -> requeue [condition="context.second_pass=true", weight=10]
                task -> done [condition="outcome=success"]
                requeue -> task
            }
            """
        )

        calls = {"task": 0}

        def runner(node_id: str, prompt: str, context: Context) -> Outcome:
            if node_id == "task":
                calls["task"] += 1
                if calls["task"] in {1, 3}:
                    return Outcome(status=OutcomeStatus.RETRY, failure_reason="transient")
                if calls["task"] == 2:
                    return Outcome(
                        status=OutcomeStatus.SUCCESS,
                        context_updates={"second_pass": "true"},
                    )
                return Outcome(status=OutcomeStatus.SUCCESS)
            if node_id == "requeue":
                return Outcome(status=OutcomeStatus.SUCCESS, context_updates={"second_pass": "false"})
            return Outcome(status=OutcomeStatus.SUCCESS)

        result = PipelineExecutor(graph, runner).run(Context())

        assert result.status == "success"
        assert calls["task"] == 4
        assert result.route_trace == ["start", "task", "requeue", "task", "done"]

    def test_retry_counter_resets_after_partial_success_before_revisiting_node(self):
        graph = parse_dot(
            """
            digraph G {
                start [shape=Mdiamond]
                task [shape=box, max_retries=1, allow_partial=true]
                requeue [shape=box]
                done [shape=Msquare]

                start -> task
                task -> requeue [condition="context.second_pass=true", weight=10]
                task -> done [condition="outcome=partial_success"]
                requeue -> task
            }
            """
        )

        calls = {"task": 0}

        def runner(node_id: str, prompt: str, context: Context) -> Outcome:
            if node_id == "task":
                calls["task"] += 1
                if calls["task"] == 2:
                    return Outcome(
                        status=OutcomeStatus.RETRY,
                        failure_reason="stuck",
                        context_updates={"second_pass": "true"},
                    )
                return Outcome(status=OutcomeStatus.RETRY, failure_reason="stuck")
            if node_id == "requeue":
                return Outcome(status=OutcomeStatus.SUCCESS, context_updates={"second_pass": "false"})
            return Outcome(status=OutcomeStatus.SUCCESS)

        result = PipelineExecutor(graph, runner).run(Context())

        assert result.status == "success"
        assert calls["task"] == 4
        assert result.route_trace == ["start", "task", "requeue", "task", "done"]
        assert result.node_outcomes["task"].status is OutcomeStatus.PARTIAL_SUCCESS

    def test_retry_exhaustion_uses_max_retries_plus_one_attempts_then_routes_fail(self):
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
                return Outcome(status=OutcomeStatus.RETRY, failure_reason="temporary")
            return Outcome(status=OutcomeStatus.SUCCESS)

        result = PipelineExecutor(graph, runner).run(Context())

        assert result.status == "success"
        assert calls["task"] == 3
        assert result.route_trace == ["start", "task", "fix", "done"]
        assert result.node_outcomes["task"].status is OutcomeStatus.FAIL
        assert result.node_outcomes["task"].failure_reason == "max retries exceeded"

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

    def test_failure_routing_uses_fallback_retry_target_before_unconditional_edge(self):
        graph = parse_dot(
            """
            digraph G {
                start [shape=Mdiamond]
                task [shape=box, max_retries=0, retry_target="missing", fallback_retry_target="fix"]
                fix [shape=box]
                done [shape=Msquare]
                start -> task
                task -> done
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
        assert result.route_trace == ["start", "task", "fix", "done"]

    def test_failure_routing_prefers_retry_target_over_fallback_retry_target(self):
        graph = parse_dot(
            """
            digraph G {
                start [shape=Mdiamond]
                task [shape=box, max_retries=0, retry_target=" fix ", fallback_retry_target="fallback"]
                fix [shape=box]
                fallback [shape=box]
                done [shape=Msquare]
                start -> task
                fix -> done
                fallback -> done
            }
            """
        )

        def runner(node_id: str, prompt: str, context: Context) -> Outcome:
            if node_id == "task":
                return Outcome(status=OutcomeStatus.FAIL, failure_reason="permanent")
            return Outcome(status=OutcomeStatus.SUCCESS)

        result = PipelineExecutor(graph, runner).run(Context())
        assert result.status == "success"
        assert result.route_trace == ["start", "task", "fix", "done"]

    def test_failure_routing_prefers_outcome_fail_edge_over_other_true_conditions(self):
        graph = parse_dot(
            """
            digraph G {
                start [shape=Mdiamond]
                task [shape=box, max_retries=0, retry_target="fix"]
                review [shape=box]
                fix [shape=box]
                done [shape=Msquare]
                start -> task
                task -> done [condition="outcome=fail"]
                task -> review [condition="context.force_route=true", weight=10]
                review -> done
                fix -> done
            }
            """
        )

        context = Context(values={"force_route": "true"})

        def runner(node_id: str, prompt: str, ctx: Context) -> Outcome:
            if node_id == "task":
                return Outcome(status=OutcomeStatus.FAIL, failure_reason="permanent")
            return Outcome(status=OutcomeStatus.SUCCESS)

        result = PipelineExecutor(graph, runner).run(context)
        assert result.status == "success"
        assert result.route_trace == ["start", "task", "done"]

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

    def test_goal_gate_without_recorded_outcome_allows_exit(self):
        graph = parse_dot(
            """
            digraph G {
                start [shape=Mdiamond]
                plan [shape=box]
                implement [shape=box, goal_gate=true]
                done [shape=Msquare]

                start -> plan
                plan -> done
            }
            """
        )

        def runner(node_id: str, prompt: str, context: Context) -> Outcome:
            return Outcome(status=OutcomeStatus.SUCCESS)

        result = PipelineExecutor(graph, runner).run(Context())
        assert result.status == "success"
        assert result.route_trace == ["start", "plan", "done"]

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

    def test_goal_gate_failure_without_retry_target_fails_at_exit(self):
        graph = parse_dot(
            """
            digraph G {
                start [shape=Mdiamond]
                implement [shape=box, goal_gate=true, max_retries=0]
                done [shape=Msquare]

                start -> implement
                implement -> done
            }
            """
        )

        def runner(node_id: str, prompt: str, context: Context) -> Outcome:
            if node_id == "implement":
                return Outcome(status=OutcomeStatus.FAIL, failure_reason="needs fix")
            return Outcome(status=OutcomeStatus.SUCCESS)

        result = PipelineExecutor(graph, runner).run(Context())
        assert result.status == "fail"
        assert result.route_trace == ["start", "implement", "done"]
        assert result.failure_reason == "Goal gate unsatisfied and no retry target"

    def test_goal_gate_failure_with_only_invalid_retry_targets_fails_at_exit(self):
        graph = parse_dot(
            """
            digraph G {
                graph [retry_target="missing_graph_retry", fallback_retry_target="missing_graph_fallback"]
                start [shape=Mdiamond]
                implement [
                    shape=box,
                    goal_gate=true,
                    max_retries=0,
                    retry_target="missing_node_retry",
                    fallback_retry_target="missing_node_fallback"
                ]
                done [shape=Msquare]

                start -> implement
                implement -> done
            }
            """
        )

        def runner(node_id: str, prompt: str, context: Context) -> Outcome:
            if node_id == "implement":
                return Outcome(status=OutcomeStatus.FAIL, failure_reason="needs fix")
            return Outcome(status=OutcomeStatus.SUCCESS)

        result = PipelineExecutor(graph, runner).run(Context())
        assert result.status == "fail"
        assert result.route_trace == ["start", "implement", "done"]
        assert result.failure_reason == "Goal gate unsatisfied and no retry target"

    def test_goal_gate_failure_without_retry_target_fails_at_exit_in_run_from(self):
        graph = parse_dot(
            """
            digraph G {
                start [shape=Mdiamond]
                implement [shape=box, goal_gate=true, max_retries=0]
                done [shape=Msquare]

                start -> implement
                implement -> done
            }
            """
        )

        def runner(node_id: str, prompt: str, context: Context) -> Outcome:
            if node_id == "implement":
                return Outcome(status=OutcomeStatus.FAIL, failure_reason="needs fix")
            return Outcome(status=OutcomeStatus.SUCCESS)

        result = PipelineExecutor(graph, runner).run_from("start", Context())
        assert result.status == "fail"
        assert result.route_trace == ["start", "implement", "done"]
        assert result.failure_reason == "Goal gate unsatisfied and no retry target"

    def test_goal_gate_enforced_when_terminal_is_in_run_from_stop_nodes(self):
        graph = parse_dot(
            """
            digraph G {
                start [shape=Mdiamond]
                implement [shape=box, goal_gate=true, max_retries=0]
                done [shape=Msquare]

                start -> implement
                implement -> done
            }
            """
        )

        def runner(node_id: str, prompt: str, context: Context) -> Outcome:
            if node_id == "implement":
                return Outcome(status=OutcomeStatus.FAIL, failure_reason="needs fix")
            return Outcome(status=OutcomeStatus.SUCCESS)

        result = PipelineExecutor(graph, runner).run_from("start", Context(), stop_nodes={"done"})
        assert result.status == "fail"
        assert result.route_trace == ["start", "implement", "done"]
        assert result.failure_reason == "Goal gate unsatisfied and no retry target"

    def test_run_from_goal_gate_recovery_checkpoints_retry_target_before_stage_execution(self):
        graph = parse_dot(
            """
            digraph G {
                graph [retry_target="fix"]
                start [shape=Mdiamond]
                implement [shape=box, goal_gate=true, max_retries=0]
                fix [shape=box]
                done [shape=Msquare]

                start -> implement
                implement -> done [condition="outcome=fail"]
                implement -> done [condition="outcome=success"]
                fix -> implement
            }
            """
        )

        events = []
        calls = {"implement": 0}

        def runner(node_id: str, prompt: str, context: Context) -> Outcome:
            if node_id == "implement":
                calls["implement"] += 1
                if calls["implement"] == 1:
                    return Outcome(status=OutcomeStatus.FAIL, failure_reason="needs fix")
            return Outcome(status=OutcomeStatus.SUCCESS)

        result = PipelineExecutor(graph, runner, on_event=events.append).run_from("start", Context())

        assert result.status == "success"
        stage_started_fix_index = next(
            i for i, event in enumerate(events) if event["type"] == "StageStarted" and event.get("node_id") == "fix"
        )
        checkpoint_fix_indices = [
            i
            for i, event in enumerate(events)
            if event["type"] == "CheckpointSaved" and event.get("node_id") == "fix"
        ]
        assert checkpoint_fix_indices
        assert any(index < stage_started_fix_index for index in checkpoint_fix_indices)

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

        with pytest.raises(RuntimeError, match="Stage 'task' failed with no outgoing fail edge"):
            PipelineExecutor(graph, runner).run(Context())

    def test_handler_exception_retries_then_persists_fail_outcome_for_fail_routing(self):
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
                raise RuntimeError("transient backend outage")
            return Outcome(status=OutcomeStatus.SUCCESS)

        result = PipelineExecutor(graph, runner).run(Context())

        assert result.status == "success"
        assert calls["task"] == 2
        assert result.node_outcomes["task"].status is OutcomeStatus.FAIL
        assert result.node_outcomes["task"].failure_reason == "transient backend outage"
        assert "fix" in result.completed_nodes

    def test_non_retryable_exception_does_not_retry_and_routes_fail_immediately(self):
        graph = parse_dot(
            """
            digraph G {
                start [shape=Mdiamond]
                task [shape=box, max_retries=3]
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
                raise RuntimeError("401 unauthorized")
            return Outcome(status=OutcomeStatus.SUCCESS)

        result = PipelineExecutor(graph, runner).run(Context())

        assert result.status == "success"
        assert calls["task"] == 1
        assert result.route_trace == ["start", "task", "fix", "done"]
        assert result.node_outcomes["task"].failure_reason == "401 unauthorized"
