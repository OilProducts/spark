import json
from pathlib import Path
import tempfile

from attractor.dsl import parse_dot
from attractor.engine import load_checkpoint
from attractor.engine.context import Context
from attractor.engine.executor import PipelineExecutor
from attractor.engine.outcome import Outcome, OutcomeStatus


class TestCheckpointAndArtifacts:
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
