import unittest

from attractor.dsl import parse_dot
from attractor.engine.context import Context
from attractor.engine.outcome import Outcome, OutcomeStatus
from attractor.engine.routing import select_next_edge


class TestRouting(unittest.TestCase):
    def _edges_from(self, graph, node_id):
        return [e for e in graph.edges if e.source == node_id]

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
        self.assertEqual("b", edge.target)

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
        self.assertEqual("b", edge_pref.target)

        out_suggested = Outcome(status=OutcomeStatus.SUCCESS, suggested_next_ids=["c"])
        edge_suggested = select_next_edge(self._edges_from(graph, "a"), out_suggested, Context())
        self.assertEqual("c", edge_suggested.target)

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
        self.assertEqual("b", edge.target)


if __name__ == "__main__":
    unittest.main()
