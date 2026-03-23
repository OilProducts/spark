from __future__ import annotations

from pathlib import Path

from fastapi.testclient import TestClient

import attractor.api.server as server
from workspace.flow_catalog import (
    LAUNCH_POLICY_AGENT_REQUESTABLE,
    LAUNCH_POLICY_DISABLED,
    read_flow_launch_policy,
    set_flow_launch_policy,
)


def _write_flow(name: str, content: str) -> Path:
    flow_path = server.get_settings().flows_dir / name
    flow_path.parent.mkdir(parents=True, exist_ok=True)
    flow_path.write_text(content, encoding="utf-8")
    return flow_path


def test_flow_catalog_round_trip_defaults_uncataloged_to_disabled() -> None:
    config_dir = server.get_settings().config_dir

    uncataloged = read_flow_launch_policy(config_dir, "uncataloged.dot")
    assert uncataloged.launch_policy is None
    assert uncataloged.effective_launch_policy == LAUNCH_POLICY_DISABLED

    saved = set_flow_launch_policy(config_dir, "agent-visible.dot", LAUNCH_POLICY_AGENT_REQUESTABLE)
    assert saved.launch_policy == LAUNCH_POLICY_AGENT_REQUESTABLE
    assert saved.effective_launch_policy == LAUNCH_POLICY_AGENT_REQUESTABLE

    reloaded = read_flow_launch_policy(config_dir, "agent-visible.dot")
    assert reloaded.launch_policy == LAUNCH_POLICY_AGENT_REQUESTABLE
    assert reloaded.effective_launch_policy == LAUNCH_POLICY_AGENT_REQUESTABLE

    catalog_path = server.get_settings().config_dir / "flow-catalog.toml"
    assert catalog_path.read_text(encoding="utf-8") == (
        '[flows."agent-visible.dot"]\n'
        'launch_policy = "agent_requestable"\n'
    )


def test_list_workspace_flows_human_surface_returns_all_flows_with_metadata_fallbacks(
    product_api_client: TestClient,
) -> None:
    _write_flow(
        "rich.dot",
        """
digraph rich {
  graph [label="Graph Label", goal="Graph goal", spark.title="Workspace Title", spark.description="Workspace description"];
  start [shape=Mdiamond];
  done [shape=Msquare];
  start -> done;
}
""".strip()
        + "\n",
    )
    _write_flow(
        "fallback.dot",
        """
digraph fallback {
  graph [label="Fallback Label", goal="Fallback goal"];
  start [shape=Mdiamond];
  done [shape=Msquare];
  start -> done;
}
""".strip()
        + "\n",
    )

    response = product_api_client.get("/workspace/api/flows", params={"surface": "human"})

    assert response.status_code == 200
    payload = response.json()
    assert payload == [
        {
            "name": "fallback.dot",
            "title": "Fallback Label",
            "description": "Fallback goal",
            "launch_policy": None,
            "effective_launch_policy": "disabled",
            "graph_label": "Fallback Label",
            "graph_goal": "Fallback goal",
        },
        {
            "name": "rich.dot",
            "title": "Workspace Title",
            "description": "Workspace description",
            "launch_policy": None,
            "effective_launch_policy": "disabled",
            "graph_label": "Graph Label",
            "graph_goal": "Graph goal",
        },
    ]


def test_list_workspace_flows_preserves_nested_relative_paths(
    product_api_client: TestClient,
) -> None:
    _write_flow("alpha.dot", "digraph alpha { start -> done; }\n")
    _write_flow(
        "ops/review/nested.dot",
        """
digraph nested {
  graph [spark.title="Nested Flow", spark.description="Nested flow description"];
  start [shape=Mdiamond];
  done [shape=Msquare];
  start -> done;
}
""".strip()
        + "\n",
    )

    response = product_api_client.get("/workspace/api/flows", params={"surface": "human"})

    assert response.status_code == 200
    assert response.json() == [
        {
            "name": "alpha.dot",
            "title": "alpha",
            "description": "",
            "launch_policy": None,
            "effective_launch_policy": "disabled",
            "graph_label": "",
            "graph_goal": "",
        },
        {
            "name": "ops/review/nested.dot",
            "title": "Nested Flow",
            "description": "Nested flow description",
            "launch_policy": None,
            "effective_launch_policy": "disabled",
            "graph_label": "",
            "graph_goal": "",
        },
    ]


