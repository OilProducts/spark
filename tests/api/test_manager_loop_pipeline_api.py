from __future__ import annotations

import json
from pathlib import Path

from fastapi.testclient import TestClient

import attractor.api.server as server
from attractor.engine.outcome import Outcome, OutcomeStatus
from tests.api._support import wait_for_pipeline_completion
from tests.support.flow_fixtures import seed_flow_fixture


class _LoopingManagerBackend:
    def __init__(self) -> None:
        self.gate_calls = 0
        self.child_task_notes: list[str] = []

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
        write_contract=None,
    ) -> Outcome:
        del prompt, context, response_contract, contract_repair_attempts, timeout, model, write_contract
        if node_id == "task":
            note = f"child-task-{len(self.child_task_notes) + 1}"
            self.child_task_notes.append(note)
            return Outcome(status=OutcomeStatus.SUCCESS, notes=note)
        if node_id == "gate":
            self.gate_calls += 1
            preferred_label = "Again" if self.gate_calls == 1 else "Done"
            return Outcome(status=OutcomeStatus.SUCCESS, preferred_label=preferred_label)
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
        del emit_event, provider, reasoning_effort
        return self.run(
            node_id,
            prompt,
            context,
            response_contract=response_contract,
            contract_repair_attempts=contract_repair_attempts,
            timeout=timeout,
            model=model,
            write_contract=write_contract,
        )


def test_pipeline_flow_name_resolves_relative_manager_child_paths_from_parent_flow_dir(
    attractor_api_client: TestClient,
    tmp_path: Path,
) -> None:
    flows_dir = tmp_path / "flows"
    flows_dir.mkdir(parents=True, exist_ok=True)
    seed_flow_fixture(flows_dir, "supervision/implementation-worker.dot", as_name="test-supervision/implementation-worker.dot")
    seed_flow_fixture(flows_dir, "supervision/supervised-manager.dot", as_name="test-supervision/supervised-manager.dot")
    workdir = tmp_path / "project"
    workdir.mkdir(parents=True, exist_ok=True)

    response = attractor_api_client.post(
        "/pipelines",
        json={
            "flow_name": "test-supervision/supervised-manager.dot",
            "working_directory": str(workdir),
            "backend": "codex-app-server",
        },
    )

    assert response.status_code == 200, response.text
    payload = response.json()
    assert payload["status"] == "started"

    final_payload = wait_for_pipeline_completion(attractor_api_client, payload["run_id"])

    assert final_payload["status"] == "completed"
    assert final_payload["outcome"] == "success"


def test_manager_child_launch_creates_first_class_child_run(
    attractor_api_client: TestClient,
    tmp_path: Path,
) -> None:
    flows_dir = tmp_path / "flows"
    flows_dir.mkdir(parents=True, exist_ok=True)
    seed_flow_fixture(flows_dir, "supervision/implementation-worker.dot", as_name="supervision/implementation-worker.dot")
    seed_flow_fixture(flows_dir, "supervision/supervised-manager.dot", as_name="supervision/supervised-manager.dot")
    workdir = tmp_path / "project"
    workdir.mkdir(parents=True, exist_ok=True)

    response = attractor_api_client.post(
        "/pipelines",
        json={
            "flow_name": "supervision/supervised-manager.dot",
            "working_directory": str(workdir),
            "backend": "codex-app-server",
        },
    )

    assert response.status_code == 200, response.text
    parent_run_id = str(response.json()["run_id"])
    final_payload = wait_for_pipeline_completion(attractor_api_client, parent_run_id)

    assert final_payload["status"] == "completed"
    parent_context = attractor_api_client.get(f"/pipelines/{parent_run_id}/context").json()["context"]
    child_run_id = parent_context["context.stack.child.run_id"]
    assert child_run_id
    assert child_run_id != parent_run_id

    runs = attractor_api_client.get("/runs").json()["runs"]
    child_run = next(run for run in runs if run["run_id"] == child_run_id)
    assert child_run["status"] == "completed"
    assert child_run["parent_run_id"] == parent_run_id
    assert child_run["parent_node_id"] == "manager"
    assert child_run["root_run_id"] == parent_run_id
    assert child_run["child_invocation_index"] == 1

    parent_run_root = server._run_root(parent_run_id)
    child_run_root = server._run_root(child_run_id)
    assert child_run_root != parent_run_root
    assert (child_run_root / "run.json").exists()
    assert (child_run_root / "state.json").exists()
    assert not (parent_run_root / "logs" / "manager" / "child").exists()

    parent_event_types = [event["type"] for event in server._read_persisted_run_events(parent_run_id)]
    assert "ChildRunStarted" in parent_event_types
    assert "ChildRunCompleted" in parent_event_types


