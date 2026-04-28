from __future__ import annotations

from pathlib import Path

import pytest
from fastapi.testclient import TestClient

import attractor.api.server as server
from attractor.llm_runtime import (
    RUNTIME_LAUNCH_PROFILE_KEY,
    RUNTIME_LAUNCH_PROVIDER_KEY,
    RUNTIME_LAUNCH_REASONING_EFFORT_KEY,
)
from tests.api._support import (
    close_task_immediately as _close_task_immediately,
    start_pipeline as _start_pipeline,
)


def _seed_inactive_source_run(
    tmp_path: Path,
    *,
    run_id: str,
    status: str,
    flow_name: str = "source.dot",
) -> tuple[server.RunRecord, Path]:
    working_directory = tmp_path / "work"
    record = server.RunRecord(
        run_id=run_id,
        flow_name=flow_name,
        status=status,
        outcome="success" if status == "completed" else None,
        outcome_reason_code=None,
        outcome_reason_message=None,
        working_directory=str(working_directory),
        model="gpt-5.4-mini",
        started_at="2026-04-01T12:00:00Z",
        ended_at="2026-04-01T12:05:00Z",
        project_path=str(tmp_path),
    )
    server._write_run_meta(record)

    run_root = server._run_root(run_id)
    server.save_checkpoint(
        run_root / "state.json",
        server.Checkpoint(
            current_node="checkpoint",
            completed_nodes=["start"],
            context={
                "context.seed": "from-source",
                "internal.run_workdir": str(working_directory),
            },
            retry_counts={},
        ),
    )
    graphviz_dir = run_root / "artifacts" / "graphviz"
    graphviz_dir.mkdir(parents=True, exist_ok=True)
    graphviz_dir.joinpath("pipeline-source.dot").write_text(
        """
        digraph Source {
            start [shape=Mdiamond]
            checkpoint [shape=box]
            done [shape=Msquare]
            start -> checkpoint -> done
        }
        """,
        encoding="utf-8",
    )
    return record, run_root


