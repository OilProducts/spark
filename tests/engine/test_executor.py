import pytest

from attractor.dsl import parse_dot
from attractor.engine.context import Context
from attractor.engine.executor import PipelineExecutor
from attractor.engine.outcome import Outcome, OutcomeStatus


class TestExecutor:
    def test_executor_resolves_runtime_fidelity_from_graph_default(self):
        graph = parse_dot(
            """
            digraph G {
                graph [default_fidelity="summary:medium"]
                start [shape=Mdiamond]
                work [shape=box]
                done [shape=Msquare]
                start -> work
                work -> done
            }
            """
        )
        seen_fidelity: list[str] = []

        def runner(node_id: str, prompt: str, context: Context) -> Outcome:
            seen_fidelity.append(str(context.get("_attractor.runtime.fidelity", "")))
            return Outcome(status=OutcomeStatus.SUCCESS)

        result = PipelineExecutor(graph, runner).run(Context())

        assert result.status == "success"
        assert seen_fidelity == ["summary:medium", "summary:medium"]

    def test_executor_mirrors_graph_goal_into_context(self):
        graph = parse_dot(
            """
            digraph G {
                graph [goal="Ship docs"]
                start [shape=Mdiamond]
                work [shape=box]
                done [shape=Msquare]
                start -> work
                work -> done
            }
            """
        )
        seen_goals: list[object] = []

        def runner(node_id: str, prompt: str, context: Context) -> Outcome:
            seen_goals.append(context.get("graph.goal"))
            return Outcome(status=OutcomeStatus.SUCCESS)

        result = PipelineExecutor(graph, runner).run(Context())

        assert result.status == "success"
        assert result.context["graph.goal"] == "Ship docs"
        assert seen_goals == ["Ship docs", "Ship docs"]

    def test_executor_emits_typed_runtime_events_for_ui_consumers(self):
        graph = parse_dot(
            """
            digraph G {
                start [shape=Mdiamond]
                work [shape=box]
                done [shape=Msquare]
                start -> work
                work -> done
            }
            """
        )
        events = []

        def runner(node_id: str, prompt: str, context: Context) -> Outcome:
            return Outcome(status=OutcomeStatus.SUCCESS)

        result = PipelineExecutor(graph, runner, on_event=events.append).run(Context())

        event_types = [event["type"] for event in events]
        assert result.status == "success"
        assert event_types[0] == "PipelineStarted"
        assert event_types[-1] == "PipelineCompleted"
        assert event_types.count("StageStarted") == 2
        assert event_types.count("StageCompleted") == 2
        assert event_types.count("CheckpointSaved") >= 1
        assert all(isinstance(event, dict) and "type" in event for event in events)

    def test_executor_emits_stage_retrying_event(self):
        graph = parse_dot(
            """
            digraph G {
                start [shape=Mdiamond]
                work [shape=box, max_retries=2]
                done [shape=Msquare]
                start -> work
                work -> done
            }
            """
        )
        events = []
        calls = {"work": 0}

        def runner(node_id: str, prompt: str, context: Context) -> Outcome:
            if node_id == "work":
                calls["work"] += 1
                if calls["work"] == 1:
                    return Outcome(status=OutcomeStatus.RETRY, failure_reason="retryable")
            return Outcome(status=OutcomeStatus.SUCCESS)

        result = PipelineExecutor(graph, runner, on_event=events.append).run(Context())

        retry_events = [event for event in events if event["type"] == "StageRetrying"]
        assert result.status == "success"
        assert len(retry_events) == 1
        assert retry_events[0]["node_id"] == "work"
        assert retry_events[0]["attempt"] == 1

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

    def test_prompt_falls_back_to_label_for_llm_stage_only(self):
        graph = parse_dot(
            """
            digraph G {
                start [shape=Mdiamond]
                plan [shape=box, label="Plan from label"]
                gate [shape=hexagon, label="Human label"]
                done [shape=Msquare]

                start -> plan
                plan -> gate
                gate -> done
            }
            """
        )

        seen_prompts: dict[str, str] = {}

        def runner(node_id: str, prompt: str, context: Context) -> Outcome:
            seen_prompts[node_id] = prompt
            return Outcome(status=OutcomeStatus.SUCCESS)

        result = PipelineExecutor(graph, runner).run(Context())

        assert result.status == "success"
        assert seen_prompts["plan"] == "Plan from label"
        assert seen_prompts["gate"] == ""

    def test_conditional_node_routes_using_prior_stage_outcome(self):
        graph = parse_dot(
            """
            digraph G {
                start [shape=Mdiamond]
                plan [shape=box]
                gate [shape=diamond]
                fix [shape=box]
                done [shape=Msquare]

                start -> plan
                plan -> gate
                gate -> done [condition="outcome=success"]
                gate -> fix [condition="outcome=fail"]
                fix -> done
            }
            """
        )

        def runner(node_id: str, prompt: str, context: Context) -> Outcome:
            if node_id == "plan":
                return Outcome(status=OutcomeStatus.FAIL)
            return Outcome(status=OutcomeStatus.SUCCESS)

        result = PipelineExecutor(graph, runner).run(Context())

        assert result.status == "success"
        assert result.route_trace == ["start", "plan", "gate", "fix", "done"]

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