def test_list_workspace_flows_agent_surface_filters_non_requestable_flows(
    product_api_client: TestClient,
) -> None:
    _write_flow("requestable.dot", "digraph requestable { start -> done; }\n")
    _write_flow("trigger-only.dot", "digraph trigger_only { start -> done; }\n")
    _write_flow("disabled.dot", "digraph disabled { start -> done; }\n")
    set_flow_launch_policy(server.get_settings().config_dir, "requestable.dot", "agent_requestable")
    set_flow_launch_policy(server.get_settings().config_dir, "trigger-only.dot", "trigger_only")
    set_flow_launch_policy(server.get_settings().config_dir, "disabled.dot", "disabled")

    response = product_api_client.get("/workspace/api/flows", params={"surface": "agent"})

    assert response.status_code == 200
    assert response.json() == [
        {
            "name": "requestable.dot",
            "title": "requestable",
            "description": "",
            "launch_policy": "agent_requestable",
            "effective_launch_policy": "agent_requestable",
            "graph_label": "",
            "graph_goal": "",
        }
    ]


def test_workspace_flow_describe_returns_derived_graph_features(
    product_api_client: TestClient,
) -> None:
    _write_flow(
        "inspectable.dot",
        """
digraph inspectable {
  graph [label="Inspectable Graph", goal="Inspect graph behavior"];
  start [shape=Mdiamond];
  human_review [shape=hexagon];
  manager [shape=house];
  done [shape=Msquare];
  start -> human_review;
  human_review -> manager;
  manager -> done;
}
""".strip()
        + "\n",
    )
    set_flow_launch_policy(server.get_settings().config_dir, "inspectable.dot", "agent_requestable")

    response = product_api_client.get("/workspace/api/flows/inspectable.dot", params={"surface": "agent"})

    assert response.status_code == 200
    assert response.json() == {
        "name": "inspectable.dot",
        "title": "Inspectable Graph",
        "description": "Inspect graph behavior",
        "launch_policy": "agent_requestable",
        "effective_launch_policy": "agent_requestable",
        "graph_label": "Inspectable Graph",
        "graph_goal": "Inspect graph behavior",
        "node_count": 4,
        "edge_count": 3,
        "features": {
            "has_human_gate": True,
            "has_manager_loop": True,
        },
    }


def test_workspace_flow_endpoints_support_nested_relative_paths(
    product_api_client: TestClient,
) -> None:
    flow_content = """
digraph nested {
  graph [label="Nested Inspectable", goal="Inspect nested graph behavior"];
  start [shape=Mdiamond];
  human_review [shape=hexagon];
  done [shape=Msquare];
  start -> human_review;
  human_review -> done;
}
""".strip() + "\n"
    _write_flow("ops/review/inspectable.dot", flow_content)
    set_flow_launch_policy(server.get_settings().config_dir, "ops/review/inspectable.dot", "agent_requestable")

    describe_response = product_api_client.get("/workspace/api/flows/ops/review/inspectable.dot", params={"surface": "agent"})
    raw_response = product_api_client.get("/workspace/api/flows/ops/review/inspectable.dot/raw", params={"surface": "agent"})
    validate_response = product_api_client.get("/workspace/api/flows/ops/review/inspectable.dot/validate")
    policy_response = product_api_client.put(
        "/workspace/api/flows/ops/review/inspectable.dot/launch-policy",
        json={"launch_policy": "trigger_only"},
    )

    assert describe_response.status_code == 200
    assert describe_response.json()["name"] == "ops/review/inspectable.dot"
    assert raw_response.status_code == 200
    assert raw_response.headers["x-spark-flow-name"] == "ops/review/inspectable.dot"
    assert raw_response.text == flow_content
    assert validate_response.status_code == 200
    assert validate_response.json()["name"] == "ops/review/inspectable.dot"
    assert validate_response.json()["path"].endswith("/ops/review/inspectable.dot")
    assert policy_response.status_code == 200
    assert policy_response.json()["name"] == "ops/review/inspectable.dot"
    assert read_flow_launch_policy(server.get_settings().config_dir, "ops/review/inspectable.dot").launch_policy == "trigger_only"


