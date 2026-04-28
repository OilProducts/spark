from __future__ import annotations

import asyncio
from pathlib import Path
import time

import pytest
from fastapi.testclient import TestClient

import attractor.api.server as server
from attractor.engine.outcome import Outcome, OutcomeStatus
from tests.api._support import wait_for_pipeline_completion


class _SequenceBackend:
    def __init__(self, outcomes: list[Outcome]):
        self._outcomes = list(outcomes)
        self.calls: list[str] = []
        self.run_kwargs: list[dict[str, object | None]] = []

    def run(
        self,
        node_id: str,
        prompt: str,
        context,
        *,
        response_contract: str = "",
        contract_repair_attempts: int = 0,
        timeout=None,
        model=None,
        provider=None,
        reasoning_effort=None,
        write_contract=None,
    ) -> Outcome:
        del prompt, context, response_contract, contract_repair_attempts, timeout, write_contract
        self.calls.append(node_id)
        self.run_kwargs.append(
            {
                "model": model,
                "provider": provider,
                "reasoning_effort": reasoning_effort,
            }
        )
        if self._outcomes:
            return self._outcomes.pop(0)
        return Outcome(status=OutcomeStatus.SUCCESS)

    def run_with_events(
        self,
        node_id: str,
        prompt: str,
        context,
        emit_event=None,
        *,
        response_contract: str = "",
        contract_repair_attempts: int = 0,
        timeout=None,
        model=None,
        provider=None,
        reasoning_effort=None,
        write_contract=None,
    ) -> Outcome:
        del emit_event
        return self.run(
            node_id,
            prompt,
            context,
            response_contract=response_contract,
            contract_repair_attempts=contract_repair_attempts,
            timeout=timeout,
            model=model,
            provider=provider,
            reasoning_effort=reasoning_effort,
            write_contract=write_contract,
        )


