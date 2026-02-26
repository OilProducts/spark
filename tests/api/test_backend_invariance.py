from __future__ import annotations

import asyncio
from pathlib import Path

import pytest

import attractor.api.server as server


FLOW = """
digraph G {
    start [shape=Mdiamond]
    done [shape=Msquare]
    start -> done
}
"""


def _close_task_immediately(coro):
    coro.close()

    class _DummyTask:
        pass

    return _DummyTask()


@pytest.mark.parametrize("backend", ["codex", "codex-cli"])
def test_pipeline_definition_is_backend_invariant_for_backend_selection(
    backend: str, monkeypatch: pytest.MonkeyPatch, tmp_path: Path
) -> None:
    monkeypatch.setattr(server, "RUNS_ROOT", tmp_path / "runs")
    monkeypatch.setattr(server.asyncio, "create_task", _close_task_immediately)

    payload = asyncio.run(
        server._start_pipeline(
            server.PipelineStartRequest(
                flow_content=FLOW,
                working_directory=str(tmp_path / "work"),
                backend=backend,
            )
        )
    )
    assert payload["status"] == "started"
    assert payload["working_directory"] == str((tmp_path / "work").resolve())

    pipeline_id = payload["pipeline_id"]
    server._pop_active_run(pipeline_id)


def test_pipeline_emits_lifecycle_phases_in_spec_order(
    monkeypatch: pytest.MonkeyPatch, tmp_path: Path
) -> None:
    monkeypatch.setattr(server, "RUNS_ROOT", tmp_path / "runs")

    async def _start_and_wait() -> str:
        payload = await server._start_pipeline(
            server.PipelineStartRequest(
                flow_content=FLOW,
                working_directory=str(tmp_path / "work"),
                backend="codex",
            )
        )
        assert payload["status"] == "started"
        run_id = payload["run_id"]

        for _ in range(400):
            record = server._read_run_meta(server._run_meta_path(run_id))
            if record and record.status != "running":
                break
            await asyncio.sleep(0.01)
        else:
            raise AssertionError("timed out waiting for pipeline completion")

        return run_id

    run_id = asyncio.run(_start_and_wait())
    lifecycle_phases = [
        str(event.get("phase"))
        for event in server.EVENT_HUB.history(run_id)
        if event.get("type") == "lifecycle"
    ]

    assert lifecycle_phases == ["PARSE", "VALIDATE", "INITIALIZE", "EXECUTE", "FINALIZE"]
