import json
from pathlib import Path
import tempfile

import pytest

from attractor.dsl import parse_dot
from attractor.engine import load_checkpoint
import attractor.engine.executor as executor_module
from attractor.engine.context import Context
from attractor.engine.executor import PipelineExecutor
from attractor.engine.outcome import Outcome, OutcomeStatus


class TestCheckpointAndArtifacts:
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

            assert result.status == "success"
            assert result.node_outcomes["plan"].status == OutcomeStatus.SUCCESS
            status_payload = json.loads((logs_root / "plan" / "status.json").read_text(encoding="utf-8"))
            assert status_payload["outcome"] == "success"
            assert status_payload["notes"] == "auto-status: handler completed without writing status"

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

            def runner(node_id: str, prompt: str, context: Context) -> Outcome:
                return Outcome(status=OutcomeStatus.SUCCESS, notes=f"response for {node_id}")

            result = PipelineExecutor(
                graph,
                runner,
                logs_root=str(logs_root),
                checkpoint_file=str(checkpoint_file),
            ).run(Context())

            assert result.status == "success"
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

            def runner(node_id: str, prompt: str, context: Context) -> Outcome:
                return Outcome(status=OutcomeStatus.SUCCESS)

            context = Context()
            context.append_log("bootstrap")

            result = PipelineExecutor(
                graph,
                runner,
                checkpoint_file=str(checkpoint_file),
            ).run(context)

            assert result.status == "success"
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

            def runner(node_id: str, prompt: str, context: Context) -> Outcome:
                if node_id == "work":
                    work_calls["count"] += 1
                    if work_calls["count"] == 1:
                        return Outcome(status=OutcomeStatus.RETRY, notes="transient")
                    return Outcome(status=OutcomeStatus.SUCCESS, notes="recovered")
                return Outcome(status=OutcomeStatus.SUCCESS)

            result = PipelineExecutor(graph, runner, logs_root=str(logs_root)).run(Context())

            assert result.status == "success"
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

            def runner(node_id: str, prompt: str, context: Context) -> Outcome:
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
            assert resumed.status == "success"
            assert resumed.current_node == "done"

            # start executes once; resume continues at plan.
            assert calls == ["start", "plan", "review"]

    def test_checkpoint_updates_after_stage_completion_before_routing_error(self):
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

            def runner(node_id: str, prompt: str, context: Context) -> Outcome:
                return Outcome(status=OutcomeStatus.SUCCESS, notes=node_id)

            executor = PipelineExecutor(
                graph,
                runner,
                checkpoint_file=str(checkpoint_file),
            )

            try:
                executor.run(Context())
                raise AssertionError("Expected pipeline execution to fail on missing outgoing edge")
            except RuntimeError as exc:
                assert "no eligible outgoing edge" in str(exc)

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

            def runner(node_id: str, prompt: str, context: Context) -> Outcome:
                raise RuntimeError("runner exploded")

            executor = PipelineExecutor(
                graph,
                runner,
                checkpoint_file=str(checkpoint_file),
                on_event=events.append,
            )

            result = executor.run(Context())

            assert result.status == "fail"
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

        assert run_result.status == "success"
        assert runner.closed == 1
        assert control.closed == 1

        runner = ClosableRunner()
        control = ClosableControl()
        run_from_result = PipelineExecutor(graph, runner, control=control).run_from("start", Context())

        assert run_from_result.status == "success"
        assert runner.closed == 1
        assert control.closed == 1

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

            def runner(node_id: str, prompt: str, context: Context) -> Outcome:
                return Outcome(status=OutcomeStatus.SUCCESS, notes=node_id)

            result = PipelineExecutor(
                graph,
                runner,
                checkpoint_file=str(checkpoint_file),
            ).run_from("start", Context())

            assert result.status == "success"
            assert ("start", ["start"]) in snapshots
            assert ("plan", ["start", "plan"]) in snapshots
