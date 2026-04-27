from __future__ import annotations

import asyncio
import json
from pathlib import Path

import pytest
from fastapi.testclient import TestClient

import attractor.api.server as server
from tests.api._support import (
    start_pipeline as _start_pipeline,
    wait_for_pipeline_completion as _wait_for_pipeline_completion,
)


class _RequestDisconnectAfterLoops:
    def __init__(self, *, loops: int) -> None:
        self._loops = loops
        self._checks = 0

    async def is_disconnected(self) -> bool:
        self._checks += 1
        return self._checks > self._loops


class _RequestNeverDisconnect:
    async def is_disconnected(self) -> bool:
        return False


class _QueueOnlyEventHub:
    def __init__(self, run_id: str, queued_events: list[dict] | None = None) -> None:
        self.run_id = run_id
        self._queue: asyncio.Queue[dict] = asyncio.Queue(maxsize=8)
        for event in queued_events or []:
            self._queue.put_nowait(dict(event))

    async def publish(self, run_id: str, event: dict) -> None:  # pragma: no cover - helper shim
        assert run_id == self.run_id
        await self._queue.put(dict(event))

    def history(self, run_id: str) -> list[dict]:
        assert run_id == self.run_id
        return []

    def subscribe(self, run_id: str) -> asyncio.Queue[dict]:
        assert run_id == self.run_id
        return self._queue

    def unsubscribe(self, run_id: str, queue: asyncio.Queue[dict]) -> None:
        assert run_id == self.run_id
        assert queue is self._queue


class _SubscriberAwareEventHub:
    def __init__(self, run_id: str) -> None:
        self.run_id = run_id
        self._history: list[dict] = []
        self._subscribers: list[asyncio.Queue[dict]] = []

    async def publish(self, run_id: str, event: dict) -> None:
        assert run_id == self.run_id
        self.inject(dict(event))

    def inject(self, event: dict) -> None:
        payload = dict(event)
        self._history.append(payload)
        for queue in list(self._subscribers):
            try:
                queue.put_nowait(dict(payload))
            except asyncio.QueueFull:
                continue

    def history(self, run_id: str) -> list[dict]:
        assert run_id == self.run_id
        return list(self._history)

    def subscribe(self, run_id: str) -> asyncio.Queue[dict]:
        assert run_id == self.run_id
        queue: asyncio.Queue[dict] = asyncio.Queue(maxsize=8)
        self._subscribers.append(queue)
        return queue

    def unsubscribe(self, run_id: str, queue: asyncio.Queue[dict]) -> None:
        assert run_id == self.run_id
        if queue in self._subscribers:
            self._subscribers.remove(queue)


def _decode_event(chunk: str | bytes) -> dict:
    text = chunk.decode("utf-8") if isinstance(chunk, bytes) else chunk
    lines = [line for line in text.splitlines() if line.startswith("data: ")]
    assert lines
    return json.loads(lines[0].removeprefix("data: "))


def _write_persisted_events(run_id: str, working_directory: Path, events: list[dict]) -> None:
    run_root = server._ensure_run_root_for_project(run_id, str(working_directory))
    events_path = run_root / "events.jsonl"
    with events_path.open("w", encoding="utf-8") as handle:
        for event in events:
            handle.write(json.dumps(event, sort_keys=True) + "\n")


def _register_known_run(run_id: str, working_directory: Path) -> None:
    server._record_run_start(
        run_id,
        flow_name="Flow",
        working_directory=str(working_directory),
        model="test-model",
    )


async def _collect_stream_events(
    run_id: str,
    *,
    count: int,
    after_sequence: int | None = None,
) -> list[dict]:
    response = await server.pipeline_events(
        run_id,
        _RequestDisconnectAfterLoops(loops=count),
        after_sequence=after_sequence,
    )
    iterator = response.body_iterator
    chunks: list[str | bytes] = []
    try:
        for _ in range(count):
            chunks.append(await anext(iterator))
    finally:
        await iterator.aclose()
    return [_decode_event(chunk) for chunk in chunks]


