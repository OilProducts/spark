import pytest
from pathlib import Path
import threading
import tempfile
import time

from attractor.dsl import parse_dot
from attractor.engine.context import Context
from attractor.engine.executor import PipelineExecutor
from attractor.engine.outcome import Outcome, OutcomeStatus
from attractor.handlers import HandlerRunner, build_default_registry
from attractor.interviewer import Answer, QueueInterviewer

BRANCHING_CONDITION_FIXTURE = Path(__file__).resolve().parents[1] / "fixtures" / "branching_condition_workflow.dot"
REFERENCE_WORKFLOW_FIXTURE = Path(__file__).resolve().parents[2] / "flows" / "reference-1.1-01.dot"


class _WorkflowBackend:
    def __init__(self, responses: dict[str, bool]):
        self._responses = responses

    def run(self, node_id: str, prompt: str, context: Context, *, timeout=None) -> bool:
        del prompt, context, timeout
        return self._responses.get(node_id, True)


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
        assert result.status == "success"
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

        assert result.status == "fail"
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

        assert result.status == "success"
        assert seen_thread_ids == ["start", "work-thread"]

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

        assert result.status == "success"
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

        assert result.status == "success"
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
                start [shape=Mdiamond]
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

        assert result.status == "success"
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

        assert result.status == "success"
        assert seen_payload == ""

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

        assert result.status == "success"
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
            assert result.status == "success"
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
        assert result.status == "success"
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

        assert result.status == "success"
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

        assert result.status == "fail"
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
        assert result.status == "success"
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

        assert result.status == "success"
        assert [event["name"] for event in stage_started] == ["start", "work"]
        assert [event["name"] for event in stage_completed] == ["start", "work"]
        assert [event["index"] for event in stage_started] == [0, 1]
        assert [event["index"] for event in stage_completed] == [0, 1]
        assert all(isinstance(event.get("duration"), (int, float)) for event in stage_completed)
        assert all(float(event["duration"]) >= 0.0 for event in stage_completed)

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

        assert result.status == "success"
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
        assert result.status == "fail"
        assert stage_failed
        assert all(event["name"] == "work" for event in stage_failed)
        assert all(event["index"] == 1 for event in stage_failed)
        assert any(event["will_retry"] is False for event in stage_failed)

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
        assert result.status == "success"
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

        assert result.status == "success"
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

        assert result.status == "fail"
        assert result.current_node == "done"
        assert result.route_trace == ["start", "review", "done"]
        assert result.failure_reason == "Goal gate unsatisfied and no retry target"

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

        assert result.status == "success"
        assert result.current_node == "done"
        assert result.route_trace == ["start", "work", "done"]

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
                        "context.custom.flag": "set",
                    },
                )
            return Outcome(status=OutcomeStatus.SUCCESS)

        result = PipelineExecutor(graph, runner).run(Context())

        assert result.status == "success"
        assert result.context["context.custom.flag"] == "set"
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

        assert result.status == "success"
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

        assert result.status == "success"
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
