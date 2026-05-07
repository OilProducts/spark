from __future__ import annotations

import asyncio
from pathlib import Path

import pytest
from fastapi.testclient import TestClient

import attractor.api.server as server
import spark.app as product_app
import spark.starter_assets as starter_assets
from spark.workspace.attractor_client import AttractorApiClient
from spark.workspace.flow_catalog import (
    EXECUTION_LOCK_CONFLICT_POLICY_QUEUE,
    EXECUTION_LOCK_SCOPE_PROJECT,
    FlowExecutionLockConfig,
    LAUNCH_POLICY_AGENT_REQUESTABLE,
    LAUNCH_POLICY_DISABLED,
    read_flow_launch_policy,
    set_flow_catalog_entry,
    seed_default_flow_catalog,
    set_flow_launch_policy,
)


def _write_flow(name: str, content: str) -> Path:
    flow_path = product_app.get_settings().flows_dir / name
    flow_path.parent.mkdir(parents=True, exist_ok=True)
    flow_path.write_text(content, encoding="utf-8")
    return flow_path


def test_flow_catalog_round_trip_defaults_uncataloged_to_disabled() -> None:
    config_dir = product_app.get_settings().config_dir

    uncataloged = read_flow_launch_policy(config_dir, "uncataloged.dot")
    assert uncataloged.launch_policy is None
    assert uncataloged.effective_launch_policy == LAUNCH_POLICY_DISABLED

    saved = set_flow_launch_policy(config_dir, "agent-visible.dot", LAUNCH_POLICY_AGENT_REQUESTABLE)
    assert saved.launch_policy == LAUNCH_POLICY_AGENT_REQUESTABLE
    assert saved.effective_launch_policy == LAUNCH_POLICY_AGENT_REQUESTABLE

    reloaded = read_flow_launch_policy(config_dir, "agent-visible.dot")
    assert reloaded.launch_policy == LAUNCH_POLICY_AGENT_REQUESTABLE
    assert reloaded.effective_launch_policy == LAUNCH_POLICY_AGENT_REQUESTABLE
    assert reloaded.execution_lock is None

    catalog_path = product_app.get_settings().config_dir / "flow-catalog.toml"
    assert catalog_path.read_text(encoding="utf-8") == (
        '[flows."agent-visible.dot"]\n'
        'launch_policy = "agent_requestable"\n'
    )


def test_attractor_workspace_start_payload_omits_legacy_backend_and_provider_defaults() -> None:
    captured: dict[str, object] = {}

    class RecordingAttractorApiClient(AttractorApiClient):
        async def _request_json(self, method: str, path: str, **kwargs):
            captured["method"] = method
            captured["path"] = path
            captured["json"] = kwargs["json"]
            return {"status": "started", "run_id": "run-123"}

    client = RecordingAttractorApiClient(base_url="http://attractor.test")

    result = asyncio.run(
        client.start_pipeline(
            run_id=None,
            flow_name="requestable.dot",
            working_directory="/tmp/project",
            model="gpt-5.4",
            goal="Implement the approved scope.",
            launch_context={"context.request.summary": "Implement the approved scope."},
        )
    )

    assert result == {"status": "started", "run_id": "run-123"}
    assert captured["method"] == "POST"
    assert captured["path"] == "/pipelines"
    payload = captured["json"]
    assert isinstance(payload, dict)
    assert "backend" not in payload
    assert "llm_provider" not in payload
    assert "reasoning_effort" not in payload