async def _collect_stream_events_until_timeout(
    response,
    *,
    timeout_seconds: float,
    max_events: int,
) -> list[dict]:
    iterator = response.body_iterator
    events: list[dict] = []
    try:
        while len(events) < max_events:
            try:
                chunk = await asyncio.wait_for(anext(iterator), timeout=timeout_seconds)
            except (asyncio.TimeoutError, StopAsyncIteration):
                break
            text = chunk.decode("utf-8") if isinstance(chunk, bytes) else chunk
            if text.startswith(": keepalive"):
                continue
            events.append(_decode_event(text))
    finally:
        await iterator.aclose()
    return events


async def _collect_stream_events_for_throughput(
    run_id: str,
    hub: server.PipelineEventHub,
    overflow: int = 40,
) -> tuple[list[dict], int, int]:
    response = await server.pipeline_events(run_id, _RequestNeverDisconnect())
    iterator = response.body_iterator
    queue = hub._subscribers[run_id][0]
    queue_maxsize = queue.maxsize
    assert queue_maxsize > 0
    publish_count = queue_maxsize + overflow
    for index in range(publish_count):
        await hub.publish(
            run_id,
            {
                "type": "runtime",
                "status": "running",
                "sequence": index + 1,
                "emitted_at": "2026-04-06T12:00:00Z",
                "run_id": run_id,
            },
        )
    events: list[dict] = []
    try:
        while len(events) < queue_maxsize:
            chunk = await anext(iterator)
            text = chunk.decode("utf-8") if isinstance(chunk, bytes) else chunk
            if text.startswith(": keepalive"):
                continue
            events.append(_decode_event(text))
    finally:
        await iterator.aclose()
    return events, queue_maxsize, publish_count


def test_parse_failure_persists_error_on_requested_run(tmp_path: Path) -> None:
    run_id = "run-sse-parse-failure"
    old_run_id = "run-sse-previous"
    working_directory = tmp_path / "work"
    working_directory.mkdir()
    server.configure_runtime_paths(runs_dir=tmp_path / "runs")
    server._ensure_run_root_for_project(old_run_id, str(working_directory))

    payload = asyncio.run(
        server._start_pipeline(
            server.PipelineStartRequest(
                flow_content="digraph G { start -> ",
                working_directory=str(working_directory),
                backend="codex-app-server",
            ),
            run_id=run_id,
        )
    )

    assert payload["status"] == "validation_error"
    run_events = server._read_persisted_run_events(run_id)
    assert [event["run_id"] for event in run_events] == [run_id, run_id]
    assert run_events[0]["type"] == "lifecycle"
    assert run_events[0]["phase"] == "PARSE"
    assert run_events[1]["type"] == "log"
    assert "Parse error" in str(run_events[1]["msg"])
    assert server._read_persisted_run_events(old_run_id) == []


def test_validation_error_attempt_reserves_run_id(tmp_path: Path) -> None:
    run_id = "run-sse-validation"
    working_directory = tmp_path / "work"
    working_directory.mkdir()
    server.configure_runtime_paths(runs_dir=tmp_path / "runs")

    invalid_payload = asyncio.run(
        server._start_pipeline(
            server.PipelineStartRequest(
                flow_content="digraph G { start -> ",
                working_directory=str(working_directory),
                backend="codex-app-server",
            ),
            run_id=run_id,
        )
    )
    assert invalid_payload["status"] == "validation_error"

    valid_payload = asyncio.run(
        server._start_pipeline(
            server.PipelineStartRequest(
                flow_content="""
                digraph G {
                    start [shape=Mdiamond]
                    done [shape=Msquare]
                    start -> done
                }
                """,
                working_directory=str(working_directory),
                backend="codex-app-server",
            ),
            run_id=run_id,
        )
    )

    assert valid_payload["status"] == "validation_error"
    assert valid_payload["error"] == f"Run id already exists: {run_id}"
    assert [event["type"] for event in server._read_persisted_run_events(run_id)] == ["lifecycle", "log"]


