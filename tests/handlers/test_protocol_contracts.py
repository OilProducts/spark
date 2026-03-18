from attractor.dsl import parse_dot
from attractor.engine.context import Context
from attractor.engine.outcome import OutcomeStatus
from attractor.handlers import HandlerRunner, build_default_registry

from tests.handlers._support.fakes import _StubBackend

class TestProtocolContracts:
    def test_house_shape_resolves_and_executes_with_default_registry(self):
        graph = parse_dot(
            """
            digraph G {
                manager [shape=house, manager.max_cycles=1, manager.poll_interval=0ms]
            }
            """
        )
        registry = build_default_registry(codergen_backend=_StubBackend())
        runner = HandlerRunner(graph, registry)

        assert registry.resolve_handler_type(graph.nodes["manager"]) == "stack.manager_loop"
        outcome = runner("manager", "", Context())
        assert outcome.status == OutcomeStatus.FAIL
        assert "Max cycles exceeded" in outcome.failure_reason
