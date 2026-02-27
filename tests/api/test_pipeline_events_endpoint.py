from __future__ import annotations

import asyncio
import json
from pathlib import Path

import pytest

import attractor.api.server as server


class _RequestDisconnectAfterOneLoop:
    def __init__(self) -> None:
        self._checks = 0

    async def is_disconnected(self) -> bool:
        self._checks += 1
        return self._checks > 1


class _InterleavingEventHub:
    def __init__(self, run_id: str) -> None:
        self.run_id = run_id
        self._queue: asyncio.Queue[dict] = asyncio.Queue(maxsize=8)
        self._history: list[dict] = [
            {"type": "runtime", "status": "running", "seq": "history", "run_id": run_id},
            {"type": "runtime", "status": "running", "seq": "late", "run_id": run_id},
        ]
        self._injected = False

    async def publish(self, run_id: str, event: dict) -> None:  # pragma: no cover - helper shim
        del run_id
        await self._queue.put(dict(event))

    def history(self, run_id: str) -> list[dict]:
        assert run_id == self.run_id
        if not self._injected:
            self._queue.put_nowait(
                {"type": "runtime", "status": "running", "seq": "late", "run_id": run_id}
            )
            self._queue.put_nowait(
                {"type": "runtime", "status": "running", "seq": "live", "run_id": run_id}
            )
            self._injected = True
        return list(self._history)

    def subscribe(self, run_id: str) -> asyncio.Queue[dict]:
        assert run_id == self.run_id
        return self._queue

    def subscribe_with_history(self, run_id: str) -> tuple[asyncio.Queue[dict], list[dict]]:
        assert run_id == self.run_id
        self._queue.put_nowait(
            {"type": "runtime", "status": "running", "seq": "live", "run_id": run_id}
        )
        return self._queue, list(self._history)

    def unsubscribe(self, run_id: str, queue: asyncio.Queue[dict]) -> None:
        assert run_id == self.run_id
        assert queue is self._queue


def _decode_event(chunk: str | bytes) -> dict:
    text = chunk.decode("utf-8") if isinstance(chunk, bytes) else chunk
    lines = [line for line in text.splitlines() if line.startswith("data: ")]
    assert lines
    return json.loads(lines[0].removeprefix("data: "))


async def _collect_stream_events(run_id: str) -> list[dict]:
    response = await server.pipeline_events(run_id, _RequestDisconnectAfterOneLoop())
    iterator = response.body_iterator
    chunks: list[str | bytes] = []
    try:
        for _ in range(3):
            chunks.append(await anext(iterator))
    finally:
        await iterator.aclose()
    return [_decode_event(chunk) for chunk in chunks]


def test_pipeline_events_replays_history_without_replaying_same_event_from_live_queue(
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    run_id = "run-sse"
    monkeypatch.setattr(server, "RUNS_ROOT", tmp_path / "runs")

    hub = _InterleavingEventHub(run_id)
    monkeypatch.setattr(server, "EVENT_HUB", hub)

    with server.ACTIVE_RUNS_LOCK:
        server.ACTIVE_RUNS[run_id] = server.ActiveRun(
            run_id=run_id,
            flow_name="Flow",
            working_directory=str(tmp_path / "work"),
            model="test-model",
            status="running",
        )

    try:
        events = asyncio.run(_collect_stream_events(run_id))
    finally:
        server._pop_active_run(run_id)

    assert [event["seq"] for event in events] == ["history", "late", "live"]
