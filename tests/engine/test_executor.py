import pytest
from pathlib import Path
import threading
import tempfile
import time

from attractor.dsl import parse_dot
from attractor.engine.context import Context
from attractor.engine.executor import PipelineExecutor
from attractor.engine.outcome import FailureKind, Outcome, OutcomeStatus
from attractor.handlers import HandlerRunner, build_default_registry
from attractor.interviewer import Answer, QueueInterviewer
from attractor.llm_runtime import RUNTIME_LAUNCH_MODEL_KEY

BRANCHING_CONDITION_FIXTURE = Path(__file__).resolve().parents[1] / "fixtures" / "branching_condition_workflow.dot"
REFERENCE_WORKFLOW_FIXTURE = Path(__file__).resolve().parents[1] / "fixtures" / "flows" / "parallel-review-reference.dot"
STARTER_SPEC_IMPLEMENTATION_FIXTURE = (
    Path(__file__).resolve().parents[2] / "src" / "spark" / "flows" / "spec-implementation" / "implement-spec.dot"
)
STARTER_SPEC_IMPLEMENTATION_MILESTONE_FIXTURE = (
    Path(__file__).resolve().parents[2]
    / "src"
    / "spark"
    / "flows"
    / "spec-implementation"
    / "implement-milestone.dot"
)


class _WorkflowBackend:
    def __init__(self, responses: dict[str, bool]):
        self._responses = responses

    def run(
        self,
        node_id: str,
        prompt: str,
        context: Context,
        *,
        response_contract: str = "",
        contract_repair_attempts: int = 0,
        timeout=None,
        model=None,
        write_contract=None,
    ) -> bool:
        del prompt, context, response_contract, contract_repair_attempts, timeout, model, write_contract
        return self._responses.get(node_id, True)


class _StructuredLoopBackend:
    def __init__(self):
        self.prompts: dict[str, list[str]] = {}
        self.review_calls = 0

    def run(
        self,
        node_id: str,
        prompt: str,
        context: Context,
        *,
        response_contract: str = "",
        contract_repair_attempts: int = 0,
        timeout=None,
        model=None,
        write_contract=None,
    ) -> str | Outcome:
        del context, response_contract, contract_repair_attempts, timeout, model, write_contract
        self.prompts.setdefault(node_id, []).append(prompt)
        if node_id == "review":
            self.review_calls += 1
            if self.review_calls == 1:
                return Outcome(
                    status=OutcomeStatus.FAIL,
                    notes="needs another pass",
                    failure_reason="review requested fixes",
                    context_updates={
                        "context.review.summary": "implementation is incomplete",
                        "context.review.required_changes": "add regression coverage and tighten edge handling",
                        "context.review.blockers": "missing regression coverage",
                    },
                )
            return Outcome(
                status=OutcomeStatus.SUCCESS,
                notes="change is ready",
                context_updates={
                    "context.review.summary": "looks good",
                    "context.review.required_changes": "",
                    "context.review.blockers": "",
                },
            )
        return f"{node_id} completed"


