import pytest
from pathlib import Path
import tempfile

from attractor.dsl import parse_dot
from attractor.engine.context import Context
from attractor.engine.executor import PipelineExecutor
from attractor.engine.outcome import Outcome, OutcomeStatus

BRANCHING_CONDITION_FIXTURE = Path(__file__).resolve().parents[1] / "fixtures" / "branching_condition_workflow.dot"


class TestExecutor:
    @pytest.mark.parametrize(
        ("validate_outcomes", "expected_route"),
        [
            (
                [OutcomeStatus.SUCCESS],
                ["start", "plan", "implement", "validate", "gate", "exit"],
            ),
            (
                [OutcomeStatus.PARTIAL_SUCCESS, OutcomeStatus.SUCCESS],
                ["start", "plan", "implement", "validate", "gate", "implement", "validate", "gate", "exit"],
            ),
        ],
    )
    def test_executes_branching_condition_workflow_fixture(self, validate_outcomes, expected_route):
        graph = parse_dot(BRANCHING_CONDITION_FIXTURE.read_text(encoding="utf-8"))
        validate_attempts = {"count": 0}

        def runner(node_id: str, prompt: str, context: Context) -> Outcome:
            if node_id == "validate":
                idx = validate_attempts["count"]
                validate_attempts["count"] += 1
                status = validate_outcomes[min(idx, len(validate_outcomes) - 1)]
                return Outcome(status=status)
            return Outcome(status=OutcomeStatus.SUCCESS)

        result = PipelineExecutor(graph, runner).run(Context())

        assert result.status == "success"
        assert result.route_trace == expected_route

    def test_loop_restart_relaunches_from_edge_target_with_fresh_result_state(self):
        graph = parse_dot(
            """
            digraph G {
                start [shape=Mdiamond]
                loop [shape=box]
                work [shape=box]
                done [shape=Msquare]

                start -> loop
                loop -> work [loop_restart=true]
                work -> done
            }
            """
        )

        calls: list[str] = []

        def runner(node_id: str, prompt: str, context: Context) -> Outcome:
            calls.append(node_id)
            return Outcome(status=OutcomeStatus.SUCCESS)

        result = PipelineExecutor(graph, runner).run(Context())

        assert result.status == "success"
        assert calls == ["start", "loop", "work"]
        assert result.current_node == "done"
        assert result.route_trace == ["work", "done"]
        assert result.completed_nodes == ["work"]
        assert set(result.node_outcomes.keys()) == {"work"}

    def test_loop_restart_switches_to_fresh_logs_directory(self):
        graph = parse_dot(
            """
            digraph G {
                start [shape=Mdiamond]
                loop [shape=box]
                work [shape=box]
                done [shape=Msquare]

                start -> loop
                loop -> work [loop_restart=true]
                work -> done
            }
            """
        )

        with tempfile.TemporaryDirectory() as tmp:
            logs_root = Path(tmp) / "logs"

            def runner(node_id: str, prompt: str, context: Context) -> Outcome:
                return Outcome(status=OutcomeStatus.SUCCESS, notes=node_id)

            result = PipelineExecutor(graph, runner, logs_root=str(logs_root)).run(Context())

            assert result.status == "success"
            assert (logs_root / "start" / "status.json").exists()
            assert (logs_root / "loop" / "status.json").exists()

            restart_logs_root = logs_root.parent / "logs.restart-1"
            assert (restart_logs_root / "work" / "status.json").exists()
            assert not (restart_logs_root / "start").exists()
            assert not (restart_logs_root / "loop").exists()

    def test_executor_resolves_runtime_thread_id_from_node_attr_for_full_fidelity(self):
        graph = parse_dot(
            """
            digraph G {
                graph [default_fidelity="full"]
                start [shape=Mdiamond]
                work [shape=box, thread_id="work-thread"]
                done [shape=Msquare]
                start -> work
                work -> done
            }
            """
        )
        seen_thread_ids: list[str] = []

        def runner(node_id: str, prompt: str, context: Context) -> Outcome:
            seen_thread_ids.append(str(context.get("_attractor.runtime.thread_id", "")))
            return Outcome(status=OutcomeStatus.SUCCESS)

        result = PipelineExecutor(graph, runner).run(Context())

        assert result.status == "success"
        assert seen_thread_ids == ["start", "work-thread"]

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

    def test_executor_node_fidelity_overrides_graph_default(self):
        graph = parse_dot(
            """
            digraph G {
                graph [default_fidelity="summary:medium"]
                start [shape=Mdiamond]
                work [shape=box, fidelity="full"]
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
        assert seen_fidelity == ["summary:medium", "full"]

    def test_executor_edge_fidelity_overrides_node_and_graph_defaults(self):
        graph = parse_dot(
            """
            digraph G {
                graph [default_fidelity="summary:medium"]
                start [shape=Mdiamond]
                work [shape=box, fidelity="compact"]
                done [shape=Msquare]
                start -> work [fidelity="full"]
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
        assert seen_fidelity == ["summary:medium", "full"]

    def test_executor_edge_thread_id_overrides_target_node_thread_id(self):
        graph = parse_dot(
            """
            digraph G {
                graph [default_fidelity="full"]
                start [shape=Mdiamond]
                work [shape=box, thread_id="work-thread"]
                done [shape=Msquare]
                start -> work [thread_id="edge-thread"]
                work -> done
            }
            """
        )
        seen_thread_ids: list[str] = []

        def runner(node_id: str, prompt: str, context: Context) -> Outcome:
            seen_thread_ids.append(str(context.get("_attractor.runtime.thread_id", "")))
            return Outcome(status=OutcomeStatus.SUCCESS)

        result = PipelineExecutor(graph, runner).run(Context())

        assert result.status == "success"
        assert seen_thread_ids == ["start", "edge-thread"]

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

    def test_executor_applies_context_updates_before_outcome_and_preferred_label(self):
        graph = parse_dot(
            """
            digraph G {
                start [shape=Mdiamond]
                plan [shape=box]
                done [shape=Msquare]

                start -> plan
                plan -> done
            }
            """
        )

        def runner(node_id: str, prompt: str, context: Context) -> Outcome:
            if node_id == "plan":
                return Outcome(
                    status=OutcomeStatus.SUCCESS,
                    preferred_label="Approve",
                    context_updates={
                        "outcome": "fail",
                        "preferred_label": "Reject",
                        "custom.flag": "set",
                    },
                )
            return Outcome(status=OutcomeStatus.SUCCESS)

        result = PipelineExecutor(graph, runner).run(Context())

        assert result.status == "success"
        assert result.context["custom.flag"] == "set"
        assert result.context["outcome"] == "success"
        assert result.context["preferred_label"] == "Approve"

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

    def test_executor_fails_fast_when_start_missing(self):
        graph = parse_dot(
            """
            digraph G {
                entry [shape=box]
                done [shape=Msquare]
                entry -> done
            }
            """
        )

        with pytest.raises(RuntimeError, match="No start node found"):
            PipelineExecutor(graph, lambda *_: Outcome(status=OutcomeStatus.SUCCESS)).run(Context())

    def test_executor_fails_fast_when_start_ambiguous_without_shape_start(self):
        graph = parse_dot(
            """
            digraph G {
                start [shape=box]
                Start [shape=box]
                done [shape=Msquare]
                start -> done
                Start -> done
            }
            """
        )

        with pytest.raises(RuntimeError, match="Ambiguous start nodes"):
            PipelineExecutor(graph, lambda *_: Outcome(status=OutcomeStatus.SUCCESS)).run(Context())