def test_manager_child_provider_validation_failure_fails_parent_with_specific_reason(
    attractor_api_client: TestClient,
    monkeypatch,
    tmp_path: Path,
) -> None:
    monkeypatch.delenv("OPENAI_API_KEY", raising=False)
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
    workdir = tmp_path / "project"
    workdir.mkdir(parents=True, exist_ok=True)

    response = attractor_api_client.post(
        "/pipelines",
        json={
            "flow_content": f"""
            digraph Parent {{
                graph [stack.child_dotfile="{child_dot_path}"]
                start [shape=Mdiamond]
                manager [shape=house, type="stack.manager_loop", manager.actions="", manager.max_cycles=1]
                done [shape=Msquare]

                start -> manager -> done
            }}
            """,
            "working_directory": str(workdir),
            "llm_provider": "openai",
        },
    )

    assert response.status_code == 200, response.text
    parent_run_id = str(response.json()["run_id"])
    final_payload = wait_for_pipeline_completion(attractor_api_client, parent_run_id)

    assert final_payload["status"] == "failed"
    assert final_payload["last_error"] == "Provider openai is not configured: missing OPENAI_API_KEY."
    parent_context = attractor_api_client.get(f"/pipelines/{parent_run_id}/context").json()["context"]
    assert parent_context["context.stack.child.status"] == "failed"
    assert parent_context["context.stack.child.failure_reason"] == final_payload["last_error"]


def test_revisiting_same_manager_node_creates_separate_child_runs_with_own_state(
    attractor_api_client: TestClient,
    monkeypatch,
    tmp_path: Path,
) -> None:
    backend = _LoopingManagerBackend()

    def fake_build_backend(  # type: ignore[no-untyped-def]
        backend_name,
        working_dir,
        emit,
        *,
        model=None,
        on_usage_update=None,
    ):
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
    workdir = tmp_path / "project"
    workdir.mkdir(parents=True, exist_ok=True)

    response = attractor_api_client.post(
        "/pipelines",
        json={
            "flow_content": f"""
            digraph Parent {{
                graph [stack.child_dotfile="{child_dot_path}"]
                start [shape=Mdiamond]
                manager [shape=house, type="stack.manager_loop", manager.actions="", manager.max_cycles=1]
                gate [shape=box, prompt="Gate"]
                done [shape=Msquare]

                start -> manager
                manager -> gate [condition="outcome=success"]
                gate -> manager [condition="preferred_label=Again"]
                gate -> done [condition="preferred_label=Done"]
            }}
            """,
            "working_directory": str(workdir),
            "backend": "codex-app-server",
        },
    )

    assert response.status_code == 200, response.text
    parent_run_id = str(response.json()["run_id"])
    final_payload = wait_for_pipeline_completion(attractor_api_client, parent_run_id)

    assert final_payload["status"] == "completed"
    assert backend.child_task_notes == ["child-task-1", "child-task-2"]
    parent_context = attractor_api_client.get(f"/pipelines/{parent_run_id}/context").json()["context"]
    parent_events = server._read_persisted_run_events(parent_run_id)
    child_run_ids = [
        str(event["child_run_id"])
        for event in parent_events
        if event.get("type") == "ChildRunStarted"
    ]
    assert len(child_run_ids) == 2
    assert len(set(child_run_ids)) == 2
    assert parent_context["context.stack.child.run_id"] == child_run_ids[-1]

    parent_run_root = server._run_root(parent_run_id)
    for index, child_run_id in enumerate(child_run_ids, start=1):
        child_record = server._read_run_meta(server._run_meta_path(child_run_id))
        assert child_record is not None
        assert child_record.parent_run_id == parent_run_id
        assert child_record.parent_node_id == "manager"
        assert child_record.root_run_id == parent_run_id
        assert child_record.child_invocation_index == index

        child_run_root = server._run_root(child_run_id)
        assert child_run_root != parent_run_root
        assert (child_run_root / "state.json").exists()
        child_response = child_run_root / "logs" / "task" / "response.md"
        assert child_response.read_text(encoding="utf-8").strip() == f"child-task-{index}"
        child_status = json.loads((child_run_root / "logs" / "task" / "status.json").read_text(encoding="utf-8"))
        assert child_status["notes"] == f"child-task-{index}"

        child_events = server._read_persisted_run_events(child_run_id)
        child_started = next(event for event in child_events if event.get("type") == "PipelineStarted")
        assert child_started["resumed"] is False
