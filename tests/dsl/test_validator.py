from pathlib import Path

import pytest

from attractor.dsl import ValidationError, parse_dot, validate, validate_graph, validate_or_raise
from attractor.dsl.models import Diagnostic, DiagnosticSeverity
from attractor.dsl.validator import clear_registered_lint_rules, register_lint_rule


SIMPLE_LINEAR_FIXTURE = Path(__file__).resolve().parents[1] / "fixtures" / "simple_linear_workflow.dot"
HUMAN_GATE_FIXTURE = Path(__file__).resolve().parents[1] / "fixtures" / "human_gate_workflow.dot"


@pytest.fixture(autouse=True)
def _clear_registered_lint_rules():
    clear_registered_lint_rules()
    yield
    clear_registered_lint_rules()


class TestDotValidator:
    def _errors(self, diagnostics):
        return [d for d in diagnostics if d.severity == DiagnosticSeverity.ERROR]

    def test_diagnostic_exposes_spec_field_aliases(self):
        dot = """
        digraph G {
            start [shape=Mdiamond]
            done [shape=Msquare]
            orphan [shape=box]
            start -> done
        }
        """
        graph = parse_dot(dot)
        diagnostics = validate_graph(graph)
        reachability = next(d for d in diagnostics if d.rule_id == "reachability")

        assert reachability.rule == "reachability"
        assert reachability.node == "orphan"
        assert reachability.edge is None
        assert reachability.fix is None

    def test_start_exit_and_reachability_rules(self):
        dot = """
        digraph G {
            start [shape=Mdiamond]
            done [shape=Msquare]
            orphan [shape=box]
            start -> done
        }
        """
        graph = parse_dot(dot)
        diagnostics = validate_graph(graph)

        rule_ids = {d.rule_id for d in diagnostics}
        assert "reachability" in rule_ids

    def test_edge_target_exists_and_start_incoming_exit_outgoing(self):
        dot = """
        digraph G {
            start [shape=Mdiamond]
            done [shape=Msquare]
            helper [shape=box]

            helper -> start
            done -> helper
            start -> missing
        }
        """
        graph = parse_dot(dot)
        diagnostics = validate_graph(graph)
        rule_ids = [d.rule_id for d in diagnostics]

        assert "edge_target_exists" in rule_ids
        assert "start_no_incoming" in rule_ids
        assert "exit_no_outgoing" in rule_ids

    def test_edge_target_exists_reports_edge_and_fix_hint(self):
        dot = """
        digraph G {
            start [shape=Mdiamond]
            done [shape=Msquare]

            start -> done
            start -> missing
        }
        """
        graph = parse_dot(dot)
        diagnostics = validate_graph(graph)
        target_errors = [
            d for d in diagnostics if d.rule_id == "edge_target_exists" and d.severity == DiagnosticSeverity.ERROR
        ]

        assert len(target_errors) == 1
        error = target_errors[0]
        assert error.edge == ("start", "missing")
        assert error.message == "edge target 'missing' does not reference an existing node"
        assert error.fix == "define node 'missing' or update the edge target"

    def test_non_exit_nodes_must_have_outgoing_edges(self):
        dot = """
        digraph G {
            start [shape=Mdiamond]
            stuck [shape=box]
            done [shape=Msquare]

            start -> stuck
        }
        """
        graph = parse_dot(dot)
        diagnostics = validate_graph(graph)
        rule_ids = {d.rule_id for d in diagnostics}

        assert "node_has_outgoing_edge" in rule_ids

    def test_condition_and_stylesheet_syntax(self):
        dot = """
        digraph G {
            graph [model_stylesheet="box model = x }"]
            start [shape=Mdiamond]
            done [shape=Msquare]
            a [shape=box]

            start -> a [condition="unknown=1"]
            a -> done
        }
        """
        graph = parse_dot(dot)
        diagnostics = validate_graph(graph)
        rule_ids = {d.rule_id for d in diagnostics}

        assert "condition_syntax" in rule_ids
        assert "stylesheet_syntax" in rule_ids

    def test_condition_syntax_rejects_invalid_context_path(self):
        dot = """
        digraph G {
            start [shape=Mdiamond]
            task [shape=box]
            done [shape=Msquare]

            start -> task
            task -> done [condition="context..tests_passed=true"]
        }
        """
        graph = parse_dot(dot)
        diagnostics = validate_graph(graph)
        error_rules = {d.rule_id for d in self._errors(diagnostics)}

        assert "condition_syntax" in error_rules

    def test_condition_syntax_accepts_valid_context_path(self):
        dot = """
        digraph G {
            start [shape=Mdiamond]
            task [shape=box]
            done [shape=Msquare]

            start -> task
            task -> done [condition="outcome=success && context.tests_passed=true"]
        }
        """
        graph = parse_dot(dot)
        diagnostics = validate_graph(graph)
        condition_errors = [d for d in self._errors(diagnostics) if d.rule_id == "condition_syntax"]

        assert condition_errors == []

    def test_condition_syntax_rejects_context_path_with_trailing_dot(self):
        dot = """
        digraph G {
            start [shape=Mdiamond]
            task [shape=box]
            done [shape=Msquare]

            start -> task
            task -> done [condition="context.tests_passed.=true"]
        }
        """
        graph = parse_dot(dot)
        diagnostics = validate_graph(graph)
        error_rules = {d.rule_id for d in self._errors(diagnostics)}

        assert "condition_syntax" in error_rules

    def test_stylesheet_selector_and_property_restrictions(self):
        dot = """
        digraph G {
            graph [model_stylesheet="box { llm_model: x; } .BadClass { llm_provider: openai; } * { model: gpt; } #node { reasoning_effort: ultra; }"]
            start [shape=Mdiamond]
            done [shape=Msquare]
            start -> done
        }
        """
        graph = parse_dot(dot)
        diagnostics = validate_graph(graph)
        errors = [d for d in diagnostics if d.rule_id == "stylesheet_syntax" and d.severity == DiagnosticSeverity.ERROR]
        assert len(errors) >= 3

    def test_stylesheet_requires_at_least_one_declaration(self):
        dot = """
        digraph G {
            graph [model_stylesheet="* { ; }"]
            start [shape=Mdiamond]
            done [shape=Msquare]
            start -> done
        }
        """
        graph = parse_dot(dot)
        diagnostics = validate_graph(graph)
        error_rules = {d.rule_id for d in self._errors(diagnostics)}

        assert "stylesheet_syntax" in error_rules

    def test_stylesheet_attribute_rejects_empty_string(self):
        dot = """
        digraph G {
            graph [model_stylesheet="   "]
            start [shape=Mdiamond]
            done [shape=Msquare]
            start -> done
        }
        """
        graph = parse_dot(dot)
        diagnostics = validate_graph(graph)
        error_rules = {d.rule_id for d in self._errors(diagnostics)}

        assert "stylesheet_syntax" in error_rules

    def test_stylesheet_requires_semicolon_between_declarations(self):
        dot = """
        digraph G {
            graph [model_stylesheet="* { llm_model: gpt llm_provider: openai; }"]
            start [shape=Mdiamond]
            done [shape=Msquare]
            start -> done
        }
        """
        graph = parse_dot(dot)
        diagnostics = validate_graph(graph)
        error_rules = {d.rule_id for d in self._errors(diagnostics)}

        assert "stylesheet_syntax" in error_rules

    def test_stylesheet_rejects_empty_property_value(self):
        dot = """
        digraph G {
            graph [model_stylesheet="* { llm_model: ; }"]
            start [shape=Mdiamond]
            done [shape=Msquare]
            start -> done
        }
        """
        graph = parse_dot(dot)
        diagnostics = validate_graph(graph)
        error_rules = {d.rule_id for d in self._errors(diagnostics)}

        assert "stylesheet_syntax" in error_rules

    def test_stylesheet_rejects_unclosed_quoted_property_value(self):
        dot = """
        digraph G {
            graph [model_stylesheet="* { llm_provider: \\"openai; }"]
            start [shape=Mdiamond]
            done [shape=Msquare]
            start -> done
        }
        """
        graph = parse_dot(dot)
        diagnostics = validate_graph(graph)
        error_rules = {d.rule_id for d in self._errors(diagnostics)}

        assert "stylesheet_syntax" in error_rules

    def test_stylesheet_rejects_empty_declaration_between_semicolons(self):
        dot = """
        digraph G {
            graph [model_stylesheet="* { llm_model: gpt;; llm_provider: openai; }"]
            start [shape=Mdiamond]
            done [shape=Msquare]
            start -> done
        }
        """
        graph = parse_dot(dot)
        diagnostics = validate_graph(graph)
        error_rules = {d.rule_id for d in self._errors(diagnostics)}

        assert "stylesheet_syntax" in error_rules

    def test_stylesheet_allows_semicolon_inside_quoted_value(self):
        dot = """
        digraph G {
            graph [model_stylesheet="* { llm_model: \\"gpt;v2\\"; llm_provider: openai; }"]
            start [shape=Mdiamond]
            done [shape=Msquare]
            start -> done
        }
        """
        graph = parse_dot(dot)
        diagnostics = validate_graph(graph)
        stylesheet_errors = [
            d for d in self._errors(diagnostics) if d.rule_id == "stylesheet_syntax"
        ]

        assert stylesheet_errors == []

    def test_stylesheet_rejects_invalid_class_selector_characters(self):
        dot = """
        digraph G {
            graph [model_stylesheet=".bad$class { llm_model: gpt-5; }"]
            start [shape=Mdiamond]
            done [shape=Msquare]
            start -> done
        }
        """
        graph = parse_dot(dot)
        diagnostics = validate_graph(graph)
        error_rules = {d.rule_id for d in self._errors(diagnostics)}

        assert "stylesheet_syntax" in error_rules

    def test_stylesheet_rejects_equals_declaration_syntax(self):
        dot = """
        digraph G {
            graph [model_stylesheet="* { llm_model = gpt-5; }"]
            start [shape=Mdiamond]
            done [shape=Msquare]
            start -> done
        }
        """
        graph = parse_dot(dot)
        diagnostics = validate_graph(graph)
        error_rules = {d.rule_id for d in self._errors(diagnostics)}

        assert "stylesheet_syntax" in error_rules

    def test_retry_target_and_fidelity_warnings(self):
        dot = """
        digraph G {
            graph [default_fidelity="invalid"]
            start [shape=Mdiamond]
            a [shape=box, retry_target="missing", fidelity="bad"]
            done [shape=Msquare]
            start -> a [fidelity="wrong"]
            a -> done
        }
        """
        graph = parse_dot(dot)
        diagnostics = validate_graph(graph)

        warning_rules = {d.rule_id for d in diagnostics if d.severity == DiagnosticSeverity.WARNING}
        assert "retry_target_exists" in warning_rules
        assert "fidelity_valid" in warning_rules

    def test_goal_gate_without_retry_targets_emits_warning(self):
        dot = """
        digraph G {
            start [shape=Mdiamond]
            gate [shape=box, goal_gate=true]
            done [shape=Msquare]
            start -> gate
            gate -> done
        }
        """
        graph = parse_dot(dot)
        diagnostics = validate_graph(graph)

        warnings = [
            d for d in diagnostics if d.rule_id == "goal_gate_has_retry" and d.severity == DiagnosticSeverity.WARNING
        ]
        assert len(warnings) == 1
        assert warnings[0].node_id == "gate"

    def test_goal_gate_with_retry_target_does_not_emit_warning(self):
        dot = """
        digraph G {
            start [shape=Mdiamond]
            gate [shape=box, goal_gate=true, retry_target="rework"]
            rework [shape=box]
            done [shape=Msquare]
            start -> gate
            gate -> done
            rework -> gate
        }
        """
        graph = parse_dot(dot)
        diagnostics = validate_graph(graph)

        warnings = [
            d for d in diagnostics if d.rule_id == "goal_gate_has_retry" and d.severity == DiagnosticSeverity.WARNING
        ]
        assert warnings == []

    def test_unknown_node_type_emits_type_known_warning(self):
        dot = """
        digraph G {
            start [shape=Mdiamond]
            known [shape=box, type="tool"]
            custom [shape=box, type="custom.unknown"]
            done [shape=Msquare]
            start -> known
            known -> custom
            custom -> done
        }
        """
        graph = parse_dot(dot)
        diagnostics = validate_graph(graph)

        type_warnings = [d for d in diagnostics if d.rule_id == "type_known" and d.severity == DiagnosticSeverity.WARNING]
        assert len(type_warnings) == 1
        assert type_warnings[0].node_id == "custom"
        assert "custom.unknown" in type_warnings[0].message

    def test_codergen_node_without_prompt_or_label_emits_warning(self):
        dot = """
        digraph G {
            start [shape=Mdiamond]
            task [shape=box]
            done [shape=Msquare]
            start -> task
            task -> done
        }
        """
        graph = parse_dot(dot)
        diagnostics = validate_graph(graph)

        prompt_warnings = [
            d for d in diagnostics if d.rule_id == "prompt_on_llm_nodes" and d.severity == DiagnosticSeverity.WARNING
        ]
        assert len(prompt_warnings) == 1
        assert prompt_warnings[0].node_id == "task"

    def test_prompt_on_llm_nodes_uses_resolved_codergen_type(self):
        dot = """
        digraph G {
            start [shape=Mdiamond]
            gate [shape=hexagon]
            custom [shape=box, type="custom.unknown"]
            done [shape=Msquare]
            start -> gate
            gate -> custom
            custom -> done
        }
        """
        graph = parse_dot(dot)
        diagnostics = validate_graph(graph)

        prompt_warnings = [
            d for d in diagnostics if d.rule_id == "prompt_on_llm_nodes" and d.severity == DiagnosticSeverity.WARNING
        ]
        assert [d.node_id for d in prompt_warnings] == ["custom"]

    def test_valid_minimal_graph(self):
        dot = """
        digraph G {
            start [shape=Mdiamond]
            task [shape=box, prompt="do"]
            done [shape=Msquare]
            start -> task
            task -> done [condition="outcome=success"]
        }
        """
        graph = parse_dot(dot)
        diagnostics = validate_graph(graph)
        assert self._errors(diagnostics) == []

    def test_simple_linear_workflow_fixture_is_validator_clean(self):
        graph = parse_dot(SIMPLE_LINEAR_FIXTURE.read_text(encoding="utf-8"))
        diagnostics = validate_graph(graph)

        assert self._errors(diagnostics) == []

    def test_human_gate_workflow_fixture_is_validator_clean(self):
        graph = parse_dot(HUMAN_GATE_FIXTURE.read_text(encoding="utf-8"))
        diagnostics = validate_graph(graph)

        assert self._errors(diagnostics) == []

    def test_shape_start_and_start_id_are_both_counted_for_cardinality(self):
        dot = """
        digraph G {
            entry [shape=Mdiamond]
            start [shape=box]
            done [shape=Msquare]

            entry -> done
            start -> done
        }
        """
        graph = parse_dot(dot)
        diagnostics = validate_graph(graph)
        error_rules = {d.rule_id for d in self._errors(diagnostics)}

        assert "start_node" in error_rules

    def test_start_no_incoming_applies_even_with_multiple_start_nodes(self):
        dot = """
        digraph G {
            entry [shape=Mdiamond]
            start [shape=box]
            helper [shape=box]
            done [shape=Msquare]

            helper -> entry
            helper -> start
            entry -> done
            start -> done
        }
        """
        graph = parse_dot(dot)
        diagnostics = validate_graph(graph)

        start_no_incoming_errors = [d for d in self._errors(diagnostics) if d.rule_id == "start_no_incoming"]
        assert {d.node_id for d in start_no_incoming_errors} == {"entry", "start"}

    def test_shape_exit_takes_precedence_over_end_id_fallback(self):
        dot = """
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
        graph = parse_dot(dot)
        diagnostics = validate_graph(graph)
        error_rules = {d.rule_id for d in self._errors(diagnostics)}

        assert "terminal_node" not in error_rules
        assert "exit_no_outgoing" not in error_rules

    def test_terminal_rule_allows_multiple_terminal_nodes(self):
        dot = """
        digraph G {
            start [shape=Mdiamond]
            review [shape=box]
            done [shape=Msquare]
            archived [shape=Msquare]

            start -> review
            review -> done
            review -> archived
        }
        """
        graph = parse_dot(dot)
        diagnostics = validate_graph(graph)
        error_rules = {d.rule_id for d in self._errors(diagnostics)}

        assert "terminal_node" not in error_rules

    def test_terminal_rule_errors_when_no_terminal_nodes_exist(self):
        dot = """
        digraph G {
            start [shape=Mdiamond]
            review [shape=box]
            draft [shape=box]

            start -> review
            review -> draft
            draft -> review
        }
        """
        graph = parse_dot(dot)
        diagnostics = validate_graph(graph)
        error_rules = {d.rule_id for d in self._errors(diagnostics)}

        assert "terminal_node" in error_rules

    def test_terminal_rule_accepts_end_id_without_msquare(self):
        dot = """
        digraph G {
            start [shape=Mdiamond]
            review [shape=box]
            end [shape=box]

            start -> review
            review -> end
        }
        """
        graph = parse_dot(dot)
        diagnostics = validate_graph(graph)
        error_rules = {d.rule_id for d in self._errors(diagnostics)}

        assert "terminal_node" not in error_rules

    def test_validate_composes_builtins_with_extra_rules(self):
        dot = """
        digraph G {
            start [shape=Mdiamond]
        }
        """
        graph = parse_dot(dot)
        baseline = validate_graph(graph)

        class _ExtraRule:
            def apply(self, current_graph):
                assert current_graph is graph
                return [
                    Diagnostic(
                        rule_id="custom_rule",
                        severity=DiagnosticSeverity.INFO,
                        message="custom lint",
                        line=1,
                    )
                ]

        diagnostics = validate(graph, extra_rules=[_ExtraRule()])

        assert diagnostics[: len(baseline)] == baseline
        assert diagnostics[len(baseline) :][-1].rule_id == "custom_rule"

    def test_validate_runs_registered_lint_rules(self):
        dot = """
        digraph G {
            start [shape=Mdiamond]
            done [shape=Msquare]

            start -> done
        }
        """
        graph = parse_dot(dot)

        class _RegisteredRule:
            def apply(self, current_graph):
                assert current_graph is graph
                return [
                    Diagnostic(
                        rule_id="registered_custom_rule",
                        severity=DiagnosticSeverity.INFO,
                        message="registered lint",
                        line=1,
                    )
                ]

        register_lint_rule(_RegisteredRule())
        diagnostics = validate(graph)

        assert diagnostics[-1].rule_id == "registered_custom_rule"

    def test_validate_snapshots_registered_rules_before_execution(self):
        dot = """
        digraph G {
            start [shape=Mdiamond]
            done [shape=Msquare]

            start -> done
        }
        """
        graph = parse_dot(dot)

        class _LateRegisteredRule:
            def apply(self, current_graph):
                assert current_graph is graph
                return [
                    Diagnostic(
                        rule_id="late_registered_rule",
                        severity=DiagnosticSeverity.INFO,
                        message="late registration should not run in current pass",
                        line=1,
                    )
                ]

        class _RegisteringRule:
            def apply(self, current_graph):
                assert current_graph is graph
                register_lint_rule(_LateRegisteredRule())
                return [
                    Diagnostic(
                        rule_id="registering_rule",
                        severity=DiagnosticSeverity.INFO,
                        message="register another lint rule during validation",
                        line=1,
                    )
                ]

        register_lint_rule(_RegisteringRule())

        first_pass = validate(graph)
        first_pass_rule_ids = [d.rule_id for d in first_pass]

        assert "registering_rule" in first_pass_rule_ids
        assert "late_registered_rule" not in first_pass_rule_ids

        second_pass = validate(graph)
        second_pass_rule_ids = [d.rule_id for d in second_pass]

        assert "late_registered_rule" in second_pass_rule_ids

    def test_validate_or_raise_returns_diagnostics_when_only_warnings(self):
        dot = """
        digraph G {
            start [shape=Mdiamond]
            custom [type="custom.handler"]
            done [shape=Msquare]

            start -> custom
            custom -> done
        }
        """
        graph = parse_dot(dot)

        diagnostics = validate_or_raise(graph)

        assert diagnostics
        assert all(d.severity != DiagnosticSeverity.ERROR for d in diagnostics)
        assert any(d.rule_id == "type_known" for d in diagnostics)

    def test_validate_or_raise_raises_with_aggregated_error_messages(self):
        dot = """
        digraph G {
            start [shape=Mdiamond]
        }
        """
        graph = parse_dot(dot)

        try:
            validate_or_raise(graph)
            assert False, "validate_or_raise should raise when validation has errors"
        except ValidationError as exc:
            assert len(exc.errors) >= 2
            assert all(d.severity == DiagnosticSeverity.ERROR for d in exc.errors)
            assert "terminal_node" in str(exc)
            assert "node_has_outgoing_edge" in str(exc)
