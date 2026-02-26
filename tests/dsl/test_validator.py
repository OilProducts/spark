from pathlib import Path

from attractor.dsl import parse_dot, validate_graph
from attractor.dsl.models import DiagnosticSeverity


SIMPLE_LINEAR_FIXTURE = Path(__file__).resolve().parents[1] / "fixtures" / "simple_linear_workflow.dot"
HUMAN_GATE_FIXTURE = Path(__file__).resolve().parents[1] / "fixtures" / "human_gate_workflow.dot"


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

    def test_shape_start_takes_precedence_over_start_id_fallback(self):
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

        assert "start_node" not in error_rules

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
