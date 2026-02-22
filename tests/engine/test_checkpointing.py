import json
from pathlib import Path
import tempfile
import unittest

from attractor.dsl import parse_dot
from attractor.engine import load_checkpoint
from attractor.engine.context import Context
from attractor.engine.executor import PipelineExecutor
from attractor.engine.outcome import Outcome, OutcomeStatus


class TestCheckpointAndArtifacts(unittest.TestCase):
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

            self.assertEqual("success", result.status)
            self.assertEqual("done", result.current_node)

            # Artifacts for non-terminal stages.
            for node_id in ["start", "plan"]:
                stage = logs_root / node_id
                self.assertTrue((stage / "prompt.md").exists())
                self.assertTrue((stage / "response.md").exists())
                self.assertTrue((stage / "status.json").exists())

                payload = json.loads((stage / "status.json").read_text(encoding="utf-8"))
                self.assertEqual("success", payload["outcome"])
                self.assertIn("notes", payload)

            checkpoint = load_checkpoint(checkpoint_file)
            self.assertIsNotNone(checkpoint)
            self.assertEqual("done", checkpoint.current_node)
            self.assertEqual(["start", "plan"], checkpoint.completed_nodes)

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
            self.assertEqual("paused", paused.status)
            self.assertEqual("plan", paused.current_node)
            self.assertEqual(["start"], paused.completed_nodes)

            resumed = executor.run(Context(), resume=True)
            self.assertEqual("success", resumed.status)
            self.assertEqual("done", resumed.current_node)

            # start executes once; resume continues at plan.
            self.assertEqual(["start", "plan", "review"], calls)


if __name__ == "__main__":
    unittest.main()
