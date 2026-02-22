import unittest

from attractor.dsl import parse_dot
from attractor.engine.context import Context
from attractor.engine.outcome import OutcomeStatus
from attractor.handlers import HandlerRunner, build_default_registry


class _StubBackend:
    def __init__(self, ok: bool = True):
        self.ok = ok
        self.calls = []

    def run(self, node_id: str, prompt: str, context: Context) -> bool:
        self.calls.append((node_id, prompt, dict(context.values)))
        return self.ok


class TestBuiltInHandlers(unittest.TestCase):
    def test_registry_resolution_by_shape_and_type(self):
        graph = parse_dot(
            """
            digraph G {
                start [shape=Mdiamond]
                human [shape=hexagon]
                custom [shape=box, type="tool", tool_command="printf hi"]
                done [shape=Msquare]
                start -> human [label="Approve"]
                human -> custom [label="Go"]
                custom -> done
            }
            """
        )

        registry = build_default_registry(codergen_backend=_StubBackend())
        self.assertEqual("start", registry.resolve_handler_type(graph.nodes["start"]))
        self.assertEqual("wait.human", registry.resolve_handler_type(graph.nodes["human"]))
        self.assertEqual("tool", registry.resolve_handler_type(graph.nodes["custom"]))

    def test_codergen_handler_calls_backend(self):
        graph = parse_dot(
            """
            digraph G {
                start [shape=Mdiamond]
                task [shape=box, prompt="Plan for $goal"]
                done [shape=Msquare]
                start -> task
                task -> done
            }
            """
        )

        backend = _StubBackend(ok=True)
        registry = build_default_registry(codergen_backend=backend)
        runner = HandlerRunner(graph, registry)
        ctx = Context(values={"graph.goal": "ship"})

        outcome = runner("task", "Plan for $goal", ctx)
        self.assertEqual(OutcomeStatus.SUCCESS, outcome.status)
        self.assertEqual("Plan for ship", backend.calls[0][1])

    def test_wait_human_uses_interviewer_and_sets_preferred_label(self):
        graph = parse_dot(
            """
            digraph G {
                gate [shape=hexagon, prompt="Choose"]
                pass [shape=box]
                fail [shape=box]
                gate -> pass [label="Approve"]
                gate -> fail [label="Fix"]
            }
            """
        )

        registry = build_default_registry(codergen_backend=_StubBackend())
        runner = HandlerRunner(graph, registry)

        outcome = runner("gate", "Choose", Context())
        self.assertEqual(OutcomeStatus.SUCCESS, outcome.status)
        self.assertEqual("Approve", outcome.preferred_label)

    def test_tool_handler_executes_command(self):
        graph = parse_dot(
            """
            digraph G {
                tool_node [shape=parallelogram, tool_command="printf hello"]
            }
            """
        )
        registry = build_default_registry(codergen_backend=_StubBackend())
        runner = HandlerRunner(graph, registry)
        outcome = runner("tool_node", "", Context())
        self.assertEqual(OutcomeStatus.SUCCESS, outcome.status)
        self.assertIn("hello", outcome.notes)


if __name__ == "__main__":
    unittest.main()