def test_retry_failed_pipeline_reuses_run_id_and_resumes_failed_checkpoint(
    attractor_api_client: TestClient,
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    backend = _SequenceBackend(
        [
            Outcome(status=OutcomeStatus.FAIL, failure_reason="first attempt failed"),
            Outcome(status=OutcomeStatus.SUCCESS),
        ]
    )

    def fake_build_backend(backend_name, working_dir, emit, *, model, on_usage_update=None):  # type: ignore[no-untyped-def]
        del backend_name, working_dir, emit, model, on_usage_update
        return backend

    monkeypatch.setattr(server, "_build_codergen_backend", fake_build_backend)
    workdir = tmp_path / "work"
    workdir.mkdir(parents=True, exist_ok=True)

    start_response = attractor_api_client.post(
        "/pipelines",
        json={
            "flow_content": """
            digraph RetryFlow {
                start [shape=Mdiamond]
                task [shape=box, prompt="Try once"]
                done [shape=Msquare]
                start -> task -> done
            }
            """,
            "working_directory": str(workdir),
            "backend": "codex-app-server",
        },
    )
    assert start_response.status_code == 200
    run_id = str(start_response.json()["run_id"])
    failed_payload = wait_for_pipeline_completion(attractor_api_client, run_id)
    assert failed_payload["status"] == "failed"
    assert backend.calls == ["task"]

    retry_response = attractor_api_client.post(f"/pipelines/{run_id}/retry")

    assert retry_response.status_code == 200
    retry_payload = retry_response.json()
    assert retry_payload["status"] == "started"
    assert retry_payload["run_id"] == run_id
    assert retry_payload["provider"] == "codex"
    assert retry_payload["llm_provider"] == "codex"
    assert retry_payload["reasoning_effort"] is None
    running_record = server._read_run_meta(server._run_meta_path(run_id))
    assert running_record is not None
    assert running_record.status == "running"
    assert running_record.last_error == ""

    final_payload = wait_for_pipeline_completion(attractor_api_client, run_id)
    assert final_payload["status"] == "completed"
    assert final_payload["run_id"] == run_id
    assert final_payload["provider"] == "codex"
    assert final_payload["llm_provider"] == "codex"
    assert final_payload["reasoning_effort"] is None
    assert backend.calls == ["task", "task"]

    runs = attractor_api_client.get("/runs").json()["runs"]
    assert [run["run_id"] for run in runs].count(run_id) == 1
    event_types = [event["type"] for event in server._read_persisted_run_events(run_id)]
    assert "PipelineRetryStarted" in event_types
    assert "PipelineRetryCompleted" in event_types


def test_retry_failed_pipeline_uses_provider_router_and_launch_reasoning_context(
    attractor_api_client: TestClient,
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    backend = _SequenceBackend(
        [
            Outcome(status=OutcomeStatus.FAIL, failure_reason="first attempt failed"),
            Outcome(status=OutcomeStatus.SUCCESS),
        ]
    )
    backend_names: list[str] = []

    def fake_build_backend(backend_name, working_dir, emit, *, model, on_usage_update=None):  # type: ignore[no-untyped-def]
        del working_dir, emit, model, on_usage_update
        backend_names.append(backend_name)
        return backend

    monkeypatch.setattr(server, "_build_codergen_backend", fake_build_backend)
    workdir = tmp_path / "work"
    workdir.mkdir(parents=True, exist_ok=True)

    start_response = attractor_api_client.post(
        "/pipelines",
        json={
            "flow_content": """
            digraph RetryProviderFlow {
                start [shape=Mdiamond]
                task [shape=box, prompt="Try once"]
                done [shape=Msquare]
                start -> task -> done
            }
            """,
            "working_directory": str(workdir),
            "model": "gpt-test",
            "llm_provider": "openai",
            "reasoning_effort": "low",
        },
    )
    assert start_response.status_code == 200
    assert start_response.json()["provider"] == "openai"
    assert start_response.json()["llm_provider"] == "openai"
    assert start_response.json()["reasoning_effort"] == "low"
    run_id = str(start_response.json()["run_id"])
    failed_payload = wait_for_pipeline_completion(attractor_api_client, run_id)
    assert failed_payload["status"] == "failed"

    retry_response = attractor_api_client.post(f"/pipelines/{run_id}/retry")

    assert retry_response.status_code == 200
    retry_payload = retry_response.json()
    assert retry_payload["provider"] == "openai"
    assert retry_payload["llm_provider"] == "openai"
    assert retry_payload["reasoning_effort"] == "low"
    final_payload = wait_for_pipeline_completion(attractor_api_client, run_id)
    assert final_payload["status"] == "completed"
    assert final_payload["provider"] == "openai"
    assert final_payload["llm_provider"] == "openai"
    assert final_payload["reasoning_effort"] == "low"
    assert backend_names == ["provider-router", "provider-router"]
    assert backend.calls == ["task", "task"]
    assert backend.run_kwargs == [
        {"model": "gpt-test", "provider": "openai", "reasoning_effort": "low"},
        {"model": "gpt-test", "provider": "openai", "reasoning_effort": "low"},
    ]
    listed_run = next(run for run in attractor_api_client.get("/runs").json()["runs"] if run["run_id"] == run_id)
    assert listed_run["provider"] == "openai"
    assert listed_run["llm_provider"] == "openai"
    assert listed_run["reasoning_effort"] == "low"


def test_retry_rejects_missing_inherited_provider_configuration(
    attractor_api_client: TestClient,
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    backend = _SequenceBackend(
        [
            Outcome(status=OutcomeStatus.FAIL, failure_reason="first attempt failed"),
            Outcome(status=OutcomeStatus.SUCCESS),
        ]
    )

    def fake_build_backend(backend_name, working_dir, emit, *, model, on_usage_update=None):  # type: ignore[no-untyped-def]
        del backend_name, working_dir, emit, model, on_usage_update
        return backend

    monkeypatch.setattr(server, "_build_codergen_backend", fake_build_backend)
    workdir = tmp_path / "work"
    workdir.mkdir(parents=True, exist_ok=True)

    start_response = attractor_api_client.post(
        "/pipelines",
        json={
            "flow_content": """
            digraph RetryProviderFlow {
                start [shape=Mdiamond]
                task [shape=box, prompt="Try once"]
                done [shape=Msquare]
                start -> task -> done
            }
            """,
            "working_directory": str(workdir),
            "model": "gpt-test",
            "llm_provider": "openai",
        },
    )
    assert start_response.status_code == 200
    run_id = str(start_response.json()["run_id"])
    failed_payload = wait_for_pipeline_completion(attractor_api_client, run_id)
    assert failed_payload["status"] == "failed"

    monkeypatch.delenv("OPENAI_API_KEY", raising=False)
    retry_response = attractor_api_client.post(f"/pipelines/{run_id}/retry")

    assert retry_response.status_code == 409
    assert retry_response.json()["detail"] == "Provider openai is not configured: missing OPENAI_API_KEY."
    assert backend.calls == ["task"]


def test_retry_start_publishes_running_run_list_upsert_without_sync_helper(
    attractor_api_client: TestClient,
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    backend = _SequenceBackend(
        [
            Outcome(status=OutcomeStatus.FAIL, failure_reason="first attempt failed"),
            Outcome(status=OutcomeStatus.SUCCESS),
        ]
    )

    def fake_build_backend(backend_name, working_dir, emit, *, model, on_usage_update=None):  # type: ignore[no-untyped-def]
        del backend_name, working_dir, emit, model, on_usage_update
        return backend

    monkeypatch.setattr(server, "_build_codergen_backend", fake_build_backend)
    workdir = tmp_path / "work"
    workdir.mkdir(parents=True, exist_ok=True)
    start_response = attractor_api_client.post(
        "/pipelines",
        json={
            "flow_content": """
            digraph RetryUpsertFlow {
                start [shape=Mdiamond]
                task [shape=box, prompt="Try once"]
                done [shape=Msquare]
                start -> task -> done
            }
            """,
            "working_directory": str(workdir),
            "backend": "codex-app-server",
        },
    )
    assert start_response.status_code == 200
    run_id = str(start_response.json()["run_id"])
    failed_payload = wait_for_pipeline_completion(attractor_api_client, run_id)
    assert failed_payload["status"] == "failed"

    hub = server.RunListEventHub()
    monkeypatch.setattr(server, "RUNS_EVENT_HUB", hub)
    queue = hub.subscribe()
    sync_publish_calls: list[str] = []

    def fail_sync_publish(loop, published_run_id):  # type: ignore[no-untyped-def]
        del loop
        sync_publish_calls.append(published_run_id)
        raise AssertionError("retry start must await _publish_run_list_upsert")

    monkeypatch.setattr(server, "_publish_run_list_upsert_sync", fail_sync_publish)

    started_at = time.perf_counter()
    retry_response = attractor_api_client.post(f"/pipelines/{run_id}/retry")
    elapsed_seconds = time.perf_counter() - started_at

    assert retry_response.status_code == 200
    assert retry_response.json()["status"] == "started"
    assert elapsed_seconds < 2.0
    assert sync_publish_calls == []

    running_event = asyncio.run(asyncio.wait_for(queue.get(), timeout=1.0))
    assert running_event["type"] == "run_upsert"
    assert running_event["run"]["run_id"] == run_id
    assert running_event["run"]["status"] == "running"
    assert running_event["run"]["last_error"] == ""
    assert running_event["run"]["ended_at"] is None

    final_payload = wait_for_pipeline_completion(attractor_api_client, run_id)
    assert final_payload["status"] == "completed"


def test_retry_rejects_non_failed_pipeline(
    attractor_api_client: TestClient,
    tmp_path: Path,
) -> None:
    workdir = tmp_path / "work"
    workdir.mkdir(parents=True, exist_ok=True)
    response = attractor_api_client.post(
        "/pipelines",
        json={
            "flow_content": """
            digraph CompleteFlow {
                start [shape=Mdiamond]
                done [shape=Msquare]
                start -> done
            }
            """,
            "working_directory": str(workdir),
            "backend": "codex-app-server",
        },
    )
    assert response.status_code == 200
    run_id = str(response.json()["run_id"])
    final_payload = wait_for_pipeline_completion(attractor_api_client, run_id)
    assert final_payload["status"] == "completed"

    retry_response = attractor_api_client.post(f"/pipelines/{run_id}/retry")

    assert retry_response.status_code == 409
    assert retry_response.json()["detail"] == "Retry requires a failed pipeline"


@pytest.mark.parametrize(
    ("snapshot_state", "expected_detail"),
    [
        ("missing", "Retry requires an available graph snapshot"),
        ("invalid", "Stored graph snapshot is invalid:"),
    ],
)
def test_retry_rejected_by_unavailable_graph_snapshot_preserves_run_state(
    attractor_api_client: TestClient,
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
    snapshot_state: str,
    expected_detail: str,
) -> None:
    backend = _SequenceBackend(
        [
            Outcome(status=OutcomeStatus.FAIL, failure_reason="first attempt failed"),
            Outcome(status=OutcomeStatus.SUCCESS),
        ]
    )

    def fake_build_backend(backend_name, working_dir, emit, *, model, on_usage_update=None):  # type: ignore[no-untyped-def]
        del backend_name, working_dir, emit, model, on_usage_update
        return backend

    monkeypatch.setattr(server, "_build_codergen_backend", fake_build_backend)
    workdir = tmp_path / "work"
    workdir.mkdir(parents=True, exist_ok=True)

    start_response = attractor_api_client.post(
        "/pipelines",
        json={
            "flow_content": """
            digraph RetrySnapshotFlow {
                start [shape=Mdiamond]
                task [shape=box, prompt="Fail before retry"]
                done [shape=Msquare]
                start -> task -> done
            }
            """,
            "working_directory": str(workdir),
            "backend": "codex-app-server",
        },
    )
    assert start_response.status_code == 200
    run_id = str(start_response.json()["run_id"])
    failed_payload = wait_for_pipeline_completion(attractor_api_client, run_id)
    assert failed_payload["status"] == "failed"

    before_checkpoint = attractor_api_client.get(f"/pipelines/{run_id}/checkpoint").json()["checkpoint"]
    before_status = attractor_api_client.get(f"/pipelines/{run_id}").json()

    graph_dir = server._run_root(run_id) / "artifacts" / "graphviz"
    if snapshot_state == "missing":
        (graph_dir / "pipeline-source.dot").unlink(missing_ok=True)
        (graph_dir / "pipeline.dot").unlink(missing_ok=True)
    else:
        (graph_dir / "pipeline-source.dot").write_text("digraph RetrySnapshotFlow { start ->", encoding="utf-8")

    retry_response = attractor_api_client.post(f"/pipelines/{run_id}/retry")

    assert retry_response.status_code == 409
    assert retry_response.json()["detail"].startswith(expected_detail)
    after_checkpoint = attractor_api_client.get(f"/pipelines/{run_id}/checkpoint").json()["checkpoint"]
    after_status = attractor_api_client.get(f"/pipelines/{run_id}").json()

    assert after_checkpoint == before_checkpoint
    metadata_fields = [
        "run_id",
        "flow_name",
        "status",
        "outcome",
        "outcome_reason_code",
        "outcome_reason_message",
        "working_directory",
        "model",
        "started_at",
        "ended_at",
        "project_path",
        "git_branch",
        "git_commit",
        "spec_id",
        "plan_id",
        "continued_from_run_id",
        "continued_from_node",
        "continued_from_flow_mode",
        "continued_from_flow_name",
        "parent_run_id",
        "parent_node_id",
        "root_run_id",
        "child_invocation_index",
        "last_error",
        "token_usage",
        "token_usage_breakdown",
        "estimated_model_cost",
    ]
    assert {field: after_status[field] for field in metadata_fields} == {
        field: before_status[field] for field in metadata_fields
    }
    assert backend.calls == ["task"]


def test_retry_parent_after_child_retry_reuses_linked_child_run(
    attractor_api_client: TestClient,
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    backend = _SequenceBackend(
        [
            Outcome(status=OutcomeStatus.FAIL, failure_reason="child failed"),
            Outcome(status=OutcomeStatus.SUCCESS),
        ]
    )

    def fake_build_backend(backend_name, working_dir, emit, *, model, on_usage_update=None):  # type: ignore[no-untyped-def]
        del backend_name, working_dir, emit, model, on_usage_update
        return backend

    monkeypatch.setattr(server, "_build_codergen_backend", fake_build_backend)
    child_dot_path = tmp_path / "child.dot"
    child_dot_path.write_text(
        """
        digraph Child {
            start [shape=Mdiamond]
            task [shape=box, prompt="Child task"]
            done [shape=Msquare]
            start -> task -> done
        }
        """,
        encoding="utf-8",
    )
    workdir = tmp_path / "work"
    workdir.mkdir(parents=True, exist_ok=True)

    start_response = attractor_api_client.post(
        "/pipelines",
        json={
            "flow_content": f"""
            digraph Parent {{
                graph [stack.child_dotfile="{child_dot_path}"]
                start [shape=Mdiamond]
                manager [shape=house, type="stack.manager_loop", manager.actions="", manager.max_cycles=1]
                done [shape=Msquare]
                start -> manager
                manager -> done [condition="outcome=success"]
            }}
            """,
            "working_directory": str(workdir),
            "backend": "codex-app-server",
        },
    )
    assert start_response.status_code == 200
    parent_run_id = str(start_response.json()["run_id"])
    parent_failed = wait_for_pipeline_completion(attractor_api_client, parent_run_id)
    assert parent_failed["status"] == "failed"
    parent_context = attractor_api_client.get(f"/pipelines/{parent_run_id}/context").json()["context"]
    child_run_id = str(parent_context["context.stack.child.run_id"])
    assert child_run_id

    child_failed = attractor_api_client.get(f"/pipelines/{child_run_id}").json()
    assert child_failed["status"] == "failed"

    child_retry_response = attractor_api_client.post(f"/pipelines/{child_run_id}/retry")
    assert child_retry_response.status_code == 200
    child_completed = wait_for_pipeline_completion(attractor_api_client, child_run_id)
    assert child_completed["status"] == "completed"

    parent_retry_response = attractor_api_client.post(f"/pipelines/{parent_run_id}/retry")
    assert parent_retry_response.status_code == 200
    parent_completed = wait_for_pipeline_completion(attractor_api_client, parent_run_id)
    assert parent_completed["status"] == "completed"
    assert backend.calls == ["task", "task"]

    child_runs = [
        run for run in attractor_api_client.get("/runs").json()["runs"]
        if run["parent_run_id"] == parent_run_id and run["parent_node_id"] == "manager"
    ]
    assert [run["run_id"] for run in child_runs] == [child_run_id]