def test_pipeline_journal_returns_newest_first_durable_history_for_completed_run(tmp_path: Path) -> None:
    run_id = "run-journal-history"
    working_directory = tmp_path / "work"
    working_directory.mkdir()
    server.configure_runtime_paths(runs_dir=tmp_path / "runs")
    _register_known_run(run_id, working_directory)
    _write_persisted_events(
        run_id,
        working_directory,
        [
            {
                "type": "StageCompleted",
                "run_id": run_id,
                "sequence": 1,
                "emitted_at": "2026-04-06T12:00:00Z",
                "node_id": "prepare",
                "index": 1,
                "source_scope": "root",
            },
            {
                "type": "StageStarted",
                "run_id": run_id,
                "sequence": 2,
                "emitted_at": "2026-04-06T12:00:05Z",
                "node_id": "plan_current",
                "index": 2,
                "llm_model": "gpt-node-override",
                "source_scope": "child",
                "source_parent_node_id": "run_milestone",
                "source_flow_name": "implement-milestone.dot",
            },
        ],
    )

    page = asyncio.run(server.pipeline_journal(run_id))

    assert [entry["sequence"] for entry in page["entries"]] == [2, 1]
    assert page["newest_sequence"] == 2
    assert page["oldest_sequence"] == 1
    assert page["has_older"] is False
    assert page["entries"][0]["emitted_at"] == "2026-04-06T12:00:05Z"
    assert page["entries"][0]["kind"] == "stage"
    assert page["entries"][0]["payload"]["llm_model"] == "gpt-node-override"
    assert page["entries"][0]["source_scope"] == "child"
    assert page["entries"][0]["source_parent_node_id"] == "run_milestone"
    assert page["entries"][0]["source_flow_name"] == "implement-milestone.dot"


def test_pipeline_journal_paginates_older_entries_and_preserves_log_and_interview_provenance(
    tmp_path: Path,
) -> None:
    run_id = "run-journal-pagination"
    working_directory = tmp_path / "work"
    working_directory.mkdir()
    server.configure_runtime_paths(runs_dir=tmp_path / "runs")
    _register_known_run(run_id, working_directory)
    _write_persisted_events(
        run_id,
        working_directory,
        [
            {
                "type": "log",
                "run_id": run_id,
                "sequence": 1,
                "emitted_at": "2026-04-06T12:00:00Z",
                "msg": "warn: disk almost full",
                "source_scope": "root",
            },
            {
                "type": "InterviewTimeout",
                "run_id": run_id,
                "sequence": 2,
                "emitted_at": "2026-04-06T12:00:01Z",
                "stage": "review_gate",
                "question_id": "question-1",
                "default_choice_label": "Fix",
                "source_scope": "root",
            },
            {
                "type": "InterviewCompleted",
                "run_id": run_id,
                "sequence": 3,
                "emitted_at": "2026-04-06T12:00:02Z",
                "stage": "review_gate",
                "question_id": "question-1",
                "answer": "Approve",
                "outcome_provenance": "accepted",
                "source_scope": "root",
            },
            {
                "type": "StageCompleted",
                "run_id": run_id,
                "sequence": 4,
                "emitted_at": "2026-04-06T12:00:03Z",
                "node_id": "done",
                "index": 3,
                "source_scope": "root",
            },
        ],
    )

    latest_page = asyncio.run(server.pipeline_journal(run_id, limit=2))
    older_page = asyncio.run(server.pipeline_journal(run_id, limit=2, before_sequence=3))

    assert [entry["sequence"] for entry in latest_page["entries"]] == [4, 3]
    assert latest_page["has_older"] is True
    assert [entry["sequence"] for entry in older_page["entries"]] == [2, 1]
    assert older_page["has_older"] is False
    assert older_page["entries"][0]["question_id"] == "question-1"
    assert older_page["entries"][0]["summary"] == "Interview timed out for review_gate (default applied: Fix)"
    assert older_page["entries"][1]["kind"] == "log"
    assert older_page["entries"][1]["severity"] == "warning"
    assert older_page["entries"][1]["summary"] == "warn: disk almost full"