class _SpecImplementationLoopBackend:
    def __init__(self):
        self.next_milestone_calls = 0
        self.child_milestone_ids: list[str] = []

    def run(
        self,
        node_id: str,
        prompt: str,
        context: Context,
        *,
        response_contract: str = "",
        contract_repair_attempts: int = 0,
        timeout=None,
        model=None,
        write_contract=None,
    ) -> Outcome:
        del prompt, response_contract, contract_repair_attempts, timeout, model, write_contract
        if node_id == "next_milestone":
            self.next_milestone_calls += 1
            if self.next_milestone_calls == 1:
                return Outcome(
                    status=OutcomeStatus.SUCCESS,
                    preferred_label="Work",
                    context_updates=self._milestone_updates(
                        milestone_id="M-ONE",
                        title="First milestone",
                    ),
                )
            if self.next_milestone_calls == 2:
                return Outcome(
                    status=OutcomeStatus.SUCCESS,
                    preferred_label="Work",
                    context_updates=self._milestone_updates(
                        milestone_id="M-TWO",
                        title="Second milestone",
                    ),
                )
            return Outcome(status=OutcomeStatus.SUCCESS, preferred_label="Audit")

        if node_id == "prepare_milestone_state":
            self.child_milestone_ids.append(str(context.get("context.milestone.id", "")))
            return Outcome(status=OutcomeStatus.SUCCESS)

        if node_id == "next_item":
            return Outcome(status=OutcomeStatus.SUCCESS, preferred_label="Audit")

        return Outcome(status=OutcomeStatus.SUCCESS)

    @staticmethod
    def _milestone_updates(*, milestone_id: str, title: str) -> dict[str, object]:
        return {
            "context.milestone.id": milestone_id,
            "context.milestone.title": title,
            "context.milestone.objective": f"Ship {title.lower()}",
            "context.milestone.requirement_ids": [f"REQ-{milestone_id}"],
            "context.milestone.acceptance_criteria": [f"{title} acceptance"],
            "context.milestone.target_paths": [f"src/{milestone_id.lower()}"],
            "context.milestone.attempts": 1,
        }


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

        assert result.status == "completed"
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

        assert result.status == "completed"
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

            assert result.status == "completed"
            assert (logs_root / "start" / "status.json").exists()
            assert (logs_root / "loop" / "status.json").exists()

            restart_logs_root = logs_root.parent / "logs.restart-1"
            assert (restart_logs_root / "work" / "status.json").exists()
            assert not (restart_logs_root / "start").exists()
            assert not (restart_logs_root / "loop").exists()

    def test_loop_restart_emits_fresh_pipeline_started_event(self):
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
        events = []

        def runner(node_id: str, prompt: str, context: Context) -> Outcome:
            return Outcome(status=OutcomeStatus.SUCCESS)

        result = PipelineExecutor(graph, runner, on_event=events.append).run(Context())

        started_nodes = [
            str(event.get("current_node"))
            for event in events
            if event.get("type") == "PipelineStarted"
        ]
        assert result.status == "completed"
        assert started_nodes == ["start", "work"]

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

        assert result.status == "completed"
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

        assert result.status == "completed"
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

        assert result.status == "completed"
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

        assert result.status == "completed"
        assert seen_fidelity == ["summary:medium", "full"]

    def test_executor_enforces_node_timeout_for_callable_runner(self):
        graph = parse_dot(
            """
            digraph G {
                start [shape=Mdiamond]
                slow [shape=box, timeout=20ms, max_retries=0]
                start -> slow
            }
            """
        )

        def runner(node_id: str, prompt: str, context: Context) -> Outcome:
            del prompt, context
            if node_id == "slow":
                time.sleep(0.1)
            return Outcome(status=OutcomeStatus.SUCCESS)

        result = PipelineExecutor(graph, runner).run(Context())

        assert result.status == "failed"
        assert "handler timed out after" in result.failure_reason

    def test_executor_target_node_thread_id_overrides_edge_thread_id(self):
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

        assert result.status == "completed"
        assert seen_thread_ids == ["start", "work-thread"]

    def test_executor_uses_edge_thread_id_when_full_fidelity_node_has_no_thread_id(self):
        graph = parse_dot(
            """
            digraph G {
                graph [default_fidelity="full"]
                start [shape=Mdiamond]
                work [shape=box]
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

        assert result.status == "completed"
        assert seen_thread_ids == ["start", "edge-thread"]

    def test_executor_uses_graph_thread_id_when_full_fidelity_has_no_node_or_edge_thread_id(self):
        graph = parse_dot(
            """
            digraph G {
                graph [default_fidelity="full", thread_id="graph-thread"]
                start [shape=Mdiamond]
                work [shape=box]
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

        assert result.status == "completed"
        assert seen_thread_ids == ["graph-thread", "graph-thread"]

    def test_executor_uses_subgraph_derived_class_for_full_fidelity_thread_fallback(self):
        graph = parse_dot(
            """
            digraph G {
                graph [default_fidelity="full"]
                start [shape=Mdiamond]
                done [shape=Msquare]

                subgraph cluster_loop {
                    graph [label="Loop A"]
                    work [shape=box]
                }

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

        assert result.status == "completed"
        assert seen_thread_ids == ["start", "loop-a"]

    @pytest.mark.parametrize(
        ("mode", "expected_snippet"),
        [
            ("truncate", "carryover:truncate"),
            ("compact", "carryover:compact"),
            ("summary:low", "carryover:summary:low"),
            ("summary:medium", "carryover:summary:medium"),
            ("summary:high", "carryover:summary:high"),
        ],
    )
    def test_executor_builds_runtime_context_carryover_for_non_full_fidelity(self, mode, expected_snippet):
        graph = parse_dot(
            f"""
            digraph G {{
                graph [goal="Ship docs", default_fidelity="{mode}"]
                start [shape=Mdiamond, spark.writes_context="[\\"context.release\\",\\"context.tests_passed\\"]"]
                work [shape=box]
                done [shape=Msquare]
                start -> work
                work -> done
            }}
            """
        )

        seen_payload = ""

        def runner(node_id: str, prompt: str, context: Context) -> Outcome:
            nonlocal seen_payload
            if node_id == "start":
                return Outcome(
                    status=OutcomeStatus.SUCCESS,
                    notes="seed-start",
                    context_updates={
                        "context.release": "v1",
                        "context.tests_passed": True,
                    },
                )
            if node_id == "work":
                seen_payload = str(context.get("_attractor.runtime.context_carryover", ""))
            return Outcome(status=OutcomeStatus.SUCCESS)

        initial_context = Context(values={"internal.run_id": "run-123"})
        result = PipelineExecutor(graph, runner).run(initial_context)

        assert result.status == "completed"
        assert expected_snippet in seen_payload
        assert "goal=Ship docs" in seen_payload
        assert "run_id=run-123" in seen_payload

    def test_executor_uses_empty_runtime_context_carryover_for_full_fidelity(self):
        graph = parse_dot(
            """
            digraph G {
                graph [goal="Ship docs", default_fidelity="full"]
                start [shape=Mdiamond]
                work [shape=box]
                done [shape=Msquare]
                start -> work
                work -> done
            }
            """
        )

        seen_payload = "not-set"

        def runner(node_id: str, prompt: str, context: Context) -> Outcome:
            nonlocal seen_payload
            if node_id == "work":
                seen_payload = str(context.get("_attractor.runtime.context_carryover", ""))
            return Outcome(status=OutcomeStatus.SUCCESS)

        result = PipelineExecutor(graph, runner).run(Context(values={"internal.run_id": "run-123"}))

        assert result.status == "completed"
        assert seen_payload == ""

    def test_executor_carries_review_feedback_into_next_implement_pass(self):
        graph = parse_dot(
            """
            digraph G {
                graph [goal="Ship docs", default_fidelity="summary:high"]
                start [shape=Mdiamond]
                implement [shape=box, prompt="Implement the requested change."]
                review [
                    shape=box,
                    prompt="Review the implementation and respond with a status envelope.",
                    spark.writes_context="[\\"context.review.summary\\",\\"context.review.required_changes\\",\\"context.review.blockers\\"]"
                ]
                done [shape=Msquare]
                start -> implement
                implement -> review [condition="outcome=success"]
                review -> done [condition="outcome=success"]
                review -> implement [condition="outcome=fail"]
            }
            """
        )

        backend = _StructuredLoopBackend()
        runner = HandlerRunner(graph, build_default_registry(codergen_backend=backend))

        result = PipelineExecutor(graph, runner).run(Context(values={"internal.run_id": "run-123"}))

        assert result.status == "completed"
        assert backend.review_calls == 2
        assert len(backend.prompts["implement"]) == 2
        assert "Current stage task:\n\nImplement the requested change." in backend.prompts["implement"][1]
        assert "context.review.required_changes=add regression coverage and tighten edge handling" in backend.prompts["implement"][1]
        assert result.route_trace == ["start", "implement", "review", "implement", "review", "done"]

    def test_executor_carries_retry_failure_reason_into_next_attempt(self):
        graph = parse_dot(
            """
            digraph G {
                graph [goal="Ship docs", default_fidelity="summary:high"]
                start [shape=Mdiamond]
                work [shape=box, max_retries=1]
                done [shape=Msquare]
                start -> work
                work -> done
            }
            """
        )

        work_calls = 0
        second_attempt_carryover = ""

        def runner(node_id: str, prompt: str, context: Context) -> Outcome:
            nonlocal work_calls, second_attempt_carryover
            del prompt
            if node_id == "work":
                work_calls += 1
                if work_calls == 1:
                    return Outcome(
                        status=OutcomeStatus.FAIL,
                        notes='{"outcome":"success","context":{"workflow_outcome":"failure"}}',
                        failure_reason="invalid structured status envelope: unexpected top-level keys context",
                    )
                second_attempt_carryover = str(context.get("_attractor.runtime.context_carryover", ""))
            return Outcome(status=OutcomeStatus.SUCCESS)

        result = PipelineExecutor(graph, runner).run(Context(values={"internal.run_id": "run-123"}))

        assert result.status == "completed"
        assert work_calls == 2
        assert "retry.node_id=work" in second_attempt_carryover
        assert "retry.attempt=1" in second_attempt_carryover
        assert "retry.max_attempts=2" in second_attempt_carryover
        assert (
            "retry.failure_reason=invalid structured status envelope: unexpected top-level keys context"
            in second_attempt_carryover
        )

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

        assert result.status == "completed"
        assert result.context["graph.goal"] == "Ship docs"
        assert seen_goals == ["Ship docs", "Ship docs"]

    def test_executor_seeds_builtin_context_keys_across_lifecycle(self):
        graph = parse_dot(
            """
            digraph G {
                graph [goal="Ship docs"]
                start [shape=Mdiamond]
                plan [shape=box]
                review [shape=box]
                done [shape=Msquare]
                start -> plan
                plan -> review
                review -> done
            }
            """
        )
        snapshots: list[dict[str, str]] = []

        def runner(node_id: str, prompt: str, context: Context) -> Outcome:
            snapshots.append(
                {
                    "node_id": node_id,
                    "outcome": str(context.get("outcome", "")),
                    "preferred_label": str(context.get("preferred_label", "")),
                    "graph.goal": str(context.get("graph.goal", "")),
                    "current_node": str(context.get("current_node", "")),
                }
            )
            if node_id == "start":
                return Outcome(status=OutcomeStatus.SUCCESS, preferred_label="Approve")
            return Outcome(status=OutcomeStatus.SUCCESS)

        result = PipelineExecutor(graph, runner).run(Context())

        assert result.status == "completed"
        assert snapshots == [
            {
                "node_id": "start",
                "outcome": "",
                "preferred_label": "",
                "graph.goal": "Ship docs",
                "current_node": "start",
            },
            {
                "node_id": "plan",
                "outcome": "success",
                "preferred_label": "Approve",
                "graph.goal": "Ship docs",
                "current_node": "plan",
            },
            {
                "node_id": "review",
                "outcome": "success",
                "preferred_label": "",
                "graph.goal": "Ship docs",
                "current_node": "review",
            },
        ]
        assert result.context["outcome"] == "success"
        assert result.context["preferred_label"] == ""
        assert result.context["graph.goal"] == "Ship docs"
        assert result.context["current_node"] == "done"

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

        with tempfile.TemporaryDirectory() as tmp:
            logs_root = Path(tmp) / "logs"
            result = PipelineExecutor(
                graph,
                runner,
                logs_root=str(logs_root),
                on_event=events.append,
            ).run(Context())

            event_types = [event["type"] for event in events]
            assert result.status == "completed"
            assert event_types[0] == "PipelineStarted"
            assert event_types[-1] == "PipelineCompleted"
            assert event_types.count("StageStarted") == 2
            assert event_types.count("StageCompleted") == 2
            assert event_types.count("CheckpointSaved") >= 1
            assert all(isinstance(event, dict) and "type" in event for event in events)

    def test_executor_does_not_emit_checkpoint_event_when_nothing_is_persisted(self):
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
        events: list[dict] = []

        def runner(node_id: str, prompt: str, context: Context) -> Outcome:
            return Outcome(status=OutcomeStatus.SUCCESS)

        result = PipelineExecutor(graph, runner, on_event=events.append).run(Context())

        event_types = [event["type"] for event in events]
        assert result.status == "completed"
        assert "CheckpointSaved" not in event_types

    def test_pipeline_started_and_completed_events_include_lifecycle_payload(self):
        graph = parse_dot(
            """
            digraph ReleaseFlow {
                start [shape=Mdiamond]
                work [shape=box]
                done [shape=Msquare]
                start -> work
                work -> done
            }
            """
        )
        events: list[dict] = []

        def runner(node_id: str, prompt: str, context: Context) -> Outcome:
            return Outcome(status=OutcomeStatus.SUCCESS)

        result = PipelineExecutor(graph, runner, on_event=events.append).run(Context())

        started = next(event for event in events if event["type"] == "PipelineStarted")
        completed = next(event for event in events if event["type"] == "PipelineCompleted")

        assert result.status == "completed"
        assert started["name"] == "ReleaseFlow"
        assert started["id"] == "ReleaseFlow"
        assert isinstance(completed["duration"], (int, float))
        assert float(completed["duration"]) >= 0.0
        assert completed["artifact_count"] == 3

    def test_pipeline_failed_event_includes_lifecycle_payload(self):
        graph = parse_dot(
            """
            digraph ReleaseFlow {
                start [shape=Mdiamond]
                work [shape=box]
                start -> work
            }
            """
        )
        events: list[dict] = []

        def runner(node_id: str, prompt: str, context: Context) -> Outcome:
            if node_id == "work":
                return Outcome(status=OutcomeStatus.FAIL, failure_reason="boom")
            return Outcome(status=OutcomeStatus.SUCCESS)

        result = PipelineExecutor(graph, runner, on_event=events.append).run(Context())
        failed = next(event for event in events if event["type"] == "PipelineFailed")

        assert result.status == "failed"
        assert failed["error"] == "boom"
        assert isinstance(failed["duration"], (int, float))
        assert float(failed["duration"]) >= 0.0
        assert failed["artifact_count"] == 2

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
        assert result.status == "completed"
        assert len(retry_events) == 1
        assert retry_events[0]["node_id"] == "work"
        assert retry_events[0]["name"] == "work"
        assert retry_events[0]["index"] == 1
        assert retry_events[0]["attempt"] == 1
        assert isinstance(retry_events[0]["delay"], (int, float))
        assert float(retry_events[0]["delay"]) >= 0.0

    def test_stage_lifecycle_events_include_spec_payload_fields(self):
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
        events: list[dict] = []

        def runner(node_id: str, prompt: str, context: Context) -> Outcome:
            return Outcome(status=OutcomeStatus.SUCCESS)

        result = PipelineExecutor(graph, runner, on_event=events.append).run(Context())

        stage_started = [event for event in events if event["type"] == "StageStarted"]
        stage_completed = [event for event in events if event["type"] == "StageCompleted"]

        assert result.status == "completed"
        assert [event["name"] for event in stage_started] == ["start", "work"]
        assert [event["name"] for event in stage_completed] == ["start", "work"]
        assert [event["index"] for event in stage_started] == [0, 1]
        assert [event["index"] for event in stage_completed] == [0, 1]
        assert all(isinstance(event.get("duration"), (int, float)) for event in stage_completed)
        assert all(float(event["duration"]) >= 0.0 for event in stage_completed)

    def test_llm_backed_stage_events_include_llm_model(self):
        graph = parse_dot(
            """
            digraph G {
                start [shape=Mdiamond]
                work [shape=box, llm_model="gpt-node-override"]
                done [shape=Msquare]
                start -> work
                work -> done
            }
            """
        )
        events: list[dict] = []

        def runner(node_id: str, prompt: str, context: Context) -> Outcome:
            return Outcome(status=OutcomeStatus.SUCCESS)

        result = PipelineExecutor(graph, runner, on_event=events.append).run(
            Context(values={RUNTIME_LAUNCH_MODEL_KEY: "gpt-launch-default"})
        )

        stage_started = [event for event in events if event["type"] == "StageStarted" and event["node_id"] == "work"]
        stage_completed = [event for event in events if event["type"] == "StageCompleted" and event["node_id"] == "work"]

        assert result.status == "completed"
        assert stage_started[0]["llm_model"] == "gpt-node-override"
        assert stage_completed[0]["llm_model"] == "gpt-node-override"

    def test_llm_backed_stage_failed_events_include_llm_model(self):
        graph = parse_dot(
            """
            digraph G {
                start [shape=Mdiamond]
                work [shape=box, llm_model="gpt-node-override"]
                start -> work
            }
            """
        )
        events: list[dict] = []

        def runner(node_id: str, prompt: str, context: Context) -> Outcome:
            if node_id == "work":
                return Outcome(status=OutcomeStatus.FAIL, failure_reason="boom")
            return Outcome(status=OutcomeStatus.SUCCESS)

        result = PipelineExecutor(graph, runner, on_event=events.append).run(
            Context(values={RUNTIME_LAUNCH_MODEL_KEY: "gpt-launch-default"})
        )

        stage_failed = [event for event in events if event["type"] == "StageFailed" and event["node_id"] == "work"]

        assert result.status == "failed"
        assert stage_failed
        assert all(event["llm_model"] == "gpt-node-override" for event in stage_failed)

    def test_non_llm_stage_events_do_not_include_llm_model(self):
        graph = parse_dot(
            """
            digraph G {
                start [shape=Mdiamond]
                gate [shape=hexagon]
                done [shape=Msquare]
                start -> gate
                gate -> done
            }
            """
        )
        events: list[dict] = []

        def runner(node_id: str, prompt: str, context: Context) -> Outcome:
            return Outcome(status=OutcomeStatus.SUCCESS)

        result = PipelineExecutor(graph, runner, on_event=events.append).run(
            Context(values={RUNTIME_LAUNCH_MODEL_KEY: "gpt-launch-default"})
        )

        gate_events = [event for event in events if event.get("node_id") == "gate"]

        assert result.status == "completed"
        assert gate_events
        assert all("llm_model" not in event for event in gate_events)

    def test_stage_failed_event_marks_retryable_attempts(self):
        graph = parse_dot(
            """
            digraph G {
                start [shape=Mdiamond]
                work [shape=box, max_retries=1]
                done [shape=Msquare]
                start -> work
                work -> done
            }
            """
        )
        events: list[dict] = []
        attempts = {"work": 0}

        def runner(node_id: str, prompt: str, context: Context) -> Outcome:
            if node_id != "work":
                return Outcome(status=OutcomeStatus.SUCCESS)
            attempts["work"] += 1
            if attempts["work"] == 1:
                return Outcome(status=OutcomeStatus.FAIL, failure_reason="transient")
            return Outcome(status=OutcomeStatus.SUCCESS)

        result = PipelineExecutor(graph, runner, on_event=events.append).run(Context())

        stage_failed = [event for event in events if event["type"] == "StageFailed"]
        retry_events = [event for event in events if event["type"] == "StageRetrying"]

        assert result.status == "completed"
        assert len(stage_failed) == 1
        assert stage_failed[0]["name"] == "work"
        assert stage_failed[0]["node_id"] == "work"
        assert stage_failed[0]["index"] == 1
        assert stage_failed[0]["error"] == "transient"
        assert stage_failed[0]["will_retry"] is True
        assert len(retry_events) == 1
        assert stage_failed[0]["attempt"] == retry_events[0]["attempt"]

    def test_stage_failed_events_keep_consistent_stage_index_on_terminal_failure(self):
        graph = parse_dot(
            """
            digraph G {
                start [shape=Mdiamond]
                work [shape=box]
                start -> work
            }
            """
        )
        events: list[dict] = []

        def runner(node_id: str, prompt: str, context: Context) -> Outcome:
            if node_id == "work":
                return Outcome(status=OutcomeStatus.FAIL, failure_reason="boom")
            return Outcome(status=OutcomeStatus.SUCCESS)

        result = PipelineExecutor(graph, runner, on_event=events.append).run(Context())

        stage_failed = [event for event in events if event["type"] == "StageFailed" and event["node_id"] == "work"]
        assert result.status == "failed"
        assert stage_failed
        assert all(event["name"] == "work" for event in stage_failed)
        assert all(event["index"] == 1 for event in stage_failed)
        assert any(event["will_retry"] is False for event in stage_failed)

    def test_failed_stage_does_not_fall_through_unconditional_edge_by_default(self):
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

        def runner(node_id: str, prompt: str, context: Context) -> Outcome:
            del prompt, context
            if node_id == "work":
                return Outcome(status=OutcomeStatus.FAIL, failure_reason="boom")
            return Outcome(status=OutcomeStatus.SUCCESS)

        result = PipelineExecutor(graph, runner).run(Context())

        assert result.status == "failed"
        assert result.current_node == "work"
        assert result.route_trace == ["start", "work"]

    def test_failed_stage_can_continue_when_node_opted_into_continue_policy(self):
        graph = parse_dot(
            """
            digraph G {
                start [shape=Mdiamond]
                work [shape=box, error_policy="continue"]
                done [shape=Msquare]
                start -> work
                work -> done
            }
            """
        )

        def runner(node_id: str, prompt: str, context: Context) -> Outcome:
            del prompt, context
            if node_id == "work":
                return Outcome(status=OutcomeStatus.FAIL, failure_reason="boom")
            return Outcome(status=OutcomeStatus.SUCCESS)

        result = PipelineExecutor(graph, runner).run(Context())

        assert result.status == "completed"
        assert result.current_node == "done"
        assert result.route_trace == ["start", "work", "done"]

    def test_failed_stage_still_uses_matching_fail_condition_without_continue_policy(self):
        graph = parse_dot(
            """
            digraph G {
                start [shape=Mdiamond]
                work [shape=box]
                blocked [shape=box]
                done [shape=Msquare]
                start -> work
                work -> blocked [condition="outcome=fail && preferred_label=Blocked", label="Blocked"]
                work -> done
                blocked -> done
            }
            """
        )

        def runner(node_id: str, prompt: str, context: Context) -> Outcome:
            del prompt, context
            if node_id == "work":
                return Outcome(status=OutcomeStatus.FAIL, preferred_label="Blocked", failure_reason="blocked")
            return Outcome(status=OutcomeStatus.SUCCESS)

        result = PipelineExecutor(graph, runner).run(Context())

        assert result.status == "completed"
        assert result.current_node == "done"
        assert result.route_trace == ["start", "work", "blocked", "done"]

    def test_failure_loop_normalizes_shorthand_context_updates_before_routing(self):
        graph = parse_dot(
            """
            digraph G {
                start [shape=Mdiamond]
                review [
                    shape=box,
                    spark.writes_context="[\\"context.review.summary\\",\\"context.review.required_changes\\",\\"context.review.blockers\\"]"
                ]
                implement [shape=box]
                done [shape=Msquare]

                start -> review
                review -> implement [condition="outcome=fail && preferred_label=Fix", label="Fix"]
                implement -> done
            }
            """
        )

        def runner(node_id: str, prompt: str, context: Context) -> Outcome:
            del prompt
            if node_id == "review":
                return Outcome(
                    status=OutcomeStatus.FAIL,
                    preferred_label="Fix",
                    failure_reason="needs fixes",
                    context_updates={
                        "review.summary": "implementation is incomplete",
                        "review.required_changes": "add regression coverage",
                        "review.blockers": "",
                    },
                )
            if node_id == "implement":
                assert context.get("context.review.summary") == "implementation is incomplete"
                assert context.get("context.review.required_changes") == "add regression coverage"
                assert context.get("review.summary") is None
            return Outcome(status=OutcomeStatus.SUCCESS)

        result = PipelineExecutor(graph, runner).run(Context())

        assert result.status == "completed"
        assert result.route_trace == ["start", "review", "implement", "done"]
        assert result.context["context.review.summary"] == "implementation is incomplete"
        assert result.context["context.review.required_changes"] == "add regression coverage"

    def test_wait_human_block_routes_to_blocked_exit(self):
        graph = parse_dot(
            """
            digraph G {
                start [shape=Mdiamond]
                review [shape=hexagon, prompt="Review"]
                blocked_exit [shape=box]
                done [shape=Msquare]
                start -> review
                review -> blocked_exit [label="Block"]
                review -> done [label="Approve"]
                blocked_exit -> done [condition="outcome=success"]
            }
            """
        )
        backend = _WorkflowBackend({"start": True, "blocked_exit": True})
        interviewer = QueueInterviewer([Answer(selected_values=["blocked_exit"])])
        runner = HandlerRunner(graph, build_default_registry(codergen_backend=backend, interviewer=interviewer))

        result = PipelineExecutor(graph, runner).run(Context())

        assert result.status == "completed"
        assert result.current_node == "done"
        assert result.route_trace == ["start", "review", "blocked_exit", "done"]

    def test_starter_spec_implementation_flow_stops_after_failed_requirements_extraction(self):
        graph = parse_dot(STARTER_SPEC_IMPLEMENTATION_FIXTURE.read_text(encoding="utf-8"))

        def runner(node_id: str, prompt: str, context: Context) -> Outcome:
            del prompt, context
            if node_id == "extract_requirements":
                return Outcome(status=OutcomeStatus.FAIL, failure_reason="timed out")
            return Outcome(status=OutcomeStatus.SUCCESS)

        result = PipelineExecutor(graph, runner).run(Context(values={"context.request.spec_path": "spec.md"}))

        assert result.status == "failed"
        assert result.current_node == "extract_requirements"
        assert "design_architecture" not in result.route_trace

    def test_starter_spec_implementation_prepare_workspace_retries_once(self):
        graph = parse_dot(STARTER_SPEC_IMPLEMENTATION_FIXTURE.read_text(encoding="utf-8"))

        assert graph.nodes["prepare_workspace"].attrs["max_retries"].value == 1

    def test_starter_spec_implementation_replans_invalid_milestones_after_architecture_evaluation(self):
        graph = parse_dot(STARTER_SPEC_IMPLEMENTATION_FIXTURE.read_text(encoding="utf-8"))
        calls: list[str] = []
        validate_attempts = 0

        def runner(node_id: str, prompt: str, context: Context) -> Outcome:
            nonlocal validate_attempts
            del prompt, context
            calls.append(node_id)
            if node_id == "evaluate_architecture":
                return Outcome(status=OutcomeStatus.SUCCESS, preferred_label="Approve")
            if node_id == "validate_milestone_plan":
                validate_attempts += 1
                if validate_attempts == 1:
                    return Outcome(
                        status=OutcomeStatus.FAIL,
                        preferred_label="Replan",
                        failure_reason="milestone cycle detected",
                    )
                return Outcome(status=OutcomeStatus.SUCCESS)
            if node_id == "next_milestone":
                return Outcome(
                    status=OutcomeStatus.FAIL,
                    preferred_label="Blocked",
                    failure_reason="stop after validation coverage",
                )
            if node_id == "blocked_exit":
                return Outcome(status=OutcomeStatus.SUCCESS)
            return Outcome(status=OutcomeStatus.SUCCESS)

        result = PipelineExecutor(graph, runner).run(Context(values={"context.request.spec_path": "spec.md"}))

        assert result.status == "completed"
        assert calls == [
            "start",
            "prepare_workspace",
            "extract_requirements",
            "design_architecture",
            "evaluate_architecture",
            "plan_milestones",
            "validate_milestone_plan",
            "plan_milestones",
            "validate_milestone_plan",
            "next_milestone",
            "blocked_exit",
        ]
        assert "run_milestone" not in calls

    def test_starter_milestone_worker_reextracts_invalid_items_before_next_item(self):
        graph = parse_dot(STARTER_SPEC_IMPLEMENTATION_MILESTONE_FIXTURE.read_text(encoding="utf-8"))
        calls: list[str] = []
        validate_attempts = 0

        def runner(node_id: str, prompt: str, context: Context) -> Outcome:
            nonlocal validate_attempts
            del prompt, context
            calls.append(node_id)
            if node_id == "validate_item_plan":
                validate_attempts += 1
                if validate_attempts == 1:
                    return Outcome(
                        status=OutcomeStatus.FAIL,
                        preferred_label="Reextract",
                        failure_reason="item dependency cycle detected",
                    )
                return Outcome(status=OutcomeStatus.SUCCESS)
            if node_id == "next_item":
                return Outcome(
                    status=OutcomeStatus.FAIL,
                    preferred_label="Blocked",
                    failure_reason="stop after validation coverage",
                )
            if node_id == "blocked_exit":
                return Outcome(status=OutcomeStatus.SUCCESS)
            return Outcome(status=OutcomeStatus.SUCCESS)

        result = PipelineExecutor(graph, runner).run(Context())

        assert result.status == "completed"
        assert calls == [
            "start",
            "prepare_milestone_state",
            "extract_items",
            "validate_item_plan",
            "extract_items",
            "validate_item_plan",
            "next_item",
            "blocked_exit",
        ]
        assert "plan_current" not in calls
        assert "implement_current" not in calls

    def test_starter_spec_implementation_flow_restarts_child_worker_for_each_selected_milestone(
        self, tmp_path
    ):
        graph = parse_dot(STARTER_SPEC_IMPLEMENTATION_FIXTURE.read_text(encoding="utf-8"))
        backend = _SpecImplementationLoopBackend()
        interviewer = QueueInterviewer([Answer(selected_values=["Approve"])])
        logs_root = tmp_path / "logs"
        runner = HandlerRunner(
            graph,
            build_default_registry(codergen_backend=backend, interviewer=interviewer),
            logs_root=logs_root,
        )

        result = PipelineExecutor(graph, runner, logs_root=str(logs_root)).run(
            Context(
                values={
                    "context.request.spec_path": "spec.md",
                    "internal.flow_source_dir": str(STARTER_SPEC_IMPLEMENTATION_FIXTURE.parent),
                    "internal.run_workdir": str(tmp_path),
                }
            )
        )

        assert result.status == "completed"
        assert result.route_trace.count("run_milestone") == 2
        assert backend.child_milestone_ids == ["M-ONE", "M-TWO"]

    def test_executor_emits_parallel_and_interview_runtime_events(self):
        graph = parse_dot(REFERENCE_WORKFLOW_FIXTURE.read_text(encoding="utf-8"))
        events = []
        backend = _WorkflowBackend(
            {
                "start": True,
                "plan": True,
                "implement": True,
                "branch_docs": True,
                "branch_tests": True,
                "final_review": True,
            }
        )
        interviewer = QueueInterviewer([Answer(selected_values=["Proceed"])])
        runner = HandlerRunner(graph, build_default_registry(codergen_backend=backend, interviewer=interviewer))

        result = PipelineExecutor(graph, runner, on_event=events.append).run(Context())

        event_types = [event["type"] for event in events]
        assert result.status == "completed"
        assert "InterviewStarted" in event_types
        assert "InterviewCompleted" in event_types
        assert "ParallelStarted" in event_types
        assert event_types.count("ParallelBranchStarted") == 2
        assert event_types.count("ParallelBranchCompleted") == 2
        assert "ParallelCompleted" in event_types

    def test_parallel_completed_event_counts_failures_even_when_ignore_policy_filters_results(self):
        graph = parse_dot(
            """
            digraph G {
                start [shape=Mdiamond]
                fan [shape=component, join_policy=wait_all, error_policy=ignore, max_parallel=1]
                branch_a_ok [shape=box]
                branch_z_fail [shape=box]
                join [shape=tripleoctagon]
                done [shape=Msquare]

                start -> fan
                fan -> branch_a_ok
                fan -> branch_z_fail
                branch_a_ok -> join [condition="outcome=success"]
                join -> done
            }
            """
        )
        events: list[dict] = []
        backend = _WorkflowBackend(
            {
                "start": True,
                "branch_a_ok": True,
                "branch_z_fail": False,
            }
        )
        runner = HandlerRunner(graph, build_default_registry(codergen_backend=backend))

        result = PipelineExecutor(graph, runner, on_event=events.append).run(Context())

        parallel_started = next(event for event in events if event["type"] == "ParallelStarted")
        parallel_completed = next(event for event in events if event["type"] == "ParallelCompleted")
        branch_started = [event for event in events if event["type"] == "ParallelBranchStarted"]
        branch_completed = [event for event in events if event["type"] == "ParallelBranchCompleted"]

        assert result.status == "completed"
        assert parallel_started["branch_count"] == 2
        assert len(branch_started) == 2
        assert len(branch_completed) == 2
        assert parallel_completed["success_count"] == 1
        assert parallel_completed["failure_count"] == 1

    def test_executor_rejects_concurrent_top_level_traversal(self):
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

        entered_start = threading.Event()
        release_start = threading.Event()

        def runner(node_id: str, prompt: str, context: Context) -> Outcome:
            if node_id == "start":
                entered_start.set()
                release_start.wait(timeout=2.0)
            return Outcome(status=OutcomeStatus.SUCCESS)

        executor = PipelineExecutor(graph, runner)
        first_run_done = threading.Event()

        def run_first_pipeline() -> None:
            try:
                executor.run(Context())
            finally:
                first_run_done.set()

        thread = threading.Thread(target=run_first_pipeline, daemon=True)
        thread.start()
        assert entered_start.wait(timeout=2.0)

        with pytest.raises(RuntimeError, match="single-threaded"):
            executor.run_from("work", Context())

        release_start.set()
        assert first_run_done.wait(timeout=2.0)

    def test_goal_gate_blocks_terminal_exit_when_visited_gate_failed(self):
        graph = parse_dot(
            """
            digraph G {
                start [shape=Mdiamond]
                review [shape=box, goal_gate=true]
                done [shape=Msquare]

                start -> review
                review -> done
            }
            """
        )

        def runner(node_id: str, prompt: str, context: Context) -> Outcome:
            if node_id == "review":
                return Outcome(status=OutcomeStatus.FAIL, failure_reason="needs changes")
            return Outcome(status=OutcomeStatus.SUCCESS)

        result = PipelineExecutor(graph, runner).run(Context())

        assert result.status == "completed"
        assert result.outcome == "failure"
        assert result.outcome_reason_code == "goal_gate_unsatisfied"
        assert result.outcome_reason_message == "Goal gate unsatisfied and no retry target"
        assert result.current_node == "done"
        assert result.route_trace == ["start", "review", "done"]
        assert result.failure_reason == ""

    def test_goal_gate_does_not_block_exit_for_unvisited_goal_gate_nodes(self):
        graph = parse_dot(
            """
            digraph G {
                start [shape=Mdiamond]
                work [shape=box]
                skipped_gate [shape=box, goal_gate=true]
                done [shape=Msquare]

                start -> work
                work -> done
            }
            """
        )

        result = PipelineExecutor(graph, lambda *_: Outcome(status=OutcomeStatus.SUCCESS)).run(Context())

        assert result.status == "completed"
        assert result.current_node == "done"

    def test_run_from_returns_success_when_non_fail_stage_has_no_route(self):
        graph = parse_dot(
            """
            digraph G {
                work [shape=box]
                alternate [shape=box]
                work -> alternate [condition="outcome=fail"]
            }
            """
        )

        def runner(node_id: str, prompt: str, context: Context) -> Outcome:
            return Outcome(status=OutcomeStatus.SUCCESS, notes=node_id)

        result = PipelineExecutor(graph, runner).run_from("work", Context())

        assert result.status == "completed"
        assert result.current_node == "work"
        assert result.completed_nodes == ["work"]
        assert result.route_trace == ["work"]

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

        assert result.status == "completed"
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

        assert result.status == "completed"
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
                plan [shape=box, prompt="plan", spark.writes_context="[\\"needs_fix\\"]"]
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

        assert result.status == "completed"
        assert result.current_node == "done"
        assert result.completed_nodes == ["start", "plan", "fix"]
        assert result.context.get("needs_fix") == "true"
        assert calls[1][1] == "plan"

    def test_executor_applies_context_updates_before_outcome_and_preferred_label(self):
        graph = parse_dot(
            """
            digraph G {
                start [shape=Mdiamond]
                plan [shape=box, spark.writes_context="[\\"outcome\\",\\"preferred_label\\",\\"context.custom.flag\\"]"]
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
                        "context.custom.flag": "set",
                    },
                )
            return Outcome(status=OutcomeStatus.SUCCESS)

        result = PipelineExecutor(graph, runner).run(Context())

        assert result.status == "completed"
        assert result.context["context.custom.flag"] == "set"
        assert result.context["outcome"] == "success"
        assert result.context["preferred_label"] == "Approve"

    def test_executor_removes_context_keys_when_context_updates_use_null(self):
        graph = parse_dot(
            """
            digraph G {
                start [shape=Mdiamond]
                plan [shape=box, spark.writes_context="[\\"context.custom.flag\\"]"]
                done [shape=Msquare]

                start -> plan
                plan -> done
            }
            """
        )

        def runner(node_id: str, prompt: str, context: Context) -> Outcome:
            del prompt, context
            if node_id == "plan":
                return Outcome(status=OutcomeStatus.SUCCESS, context_updates={"context.custom.flag": None})
            return Outcome(status=OutcomeStatus.SUCCESS)

        result = PipelineExecutor(graph, runner).run(Context(values={"context.custom.flag": "set"}))

        assert result.status == "completed"
        assert result.context.get("context.custom.flag") is None

    def test_custom_handler_undeclared_writes_fail_as_contract_fault_without_merging(self):
        graph = parse_dot(
            """
            digraph G {
                start [shape=Mdiamond]
                writer [shape=box, type="custom.writer"]
                done [shape=Msquare]

                start -> writer
                writer -> done
            }
            """
        )

        class _Writer:
            def execute(self, runtime):
                del runtime
                return Outcome(status=OutcomeStatus.SUCCESS, context_updates={"artifact_path": "/tmp/out"})

        registry = build_default_registry(
            codergen_backend=_WorkflowBackend({}),
            extra_handlers={"custom.writer": _Writer()},
        )

        result = PipelineExecutor(graph, HandlerRunner(graph, registry)).run(Context())

        assert result.status == "failed"
        assert result.current_node == "writer"
        assert result.context.get("artifact_path") is None
        assert result.node_outcomes["writer"].failure_kind is FailureKind.CONTRACT
        assert "artifact_path" in result.node_outcomes["writer"].failure_reason
        assert "<none>" in result.node_outcomes["writer"].failure_reason

    def test_custom_handler_declared_bare_writes_pass_contract_enforcement(self):
        graph = parse_dot(
            """
            digraph G {
                start [shape=Mdiamond]
                writer [shape=box, type="custom.writer", spark.writes_context="[\\"artifact_path\\"]"]
                done [shape=Msquare]

                start -> writer
                writer -> done
            }
            """
        )

        class _Writer:
            def execute(self, runtime):
                del runtime
                return Outcome(status=OutcomeStatus.SUCCESS, context_updates={"artifact_path": "/tmp/out"})

        registry = build_default_registry(
            codergen_backend=_WorkflowBackend({}),
            extra_handlers={"custom.writer": _Writer()},
        )

        result = PipelineExecutor(graph, HandlerRunner(graph, registry)).run(Context())

        assert result.status == "completed"
        assert result.context.get("artifact_path") == "/tmp/out"

    def test_custom_handler_invalid_context_update_keys_fail_as_contract_fault_without_merging(self):
        graph = parse_dot(
            """
            digraph G {
                start [shape=Mdiamond]
                writer [shape=box, type="custom.writer", spark.writes_context="[\\"context.review.summary\\"]"]
                done [shape=Msquare]

                start -> writer
                writer -> done
            }
            """
        )

        class _Writer:
            def execute(self, runtime):
                del runtime
                return Outcome(
                    status=OutcomeStatus.SUCCESS,
                    context_updates={".specflow/state.json": "/tmp/out"},
                )

        registry = build_default_registry(
            codergen_backend=_WorkflowBackend({}),
            extra_handlers={"custom.writer": _Writer()},
        )

        result = PipelineExecutor(graph, HandlerRunner(graph, registry)).run(Context())

        assert result.status == "failed"
        assert result.current_node == "writer"
        assert result.context.get(".specflow/state.json") is None
        assert result.node_outcomes["writer"].failure_kind is FailureKind.CONTRACT
        assert "invalid context_updates keys" in result.node_outcomes["writer"].failure_reason
        assert ".specflow/state.json" in result.node_outcomes["writer"].failure_reason

    def test_custom_handler_cannot_write_context_stack_child_prefix_without_declaration(self):
        graph = parse_dot(
            """
            digraph G {
                start [shape=Mdiamond]
                writer [shape=box, type="custom.writer"]
                done [shape=Msquare]

                start -> writer
                writer -> done
            }
            """
        )

        class _Writer:
            def execute(self, runtime):
                del runtime
                return Outcome(
                    status=OutcomeStatus.SUCCESS,
                    context_updates={"context.stack.child.poison": "x"},
                )

        registry = build_default_registry(
            codergen_backend=_WorkflowBackend({}),
            extra_handlers={"custom.writer": _Writer()},
        )

        result = PipelineExecutor(graph, HandlerRunner(graph, registry)).run(Context())

        assert result.status == "failed"
        assert result.current_node == "writer"
        assert result.context.get("context.stack.child.poison") is None
        assert result.node_outcomes["writer"].failure_kind is FailureKind.CONTRACT
        assert "context.stack.child.poison" in result.node_outcomes["writer"].failure_reason

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

        assert result.status == "completed"
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

        assert result.status == "completed"
        assert result.route_trace == ["start", "plan", "gate", "fix", "done"]

    def test_conditional_node_routes_using_prior_stage_preferred_label(self):
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
                gate -> fix [condition="preferred_label=Fix"]
                gate -> done [condition="preferred_label=Approve"]
                fix -> done
            }
            """
        )

        def runner(node_id: str, prompt: str, context: Context) -> Outcome:
            if node_id == "plan":
                return Outcome(status=OutcomeStatus.SUCCESS, preferred_label="Fix")
            return Outcome(status=OutcomeStatus.SUCCESS)

        result = PipelineExecutor(graph, runner).run(Context())

        assert result.status == "completed"
        assert result.route_trace == ["start", "plan", "gate", "fix", "done"]

    def test_conditional_node_preserves_prior_stage_preferred_label_exactly(self):
        graph = parse_dot(
            """
            digraph G {
                start [shape=Mdiamond]
                plan [shape=box]
                gate [shape=diamond]
                spaced [shape=box]
                trimmed [shape=box]
                done [shape=Msquare]

                start -> plan
                plan -> gate
                gate -> spaced [condition="preferred_label=\\" Fix \\""]
                gate -> trimmed [condition="preferred_label=Fix"]
                spaced -> done
                trimmed -> done
            }
            """
        )

        def runner(node_id: str, prompt: str, context: Context) -> Outcome:
            if node_id == "plan":
                return Outcome(status=OutcomeStatus.SUCCESS, preferred_label=" Fix ")
            return Outcome(status=OutcomeStatus.SUCCESS)

        result = PipelineExecutor(graph, runner).run(Context())

        assert result.status == "completed"
        assert result.route_trace == ["start", "plan", "gate", "spaced", "done"]

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