def test_continue_pipeline_creates_new_run_with_lineage_and_preserves_source_run(
    attractor_api_client: TestClient,
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    server.configure_runtime_paths(runs_dir=tmp_path / "runs")
    monkeypatch.setattr(server.asyncio, "create_task", _close_task_immediately)

    source_payload = _start_pipeline(
        attractor_api_client,
        tmp_path / "work",
        flow_content="""
        digraph G {
            start [shape=Mdiamond]
            midpoint [shape=box]
            done [shape=Msquare]
            start -> midpoint -> done
        }
        """,
    )
    source_run_id = str(source_payload["run_id"])

    response = attractor_api_client.post(
        f"/pipelines/{source_run_id}/continue",
        json={
            "start_node": "midpoint",
            "flow_source_mode": "snapshot",
        },
    )

    assert response.status_code == 200
    payload = response.json()
    assert payload["status"] == "started"
    assert payload["run_id"] != source_run_id
    assert payload["provider"] == "codex"
    assert payload["llm_provider"] == "codex"
    assert payload["reasoning_effort"] is None

    source_record = server._read_run_meta(server._run_meta_path(source_run_id))
    derived_record = server._read_run_meta(server._run_meta_path(str(payload["run_id"])))
    assert source_record is not None
    assert derived_record is not None
    assert source_record.continued_from_run_id is None
    assert derived_record.continued_from_run_id == source_run_id
    assert derived_record.continued_from_node == "midpoint"
    assert derived_record.continued_from_flow_mode == "snapshot"
    assert derived_record.continued_from_flow_name is None
    assert derived_record.working_directory == source_record.working_directory

    detail_response = attractor_api_client.get(f"/pipelines/{payload['run_id']}")
    assert detail_response.status_code == 200
    detail_payload = detail_response.json()
    assert detail_payload["continued_from_run_id"] == source_run_id
    assert detail_payload["continued_from_node"] == "midpoint"
    assert detail_payload["continued_from_flow_mode"] == "snapshot"
    assert detail_payload["provider"] == "codex"
    assert detail_payload["llm_provider"] == "codex"
    assert detail_payload["reasoning_effort"] is None

    runs_response = attractor_api_client.get("/runs")
    assert runs_response.status_code == 200
    derived_run_payload = next(
        run for run in runs_response.json()["runs"] if run["run_id"] == payload["run_id"]
    )
    assert derived_run_payload["continued_from_run_id"] == source_run_id
    assert derived_run_payload["continued_from_node"] == "midpoint"
    assert derived_run_payload["provider"] == "codex"
    assert derived_run_payload["llm_provider"] == "codex"
    assert derived_run_payload["reasoning_effort"] is None


def test_continue_pipeline_snapshot_mode_uses_stored_run_dot_snapshot(
    attractor_api_client: TestClient,
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    server.configure_runtime_paths(runs_dir=tmp_path / "runs")
    monkeypatch.setattr(server.asyncio, "create_task", _close_task_immediately)

    source_payload = _start_pipeline(
        attractor_api_client,
        tmp_path / "work",
        flow_content="""
        digraph G {
            start [shape=Mdiamond]
            snapshot_only [shape=box]
            done [shape=Msquare]
            start -> snapshot_only -> done
        }
        """,
    )
    source_run_id = str(source_payload["run_id"])

    response = attractor_api_client.post(
        f"/pipelines/{source_run_id}/continue",
        json={
            "start_node": "snapshot_only",
            "flow_source_mode": "snapshot",
        },
    )

    assert response.status_code == 200
    payload = response.json()
    assert payload["status"] == "started"


def test_continue_pipeline_flow_name_mode_uses_selected_installed_flow(
    attractor_api_client: TestClient,
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    server.configure_runtime_paths(runs_dir=tmp_path / "runs", flows_dir=tmp_path / "flows")
    monkeypatch.setattr(server.asyncio, "create_task", _close_task_immediately)

    flow_path = server.get_runtime_paths().flows_dir / "override.dot"
    flow_path.parent.mkdir(parents=True, exist_ok=True)
    flow_path.write_text(
        """
        digraph Override {
            start [shape=Mdiamond]
            override_only [shape=box]
            done [shape=Msquare]
            start -> override_only -> done
        }
        """,
        encoding="utf-8",
    )

    source_payload = _start_pipeline(
        attractor_api_client,
        tmp_path / "work",
        flow_content="""
        digraph Source {
            start [shape=Mdiamond]
            source_only [shape=box]
            done [shape=Msquare]
            start -> source_only -> done
        }
        """,
    )
    source_run_id = str(source_payload["run_id"])

    response = attractor_api_client.post(
        f"/pipelines/{source_run_id}/continue",
        json={
            "start_node": "override_only",
            "flow_source_mode": "flow_name",
            "flow_name": "override.dot",
        },
    )

    assert response.status_code == 200
    payload = response.json()
    assert payload["status"] == "started"
    derived_record = server._read_run_meta(server._run_meta_path(str(payload["run_id"])))
    assert derived_record is not None
    assert derived_record.flow_name == "override.dot"
    assert derived_record.continued_from_flow_mode == "flow_name"
    assert derived_record.continued_from_flow_name == "override.dot"


def test_continue_pipeline_returns_validation_error_for_unknown_start_node(
    attractor_api_client: TestClient,
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    server.configure_runtime_paths(runs_dir=tmp_path / "runs")
    monkeypatch.setattr(server.asyncio, "create_task", _close_task_immediately)

    source_payload = _start_pipeline(attractor_api_client, tmp_path / "work")
    source_run_id = str(source_payload["run_id"])

    response = attractor_api_client.post(
        f"/pipelines/{source_run_id}/continue",
        json={
            "start_node": "missing-node",
            "flow_source_mode": "snapshot",
        },
    )

    assert response.status_code == 200
    payload = response.json()
    assert payload["status"] == "validation_error"
    assert payload["error"] == "Unknown start node: missing-node"


@pytest.mark.parametrize("source_status", ["completed", "failed", "blocked"])
def test_continue_pipeline_supports_inactive_source_runs_without_mutating_source_state(
    attractor_api_client: TestClient,
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
    source_status: str,
) -> None:
    server.configure_runtime_paths(runs_dir=tmp_path / "runs")
    monkeypatch.setattr(server.asyncio, "create_task", _close_task_immediately)

    source_record, source_run_root = _seed_inactive_source_run(
        tmp_path,
        run_id=f"source-{source_status}",
        status=source_status,
    )
    source_meta_before = server._run_meta_path(source_record.run_id).read_text(encoding="utf-8")
    source_checkpoint_before = (source_run_root / "state.json").read_text(encoding="utf-8")

    response = attractor_api_client.post(
        f"/pipelines/{source_record.run_id}/continue",
        json={
            "start_node": "checkpoint",
            "flow_source_mode": "snapshot",
        },
    )

    assert response.status_code == 200
    payload = response.json()
    assert payload["status"] == "started"
    assert payload["run_id"] != source_record.run_id

    derived_record = server._read_run_meta(server._run_meta_path(str(payload["run_id"])))
    derived_checkpoint = server.load_checkpoint(server._run_root(str(payload["run_id"])) / "state.json")
    assert derived_record is not None
    assert derived_checkpoint is not None
    assert derived_record.continued_from_run_id == source_record.run_id
    assert derived_record.continued_from_node == "checkpoint"
    assert derived_record.continued_from_flow_mode == "snapshot"
    assert derived_checkpoint.current_node == "checkpoint"
    assert derived_checkpoint.context["context.seed"] == "from-source"

    assert server._run_meta_path(source_record.run_id).read_text(encoding="utf-8") == source_meta_before
    assert (source_run_root / "state.json").read_text(encoding="utf-8") == source_checkpoint_before


def test_continue_pipeline_preserves_source_provider_and_reasoning_when_request_omits_them(
    attractor_api_client: TestClient,
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    server.configure_runtime_paths(runs_dir=tmp_path / "runs")
    monkeypatch.setattr(server.asyncio, "create_task", _close_task_immediately)

    source_record, _ = _seed_inactive_source_run(
        tmp_path,
        run_id="source-provider-reasoning",
        status="completed",
    )
    source_checkpoint = server.load_checkpoint(server._run_root(source_record.run_id) / "state.json")
    assert source_checkpoint is not None
    source_checkpoint.context[RUNTIME_LAUNCH_PROVIDER_KEY] = "anthropic"
    source_checkpoint.context[RUNTIME_LAUNCH_REASONING_EFFORT_KEY] = "medium"
    server.save_checkpoint(server._run_root(source_record.run_id) / "state.json", source_checkpoint)

    response = attractor_api_client.post(
        f"/pipelines/{source_record.run_id}/continue",
        json={
            "start_node": "checkpoint",
            "flow_source_mode": "snapshot",
        },
    )

    assert response.status_code == 200
    payload = response.json()
    assert payload["status"] == "started"
    assert payload["provider"] == "anthropic"
    assert payload["llm_provider"] == "anthropic"
    assert payload["reasoning_effort"] == "medium"
    derived_checkpoint = server.load_checkpoint(server._run_root(str(payload["run_id"])) / "state.json")
    assert derived_checkpoint is not None
    assert derived_checkpoint.context[RUNTIME_LAUNCH_PROVIDER_KEY] == "anthropic"
    assert derived_checkpoint.context[RUNTIME_LAUNCH_REASONING_EFFORT_KEY] == "medium"
    detail_payload = attractor_api_client.get(f"/pipelines/{payload['run_id']}").json()
    assert detail_payload["provider"] == "anthropic"
    assert detail_payload["llm_provider"] == "anthropic"
    assert detail_payload["reasoning_effort"] == "medium"


def test_continue_pipeline_preserves_source_llm_profile_when_request_omits_it(
    attractor_api_client: TestClient,
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    server.configure_runtime_paths(runs_dir=tmp_path / "runs")
    (server.get_runtime_paths().config_dir / "llm-profiles.toml").write_text(
        """
        [profiles.local]
        provider = "openai_compatible"
        base_url = "http://127.0.0.1:1234/v1"
        models = ["local-model"]
        """,
        encoding="utf-8",
    )
    monkeypatch.setattr(server.asyncio, "create_task", _close_task_immediately)

    source_record, _ = _seed_inactive_source_run(
        tmp_path,
        run_id="source-profile",
        status="completed",
    )
    source_record.model = "local-model"
    source_record.llm_profile = "local"
    server._write_run_meta(source_record)
    source_checkpoint = server.load_checkpoint(server._run_root(source_record.run_id) / "state.json")
    assert source_checkpoint is not None
    source_checkpoint.context[RUNTIME_LAUNCH_PROFILE_KEY] = "local"
    server.save_checkpoint(server._run_root(source_record.run_id) / "state.json", source_checkpoint)

    response = attractor_api_client.post(
        f"/pipelines/{source_record.run_id}/continue",
        json={
            "start_node": "checkpoint",
            "flow_source_mode": "snapshot",
        },
    )

    assert response.status_code == 200
    payload = response.json()
    assert payload["status"] == "started"
    assert payload["llm_profile"] == "local"
    derived_checkpoint = server.load_checkpoint(server._run_root(str(payload["run_id"])) / "state.json")
    assert derived_checkpoint is not None
    assert derived_checkpoint.context[RUNTIME_LAUNCH_PROFILE_KEY] == "local"
    detail_payload = attractor_api_client.get(f"/pipelines/{payload['run_id']}").json()
    assert detail_payload["llm_profile"] == "local"


def test_continue_pipeline_response_uses_explicit_request_provider_and_reasoning(
    attractor_api_client: TestClient,
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    server.configure_runtime_paths(runs_dir=tmp_path / "runs")
    monkeypatch.setattr(server.asyncio, "create_task", _close_task_immediately)

    source_record, _ = _seed_inactive_source_run(
        tmp_path,
        run_id="source-request-provider-reasoning",
        status="completed",
    )

    response = attractor_api_client.post(
        f"/pipelines/{source_record.run_id}/continue",
        json={
            "start_node": "checkpoint",
            "flow_source_mode": "snapshot",
            "llm_provider": "openai",
            "reasoning_effort": "xhigh",
        },
    )

    assert response.status_code == 200
    payload = response.json()
    assert payload["status"] == "started"
    assert payload["provider"] == "openai"
    assert payload["llm_provider"] == "openai"
    assert payload["reasoning_effort"] == "xhigh"

    detail_payload = attractor_api_client.get(f"/pipelines/{payload['run_id']}").json()
    assert detail_payload["provider"] == "openai"
    assert detail_payload["llm_provider"] == "openai"
    assert detail_payload["reasoning_effort"] == "xhigh"


def test_continue_pipeline_requires_flow_source_mode(
    attractor_api_client: TestClient,
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    server.configure_runtime_paths(runs_dir=tmp_path / "runs")
    monkeypatch.setattr(server.asyncio, "create_task", _close_task_immediately)

    source_record, _ = _seed_inactive_source_run(
        tmp_path,
        run_id="source-default-snapshot",
        status="completed",
    )

    response = attractor_api_client.post(
        f"/pipelines/{source_record.run_id}/continue",
        json={"start_node": "checkpoint"},
    )

    assert response.status_code == 422
    payload = response.json()
    assert payload["detail"][0]["loc"] == ["body", "flow_source_mode"]
    assert payload["detail"][0]["type"] == "missing"
