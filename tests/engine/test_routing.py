from attractor.dsl import parse_dot
from attractor.engine.context import Context
from attractor.engine.outcome import Outcome, OutcomeStatus
from attractor.engine.routing import select_next_edge


class TestRouting:
    def _edges_from(self, graph, node_id):
        return [e for e in graph.edges if e.source == node_id]

    def _assert_stable_target(self, edges, outcome, expected_target):
        for _ in range(10):
            edge = select_next_edge(edges, outcome, Context())
            assert edge is not None
            assert edge.target == expected_target

    def test_priority_condition_over_weight(self):
        graph = parse_dot(
            """
            digraph G {
                start [shape=Mdiamond]
                a [shape=box]
                b [shape=box]
                c [shape=box]
                done [shape=Msquare]

                start -> a
                a -> b [condition="outcome=success", weight=0]
                a -> c [weight=10]
                b -> done
                c -> done
            }
            """
        )
        outcome = Outcome(status=OutcomeStatus.SUCCESS)
        edge = select_next_edge(self._edges_from(graph, "a"), outcome, Context())
        assert edge.target == "b"

    def test_preferred_label_then_suggested_ids(self):
        graph = parse_dot(
            """
            digraph G {
                start [shape=Mdiamond]
                a [shape=box]
                b [shape=box]
                c [shape=box]
                done [shape=Msquare]

                start -> a
                a -> b [label="[Y] Approve"]
                a -> c [label="Fix"]
                b -> done
                c -> done
            }
            """
        )
        out_pref = Outcome(status=OutcomeStatus.SUCCESS, preferred_label="Approve")
        edge_pref = select_next_edge(self._edges_from(graph, "a"), out_pref, Context())
        assert edge_pref.target == "b"

        out_suggested = Outcome(status=OutcomeStatus.SUCCESS, suggested_next_ids=["c"])
        edge_suggested = select_next_edge(self._edges_from(graph, "a"), out_suggested, Context())
        assert edge_suggested.target == "c"

    def test_suggested_next_ids_uses_list_priority_with_fallback_on_missing_ids(self):
        graph = parse_dot(
            """
            digraph G {
                start [shape=Mdiamond]
                a [shape=box]
                b [shape=box]
                c [shape=box]
                done [shape=Msquare]

                start -> a
                a -> b
                a -> c
                b -> done
                c -> done
            }
            """
        )
        outcome = Outcome(status=OutcomeStatus.SUCCESS, suggested_next_ids=["missing", "c", "b"])
        edge = select_next_edge(self._edges_from(graph, "a"), outcome, Context())
        assert edge.target == "c"

    def test_weight_then_lexical_tiebreak_for_unconditional(self):
        graph = parse_dot(
            """
            digraph G {
                start [shape=Mdiamond]
                a [shape=box]
                b [shape=box]
                c [shape=box]
                done [shape=Msquare]

                start -> a
                a -> c [weight=5]
                a -> b [weight=5]
                b -> done
                c -> done
            }
            """
        )
        outcome = Outcome(status=OutcomeStatus.SUCCESS)
        edge = select_next_edge(self._edges_from(graph, "a"), outcome, Context())
        assert edge.target == "b"

    def test_lexical_tiebreak_is_based_on_target_id_not_source(self):
        graph = parse_dot(
            """
            digraph G {
                a [shape=box]
                z [shape=box]
                done [shape=Msquare]

                a -> done [weight=5]
                z -> done [weight=5]
            }
            """
        )
        # Intentionally reverse insertion order to ensure source does not affect tie-breaking.
        z_edge = next(e for e in graph.edges if e.source == "z")
        a_edge = next(e for e in graph.edges if e.source == "a")
        outcome = Outcome(status=OutcomeStatus.SUCCESS)
        edge = select_next_edge([z_edge, a_edge], outcome, Context())
        assert edge.source == "z"

    def test_condition_candidate_evaluation_ignores_whitespace_only_conditions(self):
        graph = parse_dot(
            """
            digraph G {
                start [shape=Mdiamond]
                a [shape=box]
                b [shape=box]
                c [shape=box]
                done [shape=Msquare]

                start -> a
                a -> b [condition="   ", weight=99]
                a -> c [condition="outcome=success", weight=1]
                b -> done
                c -> done
            }
            """
        )
        outcome = Outcome(status=OutcomeStatus.SUCCESS)
        edge = select_next_edge(self._edges_from(graph, "a"), outcome, Context())
        assert edge.target == "c"

    def test_deterministic_step_1_condition_match_wins_over_other_signals(self):
        graph = parse_dot(
            """
            digraph G {
                start [shape=Mdiamond]
                a [shape=box]
                c1 [shape=box]
                c2 [shape=box]
                label_hit [shape=box]
                suggested_hit [shape=box]
                done [shape=Msquare]

                start -> a
                a -> c2 [condition="outcome=success", weight=4]
                a -> c1 [condition="outcome=success", weight=4]
                a -> label_hit [label="[Y] Approve", weight=99]
                a -> suggested_hit [weight=50]
                c1 -> done
                c2 -> done
                label_hit -> done
                suggested_hit -> done
            }
            """
        )
        outcome = Outcome(
            status=OutcomeStatus.SUCCESS,
            preferred_label="Approve",
            suggested_next_ids=["suggested_hit"],
        )
        self._assert_stable_target(self._edges_from(graph, "a"), outcome, "c1")

    def test_deterministic_step_2_preferred_label_match(self):
        graph = parse_dot(
            """
            digraph G {
                start [shape=Mdiamond]
                a [shape=box]
                label_hit [shape=box]
                suggested_hit [shape=box]
                weight_hit [shape=box]
                done [shape=Msquare]

                start -> a
                a -> label_hit [label="[Y] Approve"]
                a -> suggested_hit
                a -> weight_hit [weight=25]
                label_hit -> done
                suggested_hit -> done
                weight_hit -> done
            }
            """
        )
        outcome = Outcome(
            status=OutcomeStatus.FAIL,
            preferred_label="approve",
            suggested_next_ids=["suggested_hit"],
        )
        self._assert_stable_target(self._edges_from(graph, "a"), outcome, "label_hit")

    def test_deterministic_step_3_suggested_next_ids(self):
        graph = parse_dot(
            """
            digraph G {
                start [shape=Mdiamond]
                a [shape=box]
                b [shape=box]
                c [shape=box]
                done [shape=Msquare]

                start -> a
                a -> b
                a -> c
                b -> done
                c -> done
            }
            """
        )
        outcome = Outcome(status=OutcomeStatus.FAIL, suggested_next_ids=["missing", "c", "b"])
        self._assert_stable_target(self._edges_from(graph, "a"), outcome, "c")

    def test_deterministic_step_4_highest_weight_unconditional(self):
        graph = parse_dot(
            """
            digraph G {
                start [shape=Mdiamond]
                a [shape=box]
                low [shape=box]
                high [shape=box]
                done [shape=Msquare]

                start -> a
                a -> low [weight=1]
                a -> high [weight=9]
                low -> done
                high -> done
            }
            """
        )
        outcome = Outcome(status=OutcomeStatus.FAIL)
        self._assert_stable_target(self._edges_from(graph, "a"), outcome, "high")

    def test_deterministic_step_5_lexical_tiebreak(self):
        graph = parse_dot(
            """
            digraph G {
                start [shape=Mdiamond]
                a [shape=box]
                beta [shape=box]
                alpha [shape=box]
                done [shape=Msquare]

                start -> a
                a -> beta [weight=7]
                a -> alpha [weight=7]
                beta -> done
                alpha -> done
            }
            """
        )
        outcome = Outcome(status=OutcomeStatus.FAIL)
        self._assert_stable_target(self._edges_from(graph, "a"), outcome, "alpha")

    def test_deterministic_fallback_selects_best_edge_when_no_conditions_match(self):
        graph = parse_dot(
            """
            digraph G {
                start [shape=Mdiamond]
                a [shape=box]
                high [shape=box]
                low [shape=box]
                done [shape=Msquare]

                start -> a
                a -> low [condition="outcome=partial_success", weight=1]
                a -> high [condition="outcome=partial_success", weight=5]
                high -> done
                low -> done
            }
            """
        )
        outcome = Outcome(status=OutcomeStatus.SUCCESS)
        self._assert_stable_target(self._edges_from(graph, "a"), outcome, "high")

    def test_skipped_status_uses_non_fail_fallback_routing(self):
        graph = parse_dot(
            """
            digraph G {
                start [shape=Mdiamond]
                a [shape=box]
                primary [shape=box]
                secondary [shape=box]
                done [shape=Msquare]

                start -> a
                a -> secondary [condition="outcome=success", weight=1]
                a -> primary [condition="outcome=partial_success", weight=3]
                primary -> done
                secondary -> done
            }
            """
        )
        outcome = Outcome(status=OutcomeStatus.SKIPPED)
        edge = select_next_edge(self._edges_from(graph, "a"), outcome, Context())
        assert edge is not None
        assert edge.target == "primary"