def test_workspace_flow_agent_surface_hides_non_requestable_describe_and_raw(
    product_api_client: TestClient,
) -> None:
    _write_flow("trigger-only.dot", "digraph trigger_only { start -> done; }\n")
    set_flow_launch_policy(server.get_settings().config_dir, "trigger-only.dot", "trigger_only")

    describe_response = product_api_client.get("/workspace/api/flows/trigger-only.dot", params={"surface": "agent"})
    raw_response = product_api_client.get("/workspace/api/flows/trigger-only.dot/raw", params={"surface": "agent"})

    assert describe_response.status_code == 404
    assert raw_response.status_code == 404


def test_workspace_flow_raw_returns_dot_for_requestable_flow(
    product_api_client: TestClient,
) -> None:
    flow_content = 'digraph requestable { graph [label="Requestable"]; start -> done; }\n'
    _write_flow("requestable.dot", flow_content)
    set_flow_launch_policy(server.get_settings().config_dir, "requestable.dot", "agent_requestable")

    response = product_api_client.get("/workspace/api/flows/requestable.dot/raw", params={"surface": "agent"})

    assert response.status_code == 200
    assert response.text == flow_content
    assert response.headers["content-type"].startswith("text/vnd.graphviz")


def test_workspace_flow_validate_returns_preview_payload_for_existing_flow(
    product_api_client: TestClient,
) -> None:
    _write_flow(
        "draft.dot",
        """
digraph draft {
  graph [label="Draft Flow", goal="Draft the thing"];
  start [shape=Mdiamond];
  draft_email [shape=box, prompt="Draft the thing."];
  done [shape=Msquare];
  start -> draft_email;
  draft_email -> done;
}
""".strip()
        + "\n",
    )

    response = product_api_client.get("/workspace/api/flows/draft.dot/validate")

    assert response.status_code == 200
    payload = response.json()
    assert payload["name"] == "draft.dot"
    assert payload["path"].endswith("/draft.dot")
    assert payload["status"] == "ok"
    assert payload["diagnostics"] == []
    assert payload["errors"] == []


def test_workspace_flow_validate_surfaces_parse_errors(
    product_api_client: TestClient,
) -> None:
    _write_flow("broken.dot", "digraph broken { start -> \n")

    response = product_api_client.get("/workspace/api/flows/broken.dot/validate")

    assert response.status_code == 200
    payload = response.json()
    assert payload["name"] == "broken.dot"
    assert payload["status"] == "parse_error"
    assert len(payload["diagnostics"]) == 1
    assert payload["diagnostics"][0]["rule_id"] == "parse_error"


def test_workspace_flow_launch_policy_update_persists_catalog_entry(
    product_api_client: TestClient,
) -> None:
    _write_flow("editable.dot", "digraph editable { start -> done; }\n")

    response = product_api_client.put(
        "/workspace/api/flows/editable.dot/launch-policy",
        json={"launch_policy": "trigger_only"},
    )

    assert response.status_code == 200
    assert response.json() == {
        "name": "editable.dot",
        "launch_policy": "trigger_only",
        "effective_launch_policy": "trigger_only",
        "allowed_launch_policies": [
            "agent_requestable",
            "disabled",
            "trigger_only",
        ],
    }
    catalog_state = read_flow_launch_policy(server.get_settings().config_dir, "editable.dot")
    assert catalog_state.launch_policy == "trigger_only"
