import json
from pathlib import Path
import tempfile

import pytest

from attractor.dsl import parse_dot
from attractor.engine import Checkpoint, load_checkpoint, save_checkpoint
import attractor.engine.executor as executor_module
from attractor.engine.context import Context
from attractor.engine.executor import PipelineExecutor
from attractor.engine.outcome import FailureKind, Outcome, OutcomeStatus
from attractor.handlers import HandlerRunner, build_default_registry


class TestCheckpointAndArtifacts:
    def test_end_to_end_run_directory_structure_includes_terminal_stage_artifacts(self):
        graph = parse_dot(
            """
            digraph release_flow {
                graph [goal="Ship release"]
                start [shape=Mdiamond]
                plan [shape=box, prompt="Draft release plan"]
                done [shape=Msquare]

                start -> plan
                plan -> done
            }
            """
        )

        with tempfile.TemporaryDirectory() as tmp:
            logs_root = Path(tmp) / "run-logs"

            def runner(node_id: str, prompt: str, context: Context, *, emit_event=None) -> Outcome:
                return Outcome(status=OutcomeStatus.SUCCESS, notes=f"{node_id} complete")

            result = PipelineExecutor(
                graph,
                runner,
                logs_root=str(logs_root),
            ).run(Context())

            assert result.status == "completed"

            required_root_entries = [
                logs_root / "checkpoint.json",
                logs_root / "manifest.json",
                logs_root / "artifacts",
            ]
            for path in required_root_entries:
                assert path.exists()

            for node_id in ["start", "plan", "done"]:
                stage_dir = logs_root / node_id
                assert stage_dir.is_dir()
                assert (stage_dir / "status.json").is_file()
                assert (stage_dir / "prompt.md").is_file()
                assert (stage_dir / "response.md").is_file()

            terminal_status = json.loads((logs_root / "done" / "status.json").read_text(encoding="utf-8"))
            assert terminal_status["outcome"] == "success"

    def test_run_root_bootstraps_checkpoint_manifest_stage_dirs_and_artifacts_directory(self):
        graph = parse_dot(
            """
            digraph release_flow {
                graph [goal="Ship release"]
                start [shape=Mdiamond]
                plan [shape=box]
                done [shape=Msquare]

                start -> plan
                plan -> done
            }
            """
        )

        with tempfile.TemporaryDirectory() as tmp:
            logs_root = Path(tmp) / "run-logs"

            def runner(node_id: str, prompt: str, context: Context, *, emit_event=None) -> Outcome:
                return Outcome(status=OutcomeStatus.SUCCESS, notes=f"{node_id} complete")

            result = PipelineExecutor(
                graph,
                runner,
                logs_root=str(logs_root),
            ).run(Context())

            assert result.status == "completed"
            assert logs_root.is_dir()
            assert (logs_root / "checkpoint.json").exists()
            assert (logs_root / "manifest.json").exists()
            assert (logs_root / "artifacts").is_dir()
            assert (logs_root / "start").is_dir()
            assert (logs_root / "plan").is_dir()

            checkpoint_payload = json.loads((logs_root / "checkpoint.json").read_text(encoding="utf-8"))
            assert checkpoint_payload["current_node"] == "done"
            assert checkpoint_payload["completed_nodes"] == ["start", "plan"]

            manifest_payload = json.loads((logs_root / "manifest.json").read_text(encoding="utf-8"))
            assert manifest_payload["graph_id"] == "release_flow"
            assert manifest_payload["goal"] == "Ship release"
            assert manifest_payload["start_node"] == "start"
            assert isinstance(manifest_payload["started_at"], str)
            assert manifest_payload["started_at"]

    def test_auto_status_synthesizes_success_when_runner_returns_none(self):
        graph = parse_dot(
            """
            digraph G {
                start [shape=Mdiamond]
                plan [shape=box, auto_status=true]
                done [shape=Msquare]

                start -> plan
                plan -> done
            }
            """
        )

        with tempfile.TemporaryDirectory() as tmp:
            logs_root = Path(tmp) / "logs"

            def runner(node_id: str, prompt: str, context: Context):
                if node_id == "plan":
                    return None
                return Outcome(status=OutcomeStatus.SUCCESS)

            result = PipelineExecutor(
                graph,
                runner,
                logs_root=str(logs_root),
            ).run(Context())

            assert result.status == "completed"
            assert result.node_outcomes["plan"].status == OutcomeStatus.SUCCESS
            status_payload = json.loads((logs_root / "plan" / "status.json").read_text(encoding="utf-8"))
            assert status_payload["outcome"] == "success"
            assert status_payload["notes"] == "auto-status: handler completed without writing status"

    def test_auto_status_synthesizes_success_when_custom_handler_returns_none(self):
        graph = parse_dot(
            """
            digraph G {
                start [shape=Mdiamond]
                plan [shape=box, type="custom.none", auto_status=true]
                done [shape=Msquare]

                start -> plan
                plan -> done
            }
            """
        )

        class _NoOutcomeHandler:
            def execute(self, runtime):
                del runtime
                return None

        with tempfile.TemporaryDirectory() as tmp:
            logs_root = Path(tmp) / "logs"
            registry = build_default_registry(extra_handlers={"custom.none": _NoOutcomeHandler()})
            runner = HandlerRunner(graph, registry, logs_root=str(logs_root))

            result = PipelineExecutor(
                graph,
                runner,
                logs_root=str(logs_root),
            ).run(Context())

            assert result.status == "completed"
            assert result.node_outcomes["plan"].status == OutcomeStatus.SUCCESS
            status_payload = json.loads((logs_root / "plan" / "status.json").read_text(encoding="utf-8"))
            assert status_payload["outcome"] == "success"
            assert status_payload["notes"] == "auto-status: handler completed without writing status"

    def test_status_json_contract_defaults_optional_fields_for_non_terminal_stage(self):
        graph = parse_dot(
            """
            digraph G {
                start [shape=Mdiamond]
                plan [shape=box, spark.writes_context="[\\"context.visible\\",\\"_attractor.node_outcomes\\"]"]
                done [shape=Msquare]

                start -> plan
                plan -> done
            }
            """
        )

        with tempfile.TemporaryDirectory() as tmp:
            logs_root = Path(tmp) / "logs"

            def runner(node_id: str, prompt: str, context: Context, *, emit_event=None) -> Outcome:
                if node_id == "plan":
                    return Outcome(
                        status=OutcomeStatus.SUCCESS,
                        suggested_next_ids=None,  # type: ignore[arg-type]
                        context_updates=None,  # type: ignore[arg-type]
                        notes=None,  # type: ignore[arg-type]
                    )
                return Outcome(status=OutcomeStatus.SUCCESS)

            result = PipelineExecutor(
                graph,
                runner,
                logs_root=str(logs_root),
            ).run(Context())

            assert result.status == "completed"
            status_payload = json.loads((logs_root / "plan" / "status.json").read_text(encoding="utf-8"))
            assert status_payload["outcome"] == "success"
            assert status_payload["preferred_label"] == ""
            assert status_payload["suggested_next_ids"] == []
            assert status_payload["context_updates"] == {}
            assert status_payload["notes"] == ""

    def test_status_json_contract_includes_optional_failure_kind_when_present(self):
        graph = parse_dot(
            """
            digraph G {
                start [shape=Mdiamond]
                plan [shape=box, spark.writes_context="[\\"context.visible\\",\\"_attractor.node_outcomes\\"]"]
                done [shape=Msquare]

                start -> plan
                plan -> done [condition="outcome=fail"]
            }
            """
        )

        with tempfile.TemporaryDirectory() as tmp:
            logs_root = Path(tmp) / "logs"

            def runner(node_id: str, prompt: str, context: Context, *, emit_event=None) -> Outcome:
                if node_id == "plan":
                    return Outcome(
                        status=OutcomeStatus.FAIL,
                        notes="invalid envelope",
                        failure_reason="invalid structured status envelope: notes must be a string",
                        failure_kind=FailureKind.CONTRACT,
                    )
                return Outcome(status=OutcomeStatus.SUCCESS)

            result = PipelineExecutor(
                graph,
                runner,
                logs_root=str(logs_root),
            ).run(Context())

            assert result.status == "completed"
            status_payload = json.loads((logs_root / "plan" / "status.json").read_text(encoding="utf-8"))
            assert status_payload["outcome"] == "fail"
            assert status_payload["failure_kind"] == "contract"

    def test_status_json_includes_runtime_failure_kind_for_runner_exception(self):
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

        with tempfile.TemporaryDirectory() as tmp:
            logs_root = Path(tmp) / "logs"

            def runner(node_id: str, prompt: str, context: Context, *, emit_event=None) -> Outcome:
                if node_id == "plan":
                    raise RuntimeError("runner exploded")
                return Outcome(status=OutcomeStatus.SUCCESS)

            result = PipelineExecutor(
                graph,
                runner,
                logs_root=str(logs_root),
            ).run(Context())

            assert result.status == "failed"
            plan_outcome = result.node_outcomes["plan"]
            assert plan_outcome.status == OutcomeStatus.FAIL
            assert plan_outcome.failure_reason == "runner exploded"
            assert plan_outcome.failure_kind == FailureKind.RUNTIME

            status_payload = json.loads((logs_root / "plan" / "status.json").read_text(encoding="utf-8"))
            assert status_payload["outcome"] == "fail"
            assert status_payload["failure_kind"] == "runtime"

    def test_stage_response_artifact_prefers_raw_response_text_when_present(self):
        graph = parse_dot(
            """
            digraph G {
                start [shape=Mdiamond]
                plan [shape=box]
                done [shape=Msquare]

                start -> plan
                plan -> done [condition="outcome=fail"]
            }
            """
        )

        raw_response = '{"outcome":"fail","notes":"needs fixes"}'

        with tempfile.TemporaryDirectory() as tmp:
            logs_root = Path(tmp) / "logs"

            def runner(node_id: str, prompt: str, context: Context, *, emit_event=None) -> Outcome:
                if node_id == "plan":
                    return Outcome(
                        status=OutcomeStatus.FAIL,
                        notes="needs fixes",
                        raw_response_text=raw_response,
                    )
                return Outcome(status=OutcomeStatus.SUCCESS)

            result = PipelineExecutor(
                graph,
                runner,
                logs_root=str(logs_root),
            ).run(Context())

            assert result.status == "completed"
            response_path = logs_root / "plan" / "response.md"
            assert response_path.read_text(encoding="utf-8").strip() == raw_response
            status_payload = json.loads((logs_root / "plan" / "status.json").read_text(encoding="utf-8"))
            assert status_payload["notes"] == "needs fixes"

    def test_status_json_contract_constrains_invalid_outcome_enum_and_optional_field_values(self):
        graph = parse_dot(
            """
            digraph G {
                start [shape=Mdiamond]
                plan [shape=box]
                done [shape=Msquare]

                start -> plan
                plan -> done [condition="outcome=fail"]
            }
            """
        )

        with tempfile.TemporaryDirectory() as tmp:
            logs_root = Path(tmp) / "logs"

            def runner(node_id: str, prompt: str, context: Context, *, emit_event=None) -> Outcome:
                if node_id == "plan":
                    return Outcome(
                        status="not-a-valid-status",  # type: ignore[arg-type]
                        suggested_next_ids="done",  # type: ignore[arg-type]
                        context_updates="bad-updates",  # type: ignore[arg-type]
                        notes=123,  # type: ignore[arg-type]
                    )
                return Outcome(status=OutcomeStatus.SUCCESS)

            result = PipelineExecutor(
                graph,
                runner,
                logs_root=str(logs_root),
            ).run(Context())

            assert result.status == "completed"
            plan_outcome = result.node_outcomes["plan"]
            assert plan_outcome.status == OutcomeStatus.FAIL
            assert "invalid outcome status" in plan_outcome.failure_reason

            status_payload = json.loads((logs_root / "plan" / "status.json").read_text(encoding="utf-8"))
            assert status_payload["outcome"] == "fail"
            assert status_payload["preferred_label"] == ""
            assert status_payload["suggested_next_ids"] == []
            assert status_payload["context_updates"] == {}
            assert status_payload["notes"] == "123"

    def test_status_json_contract_rejects_skipped_enum_value_for_stage_outcome(self):
        graph = parse_dot(
            """
            digraph G {
                start [shape=Mdiamond]
                plan [shape=box]
                done [shape=Msquare]

                start -> plan
                plan -> done [condition="outcome=fail"]
            }
            """
        )

        with tempfile.TemporaryDirectory() as tmp:
            logs_root = Path(tmp) / "logs"

            def runner(node_id: str, prompt: str, context: Context, *, emit_event=None) -> Outcome:
                if node_id == "plan":
                    return Outcome(status=OutcomeStatus.SKIPPED, notes="condition not met")
                return Outcome(status=OutcomeStatus.SUCCESS)

            result = PipelineExecutor(
                graph,
                runner,
                logs_root=str(logs_root),
            ).run(Context())

            assert result.status == "completed"
            plan_outcome = result.node_outcomes["plan"]
            assert plan_outcome.status == OutcomeStatus.FAIL
            assert "invalid outcome status" in plan_outcome.failure_reason

            status_payload = json.loads((logs_root / "plan" / "status.json").read_text(encoding="utf-8"))
            assert status_payload["outcome"] == "fail"
            assert status_payload["notes"] == "condition not met"

    def test_status_json_context_updates_match_engine_merge_behavior(self):
        graph = parse_dot(
            """
            digraph G {
                start [shape=Mdiamond]
                plan [shape=box, spark.writes_context="[\\"context.visible\\",\\"_attractor.node_outcomes\\"]"]
                done [shape=Msquare]

                start -> plan
                plan -> done
            }
            """
        )

        with tempfile.TemporaryDirectory() as tmp:
            logs_root = Path(tmp) / "logs"

            def runner(node_id: str, prompt: str, context: Context, *, emit_event=None) -> Outcome:
                if node_id == "plan":
                    return Outcome(
                        status=OutcomeStatus.SUCCESS,
                        context_updates={
                            "context.visible": "yes",
                            "_attractor.node_outcomes": {"plan": "overwritten"},
                        },
                    )
                return Outcome(status=OutcomeStatus.SUCCESS)

            result = PipelineExecutor(
                graph,
                runner,
                logs_root=str(logs_root),
            ).run(Context())

            assert result.status == "completed"
            assert result.context["context.visible"] == "yes"
            assert result.context["_attractor.node_outcomes"]["plan"] == "success"

            status_payload = json.loads((logs_root / "plan" / "status.json").read_text(encoding="utf-8"))
            assert status_payload["context_updates"] == {"context.visible": "yes"}

    def test_artifacts_and_checkpoint_written_each_step(self):
        graph = parse_dot(
            """
            digraph G {
                start [shape=Mdiamond, prompt="start"]
                plan [shape=box, prompt="plan prompt"]
                done [shape=Msquare]

                start -> plan
                plan -> done
            }
            """
        )

        with tempfile.TemporaryDirectory() as tmp:
            logs_root = Path(tmp) / "logs"
            checkpoint_file = Path(tmp) / "attractor.state.json"

            def runner(node_id: str, prompt: str, context: Context, *, emit_event=None) -> Outcome:
                return Outcome(status=OutcomeStatus.SUCCESS, notes=f"response for {node_id}")

            result = PipelineExecutor(
                graph,
                runner,
                logs_root=str(logs_root),
                checkpoint_file=str(checkpoint_file),
            ).run(Context())

            assert result.status == "completed"
            assert result.current_node == "done"

            # Artifacts for non-terminal stages.
            for node_id in ["start", "plan"]:
                stage = logs_root / node_id
                assert (stage / "prompt.md").exists()
                assert (stage / "response.md").exists()
                assert (stage / "status.json").exists()

                payload = json.loads((stage / "status.json").read_text(encoding="utf-8"))
                assert payload["outcome"] == "success"
                assert "notes" in payload

            checkpoint = load_checkpoint(checkpoint_file)
            assert checkpoint is not None
            assert checkpoint.current_node == "done"
            assert checkpoint.completed_nodes == ["start", "plan"]

    def test_checkpoint_json_persists_timestamp_retry_context_and_logs(self):
        graph = parse_dot(
            """
            digraph G {
                start [shape=Mdiamond]
                done [shape=Msquare]
                start -> done
            }
            """
        )

        with tempfile.TemporaryDirectory() as tmp:
            checkpoint_file = Path(tmp) / "attractor.state.json"

            def runner(node_id: str, prompt: str, context: Context, *, emit_event=None) -> Outcome:
                return Outcome(status=OutcomeStatus.SUCCESS)

            context = Context()
            context.append_log("bootstrap")

            result = PipelineExecutor(
                graph,
                runner,
                checkpoint_file=str(checkpoint_file),
            ).run(context)

            assert result.status == "completed"
            raw_checkpoint = json.loads(checkpoint_file.read_text(encoding="utf-8"))
            assert raw_checkpoint["current_node"] == "done"
            assert raw_checkpoint["completed_nodes"] == ["start"]
            assert raw_checkpoint["retry_counts"] == {}
            assert isinstance(raw_checkpoint["context"], dict)
            assert raw_checkpoint["logs"] == ["bootstrap"]
            assert isinstance(raw_checkpoint["timestamp"], str)
            assert raw_checkpoint["timestamp"]

    def test_status_json_persists_stage_status_transitions_across_retries(self):
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

        with tempfile.TemporaryDirectory() as tmp:
            logs_root = Path(tmp) / "logs"
            work_calls = {"count": 0}

            def runner(node_id: str, prompt: str, context: Context, *, emit_event=None) -> Outcome:
                if node_id == "work":
                    work_calls["count"] += 1
                    if work_calls["count"] == 1:
                        return Outcome(status=OutcomeStatus.RETRY, notes="transient")
                    return Outcome(status=OutcomeStatus.SUCCESS, notes="recovered")
                return Outcome(status=OutcomeStatus.SUCCESS)

            result = PipelineExecutor(graph, runner, logs_root=str(logs_root)).run(Context())

            assert result.status == "completed"
            status_payload = json.loads((logs_root / "work" / "status.json").read_text(encoding="utf-8"))
            assert status_payload["outcome"] == "success"
            assert status_payload["status_transitions"] == ["retry", "success"]

    def test_resume_from_checkpoint(self):
        graph = parse_dot(
            """
            digraph G {
                start [shape=Mdiamond]
                plan [shape=box, prompt="plan"]
                review [shape=box, prompt="review"]
                done [shape=Msquare]

                start -> plan
                plan -> review
                review -> done
            }
            """
        )

        with tempfile.TemporaryDirectory() as tmp:
            logs_root = Path(tmp) / "logs"
            checkpoint_file = Path(tmp) / "attractor.state.json"
            calls = []

            def runner(node_id: str, prompt: str, context: Context, *, emit_event=None) -> Outcome:
                calls.append(node_id)
                return Outcome(status=OutcomeStatus.SUCCESS, notes=node_id)

            executor = PipelineExecutor(
                graph,
                runner,
                logs_root=str(logs_root),
                checkpoint_file=str(checkpoint_file),
            )

            paused = executor.run(Context(), max_steps=1)
            assert paused.status == "paused"
            assert paused.current_node == "plan"
            assert paused.completed_nodes == ["start"]

            resumed = executor.run(Context(), resume=True)
            assert resumed.status == "completed"
            assert resumed.current_node == "done"

            # start executes once; resume continues at plan.
            assert calls == ["start", "plan", "review"]

    def test_resume_from_checkpoint_continues_from_next_node_after_last_completed(self):
        graph = parse_dot(
            """
            digraph G {
                start [shape=Mdiamond]
                plan [shape=box, prompt="plan"]
                review [shape=box, prompt="review"]
                done [shape=Msquare]

                start -> plan
                plan -> review
                review -> done
            }
            """
        )

        with tempfile.TemporaryDirectory() as tmp:
            checkpoint_file = Path(tmp) / "attractor.state.json"
            calls = []

            def runner(node_id: str, prompt: str, context: Context, *, emit_event=None) -> Outcome:
                calls.append(node_id)
                return Outcome(status=OutcomeStatus.SUCCESS, notes=node_id)

            save_checkpoint(
                checkpoint_file,
                Checkpoint(
                    current_node="plan",
                    completed_nodes=["start", "plan"],
                    context={"outcome": "success"},
                ),
            )

            executor = PipelineExecutor(
                graph,
                runner,
                checkpoint_file=str(checkpoint_file),
            )

            resumed = executor.run(Context(), resume=True)
            assert resumed.status == "completed"
            assert resumed.current_node == "done"
            assert resumed.completed_nodes == ["start", "plan", "review"]
            assert calls == ["review"]

    def test_resume_degrades_first_stage_fidelity_after_full_checkpoint_hop(self):
        graph = parse_dot(
            """
            digraph G {
                graph [default_fidelity="full"]
                start [shape=Mdiamond]
                plan [shape=box, prompt="plan"]
                review [shape=box, prompt="review"]
                done [shape=Msquare]

                start -> plan
                plan -> review
                review -> done
            }
            """
        )

        with tempfile.TemporaryDirectory() as tmp:
            checkpoint_file = Path(tmp) / "attractor.state.json"
            seen_fidelity: list[str] = []

            def runner(node_id: str, prompt: str, context: Context, *, emit_event=None) -> Outcome:
                seen_fidelity.append(str(context.get("_attractor.runtime.fidelity", "")))
                return Outcome(status=OutcomeStatus.SUCCESS, notes=node_id)

            executor = PipelineExecutor(
                graph,
                runner,
                checkpoint_file=str(checkpoint_file),
            )

            paused = executor.run(Context(), max_steps=1)
            assert paused.status == "paused"

            resumed = executor.run(Context(), resume=True)
            assert resumed.status == "completed"
            assert seen_fidelity == ["full", "summary:high", "full"]

    def test_resume_restores_retry_counters_and_checkpoint_context_exactly(self):
        graph = parse_dot(
            """
            digraph G {
                start [shape=Mdiamond]
                work [shape=box, max_retries=2]
                done [shape=Msquare]

                start -> work
                work -> done [condition="outcome=success"]
            }
            """
        )

        with tempfile.TemporaryDirectory() as tmp:
            checkpoint_file = Path(tmp) / "attractor.state.json"
            attempts = {"work": 0}
            seen_resume_markers: list[str] = []

            save_checkpoint(
                checkpoint_file,
                Checkpoint(
                    current_node="work",
                    completed_nodes=["start"],
                    context={
                        "outcome": "retry",
                        "preferred_label": "",
                        "context.resume.marker": "from-checkpoint",
                    },
                    retry_counts={"work": 2},
                ),
            )

            def runner(node_id: str, prompt: str, context: Context, *, emit_event=None) -> Outcome:
                if node_id == "work":
                    attempts["work"] += 1
                    seen_resume_markers.append(str(context.get("context.resume.marker", "")))
                    return Outcome(status=OutcomeStatus.RETRY, notes="still flaky")
                return Outcome(status=OutcomeStatus.SUCCESS)

            executor = PipelineExecutor(
                graph,
                runner,
                checkpoint_file=str(checkpoint_file),
            )

            resumed = executor.run(
                Context(values={"context.resume.marker": "from-input-context"}),
                resume=True,
            )

            assert resumed.status == "failed"
            assert resumed.current_node == "work"
            assert attempts["work"] == 1
            assert seen_resume_markers == ["from-checkpoint"]
            assert resumed.context["context.resume.marker"] == "from-checkpoint"

    def test_checkpoint_updates_after_stage_completion_before_successful_no_route_completion(self):
        graph = parse_dot(
            """
            digraph G {
                start [shape=Mdiamond]
                plan [shape=box, prompt="plan"]
                done [shape=Msquare]

                start -> plan
            }
            """
        )

        with tempfile.TemporaryDirectory() as tmp:
            checkpoint_file = Path(tmp) / "attractor.state.json"

            def runner(node_id: str, prompt: str, context: Context, *, emit_event=None) -> Outcome:
                return Outcome(status=OutcomeStatus.SUCCESS, notes=node_id)

            executor = PipelineExecutor(
                graph,
                runner,
                checkpoint_file=str(checkpoint_file),
            )

            result = executor.run(Context())

            assert result.status == "completed"
            assert result.current_node == "plan"
            assert result.completed_nodes == ["start", "plan"]

            checkpoint = load_checkpoint(checkpoint_file)
            assert checkpoint is not None
            assert checkpoint.current_node == "plan"
            assert checkpoint.completed_nodes == ["start", "plan"]

    def test_finalize_persists_checkpoint_and_failure_event_when_runner_exception_becomes_fail_outcome(self):
        graph = parse_dot(
            """
            digraph G {
                start [shape=Mdiamond, max_retries=0]
                done [shape=Msquare]
                start -> done [condition="outcome=success"]
            }
            """
        )

        with tempfile.TemporaryDirectory() as tmp:
            checkpoint_file = Path(tmp) / "attractor.state.json"
            events = []

            def runner(node_id: str, prompt: str, context: Context, *, emit_event=None) -> Outcome:
                raise RuntimeError("runner exploded")

            executor = PipelineExecutor(
                graph,
                runner,
                checkpoint_file=str(checkpoint_file),
                on_event=events.append,
            )

            result = executor.run(Context())

            assert result.status == "failed"
            assert result.current_node == "start"
            assert result.failure_reason == "runner exploded"

            checkpoint = load_checkpoint(checkpoint_file)
            assert checkpoint is not None
            assert checkpoint.current_node == "start"
            assert checkpoint.completed_nodes == ["start"]
            assert checkpoint.context["outcome"] == "fail"

            event_types = [event["type"] for event in events]
            assert "CheckpointSaved" in event_types
            assert event_types[-1] == "PipelineFailed"

    def test_finalize_cleans_up_runner_and_control_for_run_and_run_from(self):
        graph = parse_dot(
            """
            digraph G {
                start [shape=Mdiamond]
                done [shape=Msquare]
                start -> done
            }
            """
        )

        class ClosableRunner:
            def __init__(self) -> None:
                self.closed = 0

            def __call__(self, node_id: str, prompt: str, context: Context) -> Outcome:
                return Outcome(status=OutcomeStatus.SUCCESS)

            def close(self) -> None:
                self.closed += 1

        class ClosableControl:
            def __init__(self) -> None:
                self.closed = 0

            def __call__(self) -> str | None:
                return None

            def close(self) -> None:
                self.closed += 1

        runner = ClosableRunner()
        control = ClosableControl()
        run_result = PipelineExecutor(graph, runner, control=control).run(Context())

        assert run_result.status == "completed"
        assert runner.closed == 1
        assert control.closed == 1

        runner = ClosableRunner()
        control = ClosableControl()
        run_from_result = PipelineExecutor(graph, runner, control=control).run_from("start", Context())

        assert run_from_result.status == "completed"
        assert runner.closed == 1
        assert control.closed == 1

    def test_run_from_writes_terminal_stage_status_artifact(self):
        graph = parse_dot(
            """
            digraph G {
                start [shape=Mdiamond]
                done [shape=Msquare]
                start -> done
            }
            """
        )

        with tempfile.TemporaryDirectory() as tmp:
            logs_root = Path(tmp) / "logs"

            def runner(node_id: str, prompt: str, context: Context, *, emit_event=None) -> Outcome:
                return Outcome(status=OutcomeStatus.SUCCESS, notes=f"{node_id} complete")

            result = PipelineExecutor(
                graph,
                runner,
                logs_root=str(logs_root),
            ).run_from("start", Context())

            assert result.status == "completed"
            terminal_status_path = logs_root / "done" / "status.json"
            assert terminal_status_path.is_file()
            terminal_status = json.loads(terminal_status_path.read_text(encoding="utf-8"))
            assert terminal_status["outcome"] == "success"

    def test_run_from_saves_checkpoint_after_each_stage_with_current_node_and_completed_list(self, monkeypatch):
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

        with tempfile.TemporaryDirectory() as tmp:
            checkpoint_file = Path(tmp) / "attractor.state.json"
            snapshots: list[tuple[str, list[str]]] = []
            original_save_checkpoint = executor_module.save_checkpoint

            def capture_checkpoint(path: Path, checkpoint):
                snapshots.append((checkpoint.current_node, list(checkpoint.completed_nodes)))
                original_save_checkpoint(path, checkpoint)

            monkeypatch.setattr(executor_module, "save_checkpoint", capture_checkpoint)

            def runner(node_id: str, prompt: str, context: Context, *, emit_event=None) -> Outcome:
                return Outcome(status=OutcomeStatus.SUCCESS, notes=node_id)

            result = PipelineExecutor(
                graph,
                runner,
                checkpoint_file=str(checkpoint_file),
            ).run_from("start", Context())

            assert result.status == "completed"
            assert ("start", ["start"]) in snapshots
            assert ("plan", ["start", "plan"]) in snapshots

    def test_run_from_persists_retry_checkpoint_with_retry_counts_and_context(self, monkeypatch):
        graph = parse_dot(
            """
            digraph G {
                start [shape=Mdiamond, spark.writes_context="[\\"context.shared\\"]"]
                work [shape=box, max_retries=1, spark.writes_context="[\\"context.retry.phase\\"]"]
                done [shape=Msquare]
                start -> work
                work -> done
            }
            """
        )

        with tempfile.TemporaryDirectory() as tmp:
            checkpoint_file = Path(tmp) / "attractor.state.json"
            calls = {"work": 0}
            snapshots: list[Checkpoint] = []
            original_save_checkpoint = executor_module.save_checkpoint

            def capture_checkpoint(path: Path, checkpoint: Checkpoint):
                snapshots.append(checkpoint)
                original_save_checkpoint(path, checkpoint)

            monkeypatch.setattr(executor_module, "save_checkpoint", capture_checkpoint)

            def runner(node_id: str, prompt: str, context: Context, *, emit_event=None) -> Outcome:
                if node_id == "start":
                    return Outcome(
                        status=OutcomeStatus.SUCCESS,
                        context_updates={"context.shared": "visible-to-work"},
                    )

                if node_id == "work":
                    calls["work"] += 1
                    assert context.get("context.shared") == "visible-to-work"
                    if calls["work"] == 1:
                        return Outcome(
                            status=OutcomeStatus.RETRY,
                            context_updates={"context.retry.phase": "first-attempt"},
                        )
                    assert context.get("context.retry.phase") == "first-attempt"
                    return Outcome(
                        status=OutcomeStatus.SUCCESS,
                        context_updates={"context.retry.phase": "second-attempt"},
                    )

                return Outcome(status=OutcomeStatus.SUCCESS)

            result = PipelineExecutor(
                graph,
                runner,
                checkpoint_file=str(checkpoint_file),
            ).run_from("start", Context(values={"context.seed": "ready"}))

            assert result.status == "completed"
            retry_snapshots = [snap for snap in snapshots if snap.current_node == "work" and snap.retry_counts]
            assert retry_snapshots, "expected checkpoint persistence for work retry attempt"
            retry_snapshot = retry_snapshots[0]
            assert retry_snapshot.retry_counts == {"work": 1}
            assert retry_snapshot.completed_nodes == ["start"]
            assert retry_snapshot.context["context.shared"] == "visible-to-work"
            assert retry_snapshot.context["context.retry.phase"] == "first-attempt"
