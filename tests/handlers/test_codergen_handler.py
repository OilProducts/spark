import json
import tempfile
from pathlib import Path

from attractor.dsl import parse_dot
from attractor.engine.context import Context
from attractor.engine.outcome import FailureKind, Outcome, OutcomeStatus
from attractor.handlers import HandlerRunner, build_default_registry
from attractor.handlers.builtin.codergen import STATUS_ENVELOPE_PROMPT_APPENDIX
from attractor.llm_runtime import RUNTIME_LAUNCH_MODEL_KEY
from attractor.transforms import ModelStylesheetTransform, RuntimePreambleTransform, TransformPipeline

from tests.handlers._support.fakes import (
    _StubBackend,
    _ArtifactProbeBackend,
    _TextBackend,
    _OutcomeBackend,
    _StageLoggingBackend,
)


class TestCodergenHandler:
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
        assert backend.calls[0][3] == ""
        assert backend.calls[0][4] == 0

    def test_codergen_handler_prepends_runtime_carryover_to_rendered_prompt(self):
        graph = parse_dot(
            """
            digraph G {
                task [shape=box, prompt="Plan for $goal"]
            }
            """
        )

        backend = _StubBackend(ok=True)
        registry = build_default_registry(codergen_backend=backend)
        runner = HandlerRunner(graph, registry)
        ctx = Context(
            values={
                "graph.goal": "ship",
                "_attractor.runtime.context_carryover": "carryover:summary:high\ncontext.review.required_changes=add tests",
            }
        )

        outcome = runner("task", "Plan for $goal", ctx)

        assert outcome.status == OutcomeStatus.SUCCESS
        assert backend.calls[0][1] == (
            "Context carryover:\n\n"
            "carryover:summary:high\n"
            "context.review.required_changes=add tests\n\n"
            "Current stage task:\n\n"
            "Plan for ship"
        )

    def test_codergen_handler_includes_declared_context_reads_in_prompt(self):
        graph = parse_dot(
            """
            digraph G {
                task [
                    shape=box,
                    prompt="Plan for $goal",
                    spark.reads_context="[\\"context.request.summary\\",\\"internal.run_id\\"]"
                ]
            }
            """
        )

        backend = _StubBackend(ok=True)
        registry = build_default_registry(codergen_backend=backend)
        runner = HandlerRunner(graph, registry)
        ctx = Context(
            values={
                "graph.goal": "ship",
                "context.request.summary": "Ship docs safely",
                "internal.run_id": "run-123",
            }
        )

        outcome = runner("task", "Plan for $goal", ctx)

        assert outcome.status == OutcomeStatus.SUCCESS
        assert backend.calls[0][1] == (
            "Declared context reads:\n\n"
            "context.request.summary=Ship docs safely\n"
            "internal.run_id=run-123\n\n"
            "Current stage task:\n\n"
            "Plan for ship"
        )

    def test_codergen_handler_marks_missing_declared_context_reads(self):
        graph = parse_dot(
            """
            digraph G {
                task [
                    shape=box,
                    prompt="Plan for $goal",
                    spark.reads_context="[\\"context.request.summary\\",\\"context.review.required_changes\\"]"
                ]
            }
            """
        )

        backend = _StubBackend(ok=True)
        registry = build_default_registry(codergen_backend=backend)
        runner = HandlerRunner(graph, registry)
        ctx = Context(
            values={
                "graph.goal": "ship",
                "context.request.summary": "Ship docs safely",
            }
        )

        outcome = runner("task", "Plan for $goal", ctx)

        assert outcome.status == OutcomeStatus.SUCCESS
        assert "context.request.summary=Ship docs safely" in backend.calls[0][1]
        assert "context.review.required_changes=<missing>" in backend.calls[0][1]

    def test_codergen_handler_preserves_explicit_dotted_non_context_reads_in_prompt(self):
        graph = parse_dot(
            """
            digraph G {
                task [
                    shape=box,
                    prompt="Plan for $goal",
                    spark.reads_context="[\\"custom.live.binding\\"]"
                ]
            }
            """
        )

        backend = _StubBackend(ok=True)
        registry = build_default_registry(codergen_backend=backend)
        runner = HandlerRunner(graph, registry)
        ctx = Context(
            values={
                "graph.goal": "ship",
                "custom.live.binding": "exact-binding",
                "context.custom.live.binding": "normalized-binding",
            }
        )

        outcome = runner("task", "Plan for $goal", ctx)
        prompt = backend.calls[0][1]

        assert outcome.status == OutcomeStatus.SUCCESS
        assert "Declared context reads:" in prompt
        assert "custom.live.binding=exact-binding" in prompt
        assert "normalized-binding" not in prompt

    def test_codergen_handler_keeps_summary_carryover_but_not_sampled_context_when_declared_reads_exist(self):
        graph = parse_dot(
            """
            digraph G {
                task [
                    shape=box,
                    prompt="Plan for $goal",
                    spark.reads_context="[\\"context.request.bound_item\\"]"
                ]
            }
            """
        )

        backend = _StubBackend(ok=True)
        registry = build_default_registry(codergen_backend=backend)
        runner = HandlerRunner(graph, registry)
        source_context = Context(
            values={
                "graph.goal": "ship",
                "internal.run_id": "run-123",
                "context.request.bound_item": "ticket-42",
                "context.sampled.alpha": "stale-alpha",
                "context.sampled.beta": "stale-beta",
                "_attractor.node_outcomes": {"review": "fail"},
                "_attractor.runtime.retry.node_id": "task",
                "_attractor.runtime.retry.attempt": 1,
                "_attractor.runtime.retry.max_attempts": 2,
            }
        )
        carryover = RuntimePreambleTransform().apply(
            "summary:high",
            source_context,
            ["review"],
            include_context_items=False,
        )
        ctx = Context(
            values={
                **source_context.snapshot(),
                "_attractor.runtime.context_carryover": carryover,
            }
        )

        outcome = runner("task", "Plan for $goal", ctx)
        prompt = backend.calls[0][1]

        assert outcome.status == OutcomeStatus.SUCCESS
        assert "Context carryover:" in prompt
        assert "recent_stages=review:fail" in prompt
        assert "retry.node_id=task" in prompt
        assert "retry.attempt=1" in prompt
        assert "Declared context reads:" in prompt
        assert "context.request.bound_item=ticket-42" in prompt
        assert "context.sampled.alpha=stale-alpha" not in prompt
        assert "context.sampled.beta=stale-beta" not in prompt

    def test_codergen_handler_keeps_existing_fallback_carryover_without_declared_reads(self):
        graph = parse_dot(
            """
            digraph G {
                task [shape=box, prompt="Plan for $goal"]
            }
            """
        )

        backend = _StubBackend(ok=True)
        registry = build_default_registry(codergen_backend=backend)
        runner = HandlerRunner(graph, registry)
        source_context = Context(
            values={
                "graph.goal": "ship",
                "internal.run_id": "run-123",
                "context.sampled.alpha": "stale-alpha",
                "_attractor.node_outcomes": {"review": "fail"},
            }
        )
        carryover = RuntimePreambleTransform().apply("summary:high", source_context, ["review"])
        ctx = Context(
            values={
                **source_context.snapshot(),
                "_attractor.runtime.context_carryover": carryover,
            }
        )

        outcome = runner("task", "Plan for $goal", ctx)
        prompt = backend.calls[0][1]

        assert outcome.status == OutcomeStatus.SUCCESS
        assert "Context carryover:" in prompt
        assert "context.sampled.alpha=stale-alpha" in prompt
        assert "Declared context reads:" not in prompt

    def test_codergen_handler_fails_fast_on_malformed_reads_context_declaration(self):
        graph = parse_dot(
            """
            digraph G {
                task [shape=box, prompt="Plan for $goal", spark.reads_context="{\\"bad\\":true}"]
            }
            """
        )

        backend = _StubBackend(ok=True)
        registry = build_default_registry(codergen_backend=backend)
        runner = HandlerRunner(graph, registry)

        outcome = runner("task", "Plan for $goal", Context(values={"graph.goal": "ship"}))

        assert outcome.status == OutcomeStatus.FAIL
        assert outcome.failure_kind == FailureKind.CONTRACT
        assert outcome.failure_reason == "spark.reads_context parse error: expected a JSON array of strings"
        assert backend.calls == []

    def test_codergen_handler_expands_goal_from_graph_attr_when_context_missing(self):
        graph = parse_dot(
            """
            digraph G {
                graph [goal="Ship docs"]
                task [shape=box, prompt="Plan for $goal"]
            }
            """
        )

        backend = _StubBackend(ok=True)
        registry = build_default_registry(codergen_backend=backend)
        runner = HandlerRunner(graph, registry)

        outcome = runner("task", "Plan for $goal", Context())
        assert outcome.status == OutcomeStatus.SUCCESS
        assert backend.calls[0][1] == "Plan for Ship docs"

    def test_codergen_handler_appends_status_envelope_contract_when_configured(self):
        graph = parse_dot(
            """
            digraph G {
                task [shape=box, prompt="Plan for $goal", codergen.response_contract="status_envelope"]
            }
            """
        )

        backend = _StubBackend(ok=True)
        registry = build_default_registry(codergen_backend=backend)
        runner = HandlerRunner(graph, registry)

        outcome = runner("task", "Plan for $goal", Context(values={"graph.goal": "ship"}))

        assert outcome.status == OutcomeStatus.SUCCESS
        assert backend.calls[0][1] == f"Plan for ship\n\n{STATUS_ENVELOPE_PROMPT_APPENDIX}"
        assert backend.calls[0][3] == "status_envelope"
        assert backend.calls[0][4] == 1

    def test_codergen_handler_passes_explicit_contract_repair_budget(self):
        graph = parse_dot(
            """
            digraph G {
                task [
                    shape=box,
                    prompt="Plan for $goal",
                    codergen.response_contract="status_envelope",
                    codergen.contract_repair_attempts=2
                ]
            }
            """
        )

        backend = _StubBackend(ok=True)
        registry = build_default_registry(codergen_backend=backend)
        runner = HandlerRunner(graph, registry)

        outcome = runner("task", "Plan for $goal", Context(values={"graph.goal": "ship"}))

        assert outcome.status == OutcomeStatus.SUCCESS
        assert backend.calls[0][3] == "status_envelope"
        assert backend.calls[0][4] == 2

    def test_codergen_handler_passes_node_level_llm_model_override(self):
        graph = parse_dot(
            """
            digraph G {
                task [shape=box, prompt="Plan", llm_model="gpt-node-override"]
            }
            """
        )

        backend = _StubBackend(ok=True)
        registry = build_default_registry(codergen_backend=backend)
        runner = HandlerRunner(graph, registry)

        outcome = runner("task", "Plan", Context(values={RUNTIME_LAUNCH_MODEL_KEY: "gpt-launch-default"}))

        assert outcome.status == OutcomeStatus.SUCCESS
        assert backend.calls[0][5] == "gpt-node-override"

    def test_codergen_handler_passes_stylesheet_resolved_llm_model_override(self):
        graph = parse_dot(
            """
            digraph G {
                graph [model_stylesheet="box { llm_model: gpt-style-override; }"]
                task [shape=box, prompt="Plan"]
            }
            """
        )
        graph = TransformPipeline([ModelStylesheetTransform()]).apply(graph)

        backend = _StubBackend(ok=True)
        registry = build_default_registry(codergen_backend=backend)
        runner = HandlerRunner(graph, registry)

        outcome = runner("task", "Plan", Context(values={RUNTIME_LAUNCH_MODEL_KEY: "gpt-launch-default"}))

        assert outcome.status == OutcomeStatus.SUCCESS
        assert backend.calls[0][5] == "gpt-style-override"

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

    def test_codergen_handler_returns_simulation_response_when_backend_absent(self):
        graph = parse_dot(
            """
            digraph G {
                task [shape=box, prompt="Plan for $goal"]
            }
            """
        )

        with tempfile.TemporaryDirectory() as tmp:
            logs_root = Path(tmp) / "logs"
            registry = build_default_registry(codergen_backend=None)
            runner = HandlerRunner(graph, registry, logs_root=logs_root)

            outcome = runner("task", "Plan for $goal", Context(values={"graph.goal": "ship"}))

            assert outcome.status == OutcomeStatus.SUCCESS
            assert outcome.notes == "Stage completed: task"
            assert outcome.context_updates == {
                "last_response": "[Simulated] Response for stage: task",
                "last_stage": "task",
            }
            response_path = logs_root / "task" / "response.md"
            assert response_path.exists()
            assert response_path.read_text(encoding="utf-8").strip() == "[Simulated] Response for stage: task"

    def test_codergen_handler_supports_backend_outcome_response(self):
        graph = parse_dot(
            """
            digraph G {
                task [shape=box, prompt="Plan for $goal"]
            }
            """
        )

        backend_outcome = Outcome(
            status=OutcomeStatus.RETRY,
            notes="please retry",
            suggested_next_ids=["fallback_stage"],
            context_updates={"work.last": "task"},
        )
        registry = build_default_registry(codergen_backend=_OutcomeBackend(backend_outcome))
        runner = HandlerRunner(graph, registry)

        outcome = runner("task", "Plan for $goal", Context(values={"graph.goal": "ship"}))

        assert outcome is backend_outcome
        assert outcome.status == OutcomeStatus.RETRY
        assert outcome.notes == "please retry"
        assert outcome.suggested_next_ids == ["fallback_stage"]
        assert outcome.context_updates == {
            "last_response": "please retry",
            "last_stage": "task",
            "work.last": "task",
        }

    def test_codergen_handler_supports_backend_text_response(self):
        graph = parse_dot(
            """
            digraph G {
                task [shape=box, prompt="Plan for $goal"]
            }
            """
        )

        with tempfile.TemporaryDirectory() as tmp:
            logs_root = Path(tmp) / "logs"
            backend = _TextBackend("backend text response")
            registry = build_default_registry(codergen_backend=backend)
            runner = HandlerRunner(graph, registry, logs_root=logs_root)

            outcome = runner("task", "Plan for $goal", Context(values={"graph.goal": "ship"}))

            assert outcome.status == OutcomeStatus.SUCCESS
            assert outcome.context_updates == {
                "last_response": "backend text response",
                "last_stage": "task",
            }
            response_path = logs_root / "task" / "response.md"
            assert response_path.exists()
            assert response_path.read_text(encoding="utf-8").strip() == "backend text response"

    def test_codergen_handler_writes_raw_response_text_when_backend_outcome_provides_it(self):
        graph = parse_dot(
            """
            digraph G {
                task [shape=box, prompt="Plan for $goal"]
            }
            """
        )

        raw_response = '{"outcome":"fail","notes":"backend returned partial"}'
        backend_outcome = Outcome(
            status=OutcomeStatus.FAIL,
            notes="backend returned partial",
            raw_response_text=raw_response,
        )

        with tempfile.TemporaryDirectory() as tmp:
            logs_root = Path(tmp) / "logs"
            registry = build_default_registry(codergen_backend=_OutcomeBackend(backend_outcome))
            runner = HandlerRunner(graph, registry, logs_root=logs_root)

            outcome = runner("task", "Plan for $goal", Context(values={"graph.goal": "ship"}))

            assert outcome is backend_outcome
            assert outcome.context_updates["last_response"] == raw_response
            response_path = logs_root / "task" / "response.md"
            assert response_path.exists()
            assert response_path.read_text(encoding="utf-8").strip() == raw_response

    def test_codergen_handler_writes_prompt_before_backend_and_response_afterward(self):
        graph = parse_dot(
            """
            digraph G {
                task [shape=box, prompt="Plan for $goal"]
            }
            """
        )

        with tempfile.TemporaryDirectory() as tmp:
            logs_root = Path(tmp) / "logs"
            backend = _ArtifactProbeBackend(logs_root)
            registry = build_default_registry(codergen_backend=backend)
            runner = HandlerRunner(graph, registry, logs_root=logs_root)
            ctx = Context(values={"graph.goal": "ship"})

            outcome = runner("task", "Plan for $goal", ctx)

            assert outcome.status == OutcomeStatus.SUCCESS
            assert backend.prompt_exists_during_call is True
            assert backend.prompt_text_during_call == "Plan for ship"
            assert backend.response_exists_during_call is False
            response_path = logs_root / "task" / "response.md"
            assert response_path.exists()
            assert response_path.read_text(encoding="utf-8").strip() == "codergen backend success"

    def test_codergen_handler_writes_status_json_from_final_outcome(self):
        graph = parse_dot(
            """
            digraph G {
                task [shape=box, prompt="Plan for $goal"]
            }
            """
        )
        backend_outcome = Outcome(
            status=OutcomeStatus.FAIL,
            preferred_label="continue",
            suggested_next_ids=["followup"],
            context_updates={"work.last": "task"},
            notes="backend returned partial",
            failure_kind=FailureKind.CONTRACT,
        )

        with tempfile.TemporaryDirectory() as tmp:
            logs_root = Path(tmp) / "logs"
            registry = build_default_registry(codergen_backend=_OutcomeBackend(backend_outcome))
            runner = HandlerRunner(graph, registry, logs_root=logs_root)

            outcome = runner("task", "Plan for $goal", Context(values={"graph.goal": "ship"}))

            assert outcome is backend_outcome
            status_path = logs_root / "task" / "status.json"
            assert status_path.exists()
            assert json.loads(status_path.read_text(encoding="utf-8")) == {
                "context_updates": {
                    "last_response": "backend returned partial",
                    "last_stage": "task",
                    "work.last": "task",
                },
                "failure_kind": "contract",
                "notes": "backend returned partial",
                "outcome": "fail",
                "preferred_label": "continue",
                "suggested_next_ids": ["followup"],
            }

    def test_codergen_handler_binds_stage_raw_rpc_logging_for_supporting_backends(self):
        graph = parse_dot(
            """
            digraph G {
                task [shape=box, prompt="Plan for $goal"]
            }
            """
        )

        with tempfile.TemporaryDirectory() as tmp:
            logs_root = Path(tmp) / "logs"
            backend = _StageLoggingBackend("backend text response")
            registry = build_default_registry(codergen_backend=backend)
            runner = HandlerRunner(graph, registry, logs_root=logs_root)

            outcome = runner("task", "Plan for $goal", Context(values={"graph.goal": "ship"}))

            assert outcome.status == OutcomeStatus.SUCCESS
            assert backend.run_bound is True
            assert backend.bind_calls == [("task", logs_root)]
