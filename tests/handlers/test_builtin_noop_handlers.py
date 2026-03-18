from attractor.dsl import parse_dot
from attractor.engine.context import Context
from attractor.engine.outcome import OutcomeStatus
from attractor.handlers import HandlerRunner, build_default_registry

from tests.handlers._support.fakes import _StubBackend

class TestBuiltInNoopHandlers:
    def test_conditional_handler_is_noop_success(self):
        graph = parse_dot(
            """
            digraph G {
                gate [shape=diamond]
            }
            """
        )
        registry = build_default_registry(codergen_backend=_StubBackend())
        runner = HandlerRunner(graph, registry)

        outcome = runner("gate", "", Context(values={"outcome": "fail"}))
        assert outcome.status == OutcomeStatus.SUCCESS

    def test_start_handler_is_noop_success(self):
        graph = parse_dot(
            """
            digraph G {
                start [shape=Mdiamond]
            }
            """
        )
        registry = build_default_registry(codergen_backend=_StubBackend())
        runner = HandlerRunner(graph, registry)

        outcome = runner("start", "", Context(values={"goal": "ship"}))

        assert outcome.status == OutcomeStatus.SUCCESS

    def test_exit_handler_is_noop_success(self):
        graph = parse_dot(
            """
            digraph G {
                done [shape=Msquare]
            }
            """
        )
        registry = build_default_registry(codergen_backend=_StubBackend())
        runner = HandlerRunner(graph, registry)

        outcome = runner("done", "", Context(values={"outcome": "fail"}))

        assert outcome.status == OutcomeStatus.SUCCESS