def test_pipeline_events_live_tail_without_after_sequence_does_not_replay_persisted_history(
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    run_id = "run-sse-live-tail"
    working_directory = tmp_path / "work"
    server.configure_runtime_paths(runs_dir=tmp_path / "runs")
    _write_persisted_events(
        run_id,
        working_directory,
        [
            {
                "type": "runtime",
                "status": "running",
                "run_id": run_id,
                "sequence": 1,
                "emitted_at": "2026-04-06T12:00:00Z",
                "source_scope": "root",
            },
        ],
    )

    hub = _QueueOnlyEventHub(
        run_id,
        queued_events=[
            {
                "type": "runtime",
                "status": "running",
                "run_id": run_id,
                "sequence": 2,
                "emitted_at": "2026-04-06T12:00:01Z",
                "source_scope": "root",
            },
        ],
    )
    monkeypatch.setattr(server, "EVENT_HUB", hub)

    events = asyncio.run(_collect_stream_events(run_id, count=1))

    assert [event["sequence"] for event in events] == [2]


def test_pipeline_events_gap_fill_after_requested_sequence_without_replaying_duplicate_live_sequence(
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    run_id = "run-sse-dedupe"
    working_directory = tmp_path / "work"
    server.configure_runtime_paths(runs_dir=tmp_path / "runs")
    _write_persisted_events(
        run_id,
        working_directory,
        [
            {
                "type": "runtime",
                "status": "running",
                "run_id": run_id,
                "sequence": 1,
                "emitted_at": "2026-04-06T12:00:00Z",
                "source_scope": "root",
            },
            {
                "type": "runtime",
                "status": "running",
                "run_id": run_id,
                "sequence": 2,
                "emitted_at": "2026-04-06T12:00:01Z",
                "source_scope": "root",
            },
        ],
    )

    hub = _QueueOnlyEventHub(
        run_id,
        queued_events=[
            {
                "type": "runtime",
                "status": "running",
                "run_id": run_id,
                "sequence": 2,
                "emitted_at": "2026-04-06T12:00:01Z",
                "source_scope": "root",
            },
            {
                "type": "runtime",
                "status": "running",
                "run_id": run_id,
                "sequence": 3,
                "emitted_at": "2026-04-06T12:00:02Z",
                "source_scope": "root",
            },
        ],
    )
    monkeypatch.setattr(server, "EVENT_HUB", hub)

    events = asyncio.run(_collect_stream_events(run_id, count=2, after_sequence=1))

    assert [event["sequence"] for event in events] == [2, 3]


def test_pipeline_events_do_not_drop_live_events_published_during_history_handoff(
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    run_id = "run-sse-handoff"
    server.configure_runtime_paths(runs_dir=tmp_path / "runs")

    hub = _SubscriberAwareEventHub(run_id)
    monkeypatch.setattr(server, "EVENT_HUB", hub)

    gap_event = {
        "type": "runtime",
        "status": "running",
        "run_id": run_id,
        "sequence": 1,
        "emitted_at": "2026-04-06T12:00:00Z",
        "source_scope": "root",
    }
    follow_up_event = {
        "type": "runtime",
        "status": "running",
        "run_id": run_id,
        "sequence": 2,
        "emitted_at": "2026-04-06T12:00:01Z",
        "source_scope": "root",
    }

    def _read_persisted_run_events_with_gap(pipeline_id: str) -> list[dict]:
        assert pipeline_id == run_id
        hub.inject(gap_event)
        return []

    monkeypatch.setattr(server, "_read_persisted_run_events", _read_persisted_run_events_with_gap)

    with server.ACTIVE_RUNS_LOCK:
        server.ACTIVE_RUNS[run_id] = server.ActiveRun(
            run_id=run_id,
            flow_name="Flow",
            working_directory=str(tmp_path / "work"),
            model="test-model",
            status="running",
        )

    try:
        async def _exercise_stream() -> list[dict]:
            response = await server.pipeline_events(run_id, _RequestNeverDisconnect(), after_sequence=0)
            await hub.publish(run_id, follow_up_event)
            return await _collect_stream_events_until_timeout(
                response,
                timeout_seconds=0.05,
                max_events=2,
            )

        events = asyncio.run(_exercise_stream())
    finally:
        server._pop_active_run(run_id)

    assert [event["sequence"] for event in events] == [1, 2]


def test_publish_run_event_persists_sequence_and_timestamp(tmp_path: Path) -> None:
    run_id = "run-sse-persist"
    working_directory = tmp_path / "work"
    server.configure_runtime_paths(runs_dir=tmp_path / "runs")
    server._ensure_run_root_for_project(run_id, str(working_directory))

    asyncio.run(server._publish_run_event(run_id, {"type": "StageStarted", "node_id": "prepare", "index": 1}))
    asyncio.run(server._publish_run_event(run_id, {"type": "StageCompleted", "node_id": "prepare", "index": 1}))

    events = server._read_persisted_run_events(run_id)

    assert [event["sequence"] for event in events] == [1, 2]
    assert all(event["run_id"] == run_id for event in events)
    assert all(isinstance(event["emitted_at"], str) and event["emitted_at"].endswith("Z") for event in events)


def test_pipeline_persists_first_class_child_stage_events_on_child_run(
    attractor_api_client: TestClient,
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    server.configure_runtime_paths(runs_dir=tmp_path / "runs")

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

    class _Backend:
        def run(  # type: ignore[no-untyped-def]
            self,
            node_id,
            prompt,
            context,
            *,
            response_contract="",
            contract_repair_attempts=0,
            timeout=None,
            model=None,
            write_contract=None,
        ):
            del node_id, prompt, context, response_contract, contract_repair_attempts, timeout, model, write_contract
            return "child ok"

        def run_with_events(  # type: ignore[no-untyped-def]
            self,
            node_id,
            prompt,
            context,
            emit_event=None,
            *,
            response_contract="",
            contract_repair_attempts=0,
            timeout=None,
            model=None,
            provider=None,
            reasoning_effort=None,
            write_contract=None,
        ):
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

    monkeypatch.setattr(
        server,
        "_build_codergen_backend",
        lambda backend_name, working_dir, emit, model=None: _Backend(),
    )

    start_payload = _start_pipeline(
        attractor_api_client,
        tmp_path / "work",
        flow_content=f"""
        digraph Parent {{
            graph [stack.child_dotfile="{child_dot_path}"]
            start [shape=Mdiamond]
            manager [shape=house, manager.poll_interval=0ms, manager.max_cycles=1, manager.actions=""]
            done [shape=Msquare]

            start -> manager -> done
        }}
        """,
    )
    run_id = str(start_payload["pipeline_id"])
    final_payload = _wait_for_pipeline_completion(attractor_api_client, run_id)

    assert final_payload["status"] == "completed"

    events = server._read_persisted_run_events(run_id)
    child_lifecycle_events = [
        event
        for event in events
        if event.get("type") in {"ChildRunStarted", "ChildRunCompleted"}
    ]
    assert [event["type"] for event in child_lifecycle_events] == ["ChildRunStarted", "ChildRunCompleted"]
    child_run_id = str(child_lifecycle_events[0]["child_run_id"])
    assert child_run_id != run_id

    child_events = server._read_persisted_run_events(child_run_id)
    child_stage_events = [
        event
        for event in child_events
        if event.get("source_scope") == "root" and event.get("type") in {"StageStarted", "StageCompleted"}
    ]

    assert [event["node_id"] for event in child_stage_events] == ["start", "start", "task", "task"]
    child_record = server._read_run_meta(server._run_meta_path(child_run_id))
    assert child_record is not None
    assert child_record.parent_run_id == run_id
    assert child_record.parent_node_id == "manager"


def test_pipeline_events_drop_oldest_events_under_sustained_throughput(
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    run_id = "run-sse-throughput"
    server.configure_runtime_paths(runs_dir=tmp_path / "runs")

    hub = server.PipelineEventHub()
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
        events, queue_maxsize, publish_count = asyncio.run(
            _collect_stream_events_for_throughput(run_id, hub)
        )
    finally:
        server._pop_active_run(run_id)

    assert len(events) == queue_maxsize
    assert events[0]["sequence"] == publish_count - queue_maxsize + 1
    assert events[-1]["sequence"] == publish_count