def test_attractor_workspace_start_payload_includes_provider_and_reasoning_when_set() -> None:
    captured: dict[str, object] = {}

    class RecordingAttractorApiClient(AttractorApiClient):
        async def _request_json(self, method: str, path: str, **kwargs):
            captured["method"] = method
            captured["path"] = path
            captured["json"] = kwargs["json"]
            return {"status": "started", "run_id": "run-456"}

    client = RecordingAttractorApiClient(base_url="http://attractor.test")

    result = asyncio.run(
        client.start_pipeline(
            run_id=None,
            flow_name="requestable.dot",
            working_directory="/tmp/project",
            model="gpt-5.4",
            llm_provider="openai",
            reasoning_effort="high",
        )
    )

    assert result == {"status": "started", "run_id": "run-456"}
    payload = captured["json"]
    assert isinstance(payload, dict)
    assert payload["llm_provider"] == "openai"
    assert payload["reasoning_effort"] == "high"


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
            "execution_lock": None,
            "graph_label": "Fallback Label",
            "graph_goal": "Fallback goal",
        },
        {
            "name": "rich.dot",
            "title": "Workspace Title",
            "description": "Workspace description",
            "launch_policy": None,
            "effective_launch_policy": "disabled",
            "execution_lock": None,
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
            "execution_lock": None,
            "graph_label": "",
            "graph_goal": "",
        },
        {
            "name": "ops/review/nested.dot",
            "title": "Nested Flow",
            "description": "Nested flow description",
            "launch_policy": None,
            "effective_launch_policy": "disabled",
            "execution_lock": None,
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
    set_flow_launch_policy(product_app.get_settings().config_dir, "requestable.dot", "agent_requestable")
    set_flow_launch_policy(product_app.get_settings().config_dir, "trigger-only.dot", "trigger_only")
    set_flow_launch_policy(product_app.get_settings().config_dir, "disabled.dot", "disabled")

    response = product_api_client.get("/workspace/api/flows", params={"surface": "agent"})

    assert response.status_code == 200
    assert response.json() == [
        {
            "name": "requestable.dot",
            "title": "requestable",
            "description": "",
            "launch_policy": "agent_requestable",
            "effective_launch_policy": "agent_requestable",
            "execution_lock": None,
            "graph_label": "",
            "graph_goal": "",
        }
    ]


def test_list_workspace_flows_agent_surface_includes_seeded_core_flows(
    product_api_client: TestClient,
) -> None:
    settings = product_app.get_settings()
    starter_assets.seed_starter_flows(settings.flows_dir)
    seed_default_flow_catalog(settings.config_dir)

    response = product_api_client.get("/workspace/api/flows", params={"surface": "agent"})

    assert response.status_code == 200
    flow_names = {flow["name"] for flow in response.json()}
    assert flow_names == {
        "software-development/implement-change-request.dot",
        "software-development/spec-implementation/implement-spec.dot",
    }


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
    set_flow_launch_policy(product_app.get_settings().config_dir, "inspectable.dot", "agent_requestable")

    response = product_api_client.get("/workspace/api/flows/inspectable.dot", params={"surface": "agent"})

    assert response.status_code == 200
    assert response.json() == {
        "name": "inspectable.dot",
        "title": "Inspectable Graph",
        "description": "Inspect graph behavior",
        "launch_policy": "agent_requestable",
        "effective_launch_policy": "agent_requestable",
        "execution_lock": None,
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
    set_flow_launch_policy(product_app.get_settings().config_dir, "ops/review/inspectable.dot", "agent_requestable")

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
    assert read_flow_launch_policy(product_app.get_settings().config_dir, "ops/review/inspectable.dot").launch_policy == "trigger_only"


def test_workspace_flow_agent_surface_hides_non_requestable_describe_and_raw(
    product_api_client: TestClient,
) -> None:
    _write_flow("trigger-only.dot", "digraph trigger_only { start -> done; }\n")
    set_flow_launch_policy(product_app.get_settings().config_dir, "trigger-only.dot", "trigger_only")

    describe_response = product_api_client.get("/workspace/api/flows/trigger-only.dot", params={"surface": "agent"})
    raw_response = product_api_client.get("/workspace/api/flows/trigger-only.dot/raw", params={"surface": "agent"})

    assert describe_response.status_code == 404
    assert raw_response.status_code == 404


def test_workspace_flow_raw_returns_dot_for_requestable_flow(
    product_api_client: TestClient,
) -> None:
    flow_content = 'digraph requestable { graph [label="Requestable"]; start -> done; }\n'
    _write_flow("requestable.dot", flow_content)
    set_flow_launch_policy(product_app.get_settings().config_dir, "requestable.dot", "agent_requestable")

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
        "execution_lock": None,
        "allowed_launch_policies": [
            "agent_requestable",
            "disabled",
            "trigger_only",
        ],
        "allowed_execution_lock_scopes": [
            "project",
        ],
        "allowed_execution_lock_conflict_policies": [
            "queue",
        ],
    }
    catalog_state = read_flow_launch_policy(product_app.get_settings().config_dir, "editable.dot")
    assert catalog_state.launch_policy == "trigger_only"


def test_flow_catalog_round_trip_persists_execution_lock_config() -> None:
    config_dir = product_app.get_settings().config_dir

    saved = set_flow_catalog_entry(
        config_dir,
        "locked.dot",
        launch_policy=LAUNCH_POLICY_DISABLED,
        execution_lock=FlowExecutionLockConfig(
            scope=EXECUTION_LOCK_SCOPE_PROJECT,
            key="main-worktree-integration",
            conflict_policy=EXECUTION_LOCK_CONFLICT_POLICY_QUEUE,
        ),
    )

    assert saved.launch_policy == LAUNCH_POLICY_DISABLED
    assert saved.execution_lock == FlowExecutionLockConfig(
        scope="project",
        key="main-worktree-integration",
        conflict_policy="queue",
    )

    reloaded = read_flow_launch_policy(config_dir, "locked.dot")
    assert reloaded.execution_lock == FlowExecutionLockConfig(
        scope="project",
        key="main-worktree-integration",
        conflict_policy="queue",
    )


@pytest.mark.asyncio
async def test_execution_lock_launches_immediately_when_unheld(product_api_client: TestClient, tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    project_dir = (tmp_path / "project").resolve()
    project_dir.mkdir()
    set_flow_catalog_entry(
        product_app.get_settings().config_dir,
        "locked.dot",
        launch_policy=LAUNCH_POLICY_DISABLED,
        execution_lock=FlowExecutionLockConfig(
            scope="project",
            key="main-worktree-integration",
            conflict_policy="queue",
        ),
    )

    start_calls: list[dict[str, object]] = []

    async def fake_start_pipeline_unlocked(req, *, run_id=None, execution_lock=None, on_complete=None):
        resolved_run_id = str(run_id or req.run_id or "run-1")
        start_calls.append(
            {
                "run_id": resolved_run_id,
                "working_directory": req.working_directory,
                "execution_lock": execution_lock.to_dict() if execution_lock is not None else None,
                "on_complete": on_complete,
            }
        )
        return {"status": "started", "run_id": resolved_run_id, "pipeline_id": resolved_run_id}

    monkeypatch.setattr(server, "_start_pipeline_unlocked", fake_start_pipeline_unlocked)

    result = await server._start_pipeline(
        server.PipelineStartRequest(
            flow_name="locked.dot",
            flow_content="digraph locked { start -> done; }",
            working_directory=str(project_dir),
        )
    )

    assert result["status"] == "started"
    assert len(start_calls) == 1
    assert start_calls[0]["run_id"] == result["run_id"]
    assert start_calls[0]["working_directory"] == str(project_dir)
    assert start_calls[0]["execution_lock"] == {
        "scope": "project",
        "key": "main-worktree-integration",
        "conflict_policy": "queue",
        "identity": start_calls[0]["execution_lock"]["identity"] if isinstance(start_calls[0]["execution_lock"], dict) else "",
        "state": "holding",
    }
    assert start_calls[0]["on_complete"] is not None


@pytest.mark.asyncio
async def test_execution_lock_queues_same_project_and_starts_fifo_on_completion(
    product_api_client: TestClient,
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    project_dir = (tmp_path / "project").resolve()
    project_dir.mkdir()
    set_flow_catalog_entry(
        product_app.get_settings().config_dir,
        "locked.dot",
        launch_policy=LAUNCH_POLICY_DISABLED,
        execution_lock=FlowExecutionLockConfig(
            scope="project",
            key="main-worktree-integration",
            conflict_policy="queue",
        ),
    )

    start_calls: list[dict[str, object]] = []
    completion_callbacks: dict[str, object] = {}

    async def fake_start_pipeline_unlocked(req, *, run_id=None, execution_lock=None, on_complete=None):
        resolved_run_id = str(run_id or req.run_id or f"run-{len(start_calls) + 1}")
        start_calls.append(
            {
                "run_id": resolved_run_id,
                "flow_name": req.flow_name,
                "execution_lock": execution_lock.to_dict() if execution_lock is not None else None,
            }
        )
        if on_complete is not None:
            completion_callbacks[resolved_run_id] = on_complete
        return {"status": "started", "run_id": resolved_run_id, "pipeline_id": resolved_run_id}

    monkeypatch.setattr(server, "_start_pipeline_unlocked", fake_start_pipeline_unlocked)

    first = await server._start_pipeline(
        server.PipelineStartRequest(
            flow_name="locked.dot",
            flow_content="digraph locked { start -> done; }",
            working_directory=str(project_dir),
        )
    )
    second = await server._start_pipeline(
        server.PipelineStartRequest(
            flow_name="locked.dot",
            flow_content="digraph locked { start -> done; }",
            working_directory=str(project_dir),
        )
    )
    third = await server._start_pipeline(
        server.PipelineStartRequest(
            flow_name="locked.dot",
            flow_content="digraph locked { start -> done; }",
            working_directory=str(project_dir),
        )
    )

    assert first["status"] == "started"
    assert second["status"] == "queued"
    assert third["status"] == "queued"
    assert len(start_calls) == 1

    first_complete = completion_callbacks[first["run_id"]]
    completion_result = first_complete(first["run_id"], "completed")
    if asyncio.iscoroutine(completion_result):
        await completion_result
    await asyncio.sleep(0)

    assert [call["run_id"] for call in start_calls] == [first["run_id"], second["run_id"]]

    second_complete = completion_callbacks[second["run_id"]]
    completion_result = second_complete(second["run_id"], "completed")
    if asyncio.iscoroutine(completion_result):
        await completion_result
    await asyncio.sleep(0)

    assert [call["run_id"] for call in start_calls] == [first["run_id"], second["run_id"], third["run_id"]]


@pytest.mark.asyncio
async def test_execution_lock_serializes_different_flows_sharing_same_key(
    product_api_client: TestClient,
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    project_dir = (tmp_path / "project").resolve()
    project_dir.mkdir()
    for flow_name in ("first.dot", "second.dot"):
        set_flow_catalog_entry(
            product_app.get_settings().config_dir,
            flow_name,
            launch_policy=LAUNCH_POLICY_DISABLED,
            execution_lock=FlowExecutionLockConfig(
                scope="project",
                key="shared-resource",
                conflict_policy="queue",
            ),
        )

    start_calls: list[str] = []

    async def fake_start_pipeline_unlocked(req, *, run_id=None, execution_lock=None, on_complete=None):
        resolved_run_id = str(run_id or req.run_id or f"run-{len(start_calls) + 1}")
        start_calls.append(str(req.flow_name))
        return {"status": "started", "run_id": resolved_run_id, "pipeline_id": resolved_run_id}

    monkeypatch.setattr(server, "_start_pipeline_unlocked", fake_start_pipeline_unlocked)

    first = await server._start_pipeline(
        server.PipelineStartRequest(
            flow_name="first.dot",
            flow_content="digraph first { start -> done; }",
            working_directory=str(project_dir),
        )
    )
    second = await server._start_pipeline(
        server.PipelineStartRequest(
            flow_name="second.dot",
            flow_content="digraph second { start -> done; }",
            working_directory=str(project_dir),
        )
    )

    assert first["status"] == "started"
    assert second["status"] == "queued"
    assert start_calls == ["first.dot"]


@pytest.mark.asyncio
async def test_execution_lock_does_not_block_different_projects_with_same_key(
    product_api_client: TestClient,
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    project_a = (tmp_path / "project-a").resolve()
    project_b = (tmp_path / "project-b").resolve()
    project_a.mkdir()
    project_b.mkdir()
    set_flow_catalog_entry(
        product_app.get_settings().config_dir,
        "locked.dot",
        launch_policy=LAUNCH_POLICY_DISABLED,
        execution_lock=FlowExecutionLockConfig(
            scope="project",
            key="main-worktree-integration",
            conflict_policy="queue",
        ),
    )

    start_calls: list[str] = []

    async def fake_start_pipeline_unlocked(req, *, run_id=None, execution_lock=None, on_complete=None):
        resolved_run_id = str(run_id or req.run_id or f"run-{len(start_calls) + 1}")
        start_calls.append(req.working_directory)
        return {"status": "started", "run_id": resolved_run_id, "pipeline_id": resolved_run_id}

    monkeypatch.setattr(server, "_start_pipeline_unlocked", fake_start_pipeline_unlocked)

    first = await server._start_pipeline(
        server.PipelineStartRequest(
            flow_name="locked.dot",
            flow_content="digraph locked { start -> done; }",
            working_directory=str(project_a),
        )
    )
    second = await server._start_pipeline(
        server.PipelineStartRequest(
            flow_name="locked.dot",
            flow_content="digraph locked { start -> done; }",
            working_directory=str(project_b),
        )
    )

    assert first["status"] == "started"
    assert second["status"] == "started"
    assert start_calls == [str(project_a), str(project_b)]
