from attractor.dsl import parse_dot
from attractor.engine.context import Context
from attractor.engine.outcome import Outcome
from attractor.engine.outcome import OutcomeStatus
from attractor.handlers.base import CodergenBackend
from attractor.handlers import HandlerRunner, build_default_registry
from attractor.interviewer import Answer, Interviewer, Question


class _StubBackend:
    def __init__(self, ok: bool = True):
        self.ok = ok
        self.calls = []

    def run(self, node_id: str, prompt: str, context: Context, *, timeout=None) -> bool:
        self.calls.append((node_id, prompt, dict(context.values)))
        return self.ok


class _PluginHandler:
    def run(self, runtime):
        return Outcome(status=OutcomeStatus.SUCCESS, notes=f"plugin:{runtime.node_id}")


class _FalseyInterviewer(Interviewer):
    def __bool__(self) -> bool:
        return False

    def ask(self, question: Question) -> Answer:
        return Answer(selected_values=["Fix"])


class TestBuiltInHandlers:
    def test_codergen_backend_protocol_is_structural(self):
        class _AltBackend:
            def run(self, node_id: str, prompt: str, context: Context, *, timeout=None) -> bool:
                return True

        assert isinstance(_StubBackend(), CodergenBackend)
        assert isinstance(_AltBackend(), CodergenBackend)

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
        assert registry.resolve_handler_type(graph.nodes["start"]) == "start"
        assert registry.resolve_handler_type(graph.nodes["human"]) == "wait.human"
        assert registry.resolve_handler_type(graph.nodes["custom"]) == "tool"

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
        assert outcome.status == OutcomeStatus.SUCCESS
        assert backend.calls[0][1] == "Plan for ship"

    def test_codergen_handler_falls_back_to_label_when_prompt_is_empty(self):
        graph = parse_dot(
            """
            digraph G {
                task [shape=box, label="Label Prompt"]
            }
            """
        )

        backend = _StubBackend(ok=True)
        registry = build_default_registry(codergen_backend=backend)
        runner = HandlerRunner(graph, registry)

        outcome = runner("task", "", Context())
        assert outcome.status == OutcomeStatus.SUCCESS
        assert backend.calls[0][1] == "Label Prompt"

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
        assert outcome.status == OutcomeStatus.SUCCESS
        assert outcome.preferred_label == "Approve"

    def test_wait_human_uses_falsey_external_interviewer(self):
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

        registry = build_default_registry(
            codergen_backend=_StubBackend(),
            interviewer=_FalseyInterviewer(),
        )
        runner = HandlerRunner(graph, registry)

        outcome = runner("gate", "Choose", Context())
        assert outcome.status == OutcomeStatus.SUCCESS
        assert outcome.preferred_label == "Fix"

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
        assert outcome.status == OutcomeStatus.SUCCESS
        assert "hello" in outcome.notes

    def test_registry_allows_plugin_injection_via_builder(self):
        graph = parse_dot(
            """
            digraph G {
                plugin_stage [shape=box, type="custom.plugin"]
            }
            """
        )
        registry = build_default_registry(
            codergen_backend=_StubBackend(),
            extra_handlers={"custom.plugin": _PluginHandler()},
        )
        runner = HandlerRunner(graph, registry)
        outcome = runner("plugin_stage", "", Context())
        assert outcome.status == OutcomeStatus.SUCCESS
        assert outcome.notes == "plugin:plugin_stage"

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
