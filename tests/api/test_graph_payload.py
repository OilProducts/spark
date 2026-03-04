from __future__ import annotations

from attractor.api import server
from attractor.dsl import parse_dot


def test_graph_payload_defaults_label_to_empty_string() -> None:
    graph = parse_dot(
        """
        digraph G {
            start [shape=Mdiamond]
            done [shape=Msquare]
            start -> done
        }
        """
    )

    payload = server._graph_payload(graph)

    assert payload["graph_attrs"]["label"] == ""


def test_graph_payload_keeps_explicit_graph_label() -> None:
    graph = parse_dot(
        """
        digraph G {
            graph [label="Build Flow"]
            start [shape=Mdiamond]
            done [shape=Msquare]
            start -> done
        }
        """
    )

    payload = server._graph_payload(graph)

    assert payload["graph_attrs"]["label"] == "Build Flow"


def test_graph_payload_emits_scope_metadata_and_extension_attrs_item_11_1_02() -> None:
    graph = parse_dot(
        """
        digraph ScopePayload {
            graph [label="Scope Payload", ui_extension.graph_policy="strict"]
            node [timeout=5m, custom.node_default="global"]
            edge [weight=2, custom.edge_default="global"]

            start [shape=Mdiamond]

            subgraph cluster_loop {
                graph [label="Loop A", ui_extension.scope="loop"]
                node [thread_id="loop-a"]
                edge [weight=9]
                plan [prompt="plan", custom.node_behavior="retain"]
                subgraph cluster_inner {
                    node [timeout=45s]
                    review [prompt="review"]
                    plan -> review [custom.edge_hint="inner"]
                }
            }

            done [shape=Msquare]
            start -> plan [custom.edge_hint="outer"]
            review -> done
        }
        """
    )

    payload = server._graph_payload(graph)
    nodes_by_id = {node["id"]: node for node in payload["nodes"]}

    assert payload["graph_attrs"]["ui_extension.graph_policy"] == "strict"
    assert nodes_by_id["plan"]["custom.node_behavior"] == "retain"
    assert payload["edges"][0]["custom.edge_hint"] == "inner"

    assert payload["defaults"]["node"]["timeout"] == "5m"
    assert payload["defaults"]["node"]["custom.node_default"] == "global"
    assert payload["defaults"]["edge"]["weight"] == 2
    assert payload["defaults"]["edge"]["custom.edge_default"] == "global"

    assert len(payload["subgraphs"]) == 1
    loop_scope = payload["subgraphs"][0]
    assert loop_scope["id"] == "cluster_loop"
    assert loop_scope["attrs"]["label"] == "Loop A"
    assert loop_scope["attrs"]["ui_extension.scope"] == "loop"
    assert set(loop_scope["node_ids"]) == {"plan", "review"}
    assert loop_scope["defaults"]["node"]["thread_id"] == "loop-a"
    assert loop_scope["defaults"]["edge"]["weight"] == 9
    assert len(loop_scope["subgraphs"]) == 1

    inner_scope = loop_scope["subgraphs"][0]
    assert inner_scope["id"] == "cluster_inner"
    assert inner_scope["defaults"]["node"]["timeout"] == "45s"
