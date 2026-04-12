import json
from pathlib import Path

import pytest

from attractor.dsl import parse_dot
from attractor.engine.context import Context
from attractor.engine.outcome import OutcomeStatus
from attractor.handlers import HandlerRunner, build_default_registry

from tests.handlers._support.fakes import _StubBackend


class _MilestoneRecordingBackend:
    def __init__(self):
        self.milestone_ids: list[str] = []

    def run(
        self,
        node_id: str,
        prompt: str,
        context: Context,
        *,
        response_contract: str = "",
        contract_repair_attempts: int = 0,
        timeout=None,
        model=None,
        write_contract=None,
    ) -> bool:
        del prompt, response_contract, contract_repair_attempts, timeout, model, write_contract
        if node_id == "task":
            self.milestone_ids.append(str(context.get("context.milestone.id", "")))
        return True


class TestManagerLoopHandler:
    def test_manager_loop_autostarts_child_pipeline_from_graph_attr(self, tmp_path):
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

        graph = parse_dot(
            f"""
            digraph G {{
                graph [stack.child_dotfile="{child_dot_path}"]
                manager [shape=house, manager.poll_interval=0ms, manager.max_cycles=1, manager.actions=""]
            }}
            """
        )
        backend = _StubBackend(ok=True)
        registry = build_default_registry(codergen_backend=backend)
        runner = HandlerRunner(graph, registry, logs_root=tmp_path / "logs")
        context = Context()

        outcome = runner("manager", "", context)

        assert outcome.status == OutcomeStatus.SUCCESS
        assert outcome.notes == "Child completed"
        assert [call[0] for call in backend.calls] == ["task"]
        assert context.get("context.stack.child.status") == "completed"
        assert context.get("context.stack.child.outcome") == "success"
        assert context.get("context.stack.child.active_stage") == "done"

    def test_manager_loop_forwards_child_events_into_parent_stream(self, tmp_path):
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

        graph = parse_dot(
            f"""
            digraph G {{
                graph [stack.child_dotfile="{child_dot_path}"]
                manager [shape=house, manager.poll_interval=0ms, manager.max_cycles=1, manager.actions=""]
            }}
            """
        )
        backend = _StubBackend(ok=True)
        registry = build_default_registry(codergen_backend=backend)
        runner = HandlerRunner(graph, registry, logs_root=tmp_path / "logs")
        captured_events: list[dict[str, object]] = []

        outcome = runner.run_with_events(
            "manager",
            "",
            Context(),
            lambda event_type, **payload: captured_events.append({"type": event_type, **payload}),
        )

        assert outcome is not None
        assert outcome.status == OutcomeStatus.SUCCESS
        event_types = [event["type"] for event in captured_events]
        assert event_types[0] == "PipelineStarted"
        assert event_types[-1] == "PipelineCompleted"
        assert event_types.count("StageStarted") == 2
        assert event_types.count("StageCompleted") == 2
        assert "CheckpointSaved" in event_types
        assert all(event.get("source_scope") == "child" for event in captured_events)
        assert all(event.get("source_parent_node_id") == "manager" for event in captured_events)
        assert all(event.get("source_flow_name") == "child.dot" for event in captured_events)

    def test_manager_loop_propagates_parent_cancel_control_to_child_executor(self, tmp_path):
        child_dot_path = tmp_path / "child.dot"
        child_dot_path.write_text(
            """
            digraph Child {
                start [shape=Mdiamond]
                task1 [shape=box, prompt="Child task 1"]
                task2 [shape=box, prompt="Child task 2"]
                done [shape=Msquare]

                start -> task1 -> task2 -> done
            }
            """,
            encoding="utf-8",
        )

        graph = parse_dot(
            f"""
            digraph G {{
                graph [stack.child_dotfile="{child_dot_path}"]
                manager [shape=house, manager.poll_interval=0ms, manager.max_cycles=1, manager.actions=""]
            }}
            """
        )
        backend = _StubBackend(ok=True)
        registry = build_default_registry(codergen_backend=backend)
        runner = HandlerRunner(graph, registry, logs_root=tmp_path / "logs")
        def control() -> str | None:
            if backend.calls:
                return "abort"
            return None

        runner.set_control(control)
        context = Context()

        outcome = runner("manager", "", context)

        assert outcome.status == OutcomeStatus.FAIL
        assert outcome.failure_reason == "aborted_by_user"
        assert [call[0] for call in backend.calls] == ["task1"]
        assert context.get("context.stack.child.status") == "aborted"
        assert context.get("context.stack.child.failure_reason") == "aborted_by_user"

    def test_manager_loop_autostarts_fresh_child_when_stale_completed_child_state_exists(self, tmp_path):
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

        graph = parse_dot(
            f"""
            digraph G {{
                graph [stack.child_dotfile="{child_dot_path}"]
                manager [shape=house, manager.poll_interval=0ms, manager.max_cycles=1, manager.actions=""]
            }}
            """
        )
        backend = _StubBackend(ok=True)
        registry = build_default_registry(codergen_backend=backend)
        runner = HandlerRunner(graph, registry, logs_root=tmp_path / "logs")
        context = Context(
            values={
                "context.stack.child.status": "completed",
                "context.stack.child.outcome": "success",
                "context.stack.child.outcome_reason_message": "stale success",
                "context.stack.child.active_stage": "old-stage",
                "context.stack.child.failure_reason": "old failure",
                "context.stack.child.route_trace": ["old"],
            }
        )

        outcome = runner("manager", "", context)

        assert outcome.status == OutcomeStatus.SUCCESS
        assert outcome.notes == "Child completed"
        assert [call[0] for call in backend.calls] == ["task"]
        assert context.get("context.stack.child.active_stage") == "done"
        assert context.get("context.stack.child.outcome_reason_message") == ""
        assert context.get("context.stack.child.failure_reason") == ""
        assert context.get("context.stack.child.route_trace") == ["start", "task", "done"]

    def test_manager_loop_autostarts_fresh_child_when_stale_failed_child_state_exists(self, tmp_path):
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

        graph = parse_dot(
            f"""
            digraph G {{
                graph [stack.child_dotfile="{child_dot_path}"]
                manager [shape=house, manager.poll_interval=0ms, manager.max_cycles=1, manager.actions=""]
            }}
            """
        )
        backend = _StubBackend(ok=True)
        registry = build_default_registry(codergen_backend=backend)
        runner = HandlerRunner(graph, registry, logs_root=tmp_path / "logs")
        context = Context(
            values={
                "context.stack.child.status": "failed",
                "context.stack.child.outcome": "failure",
                "context.stack.child.failure_reason": "stale failure",
            }
        )

        outcome = runner("manager", "", context)

        assert outcome.status == OutcomeStatus.SUCCESS
        assert [call[0] for call in backend.calls] == ["task"]
        assert context.get("context.stack.child.status") == "completed"
        assert context.get("context.stack.child.outcome") == "success"
        assert context.get("context.stack.child.failure_reason") == ""

    def test_manager_loop_applies_child_graph_transforms_before_execution(self, tmp_path):
        child_dot_path = tmp_path / "child.dot"
        child_dot_path.write_text(
            """
            digraph Child {
                graph [goal="Ship child"]
                start [shape=Mdiamond]
                task [shape=box, prompt="Plan for $goal"]
                done [shape=Msquare]

                start -> task -> done
            }
            """,
            encoding="utf-8",
        )

        graph = parse_dot(
            f"""
            digraph G {{
                graph [stack.child_dotfile="{child_dot_path}"]
                manager [shape=house, manager.poll_interval=0ms, manager.max_cycles=1, manager.actions=""]
            }}
            """
        )
        backend = _StubBackend(ok=True)
        registry = build_default_registry(codergen_backend=backend)
        runner = HandlerRunner(graph, registry, logs_root=tmp_path / "logs")

        outcome = runner("manager", "", Context())

        assert outcome.status == OutcomeStatus.SUCCESS
        assert len(backend.calls) == 1
        assert backend.calls[0][1].endswith("Current stage task:\n\nPlan for Ship child")

    def test_manager_loop_fails_when_child_graph_validation_fails(self, tmp_path):
        child_dot_path = tmp_path / "child.dot"
        child_dot_path.write_text(
            """
            digraph Child {
                start [shape=Mdiamond]
                task [shape=box, prompt="Child task"]
                start -> task
            }
            """,
            encoding="utf-8",
        )

        graph = parse_dot(
            f"""
            digraph G {{
                graph [stack.child_dotfile="{child_dot_path}"]
                manager [shape=house, manager.poll_interval=0ms, manager.max_cycles=1, manager.actions=""]
            }}
            """
        )
        registry = build_default_registry(codergen_backend=_StubBackend(ok=True))
        runner = HandlerRunner(graph, registry, logs_root=tmp_path / "logs")

        outcome = runner("manager", "", Context())

        assert outcome.status == OutcomeStatus.FAIL
        assert outcome.failure_reason is not None
        assert outcome.failure_reason.startswith("Child DOT graph failed validation:")

    def test_manager_loop_observe_action_writes_telemetry_artifacts(self, monkeypatch, tmp_path):
        def _fake_observe(context: Context, node_id: str, cycle: int) -> None:
            del node_id
            context.set("context.stack.child.status", f"running-{cycle}")
            context.set("context.stack.child.outcome", "")
            context.set("context.stack.child.active_stage", f"stage-{cycle}")
            context.set("context.stack.child.retry_count", cycle)

        monkeypatch.setattr("attractor.handlers.builtin.manager_loop._ingest_child_telemetry", _fake_observe)
        graph = parse_dot(
            """
            digraph G {
                manager [shape=house, manager.poll_interval=0ms, manager.max_cycles=2, manager.actions="observe"]
            }
            """
        )
        logs_root = tmp_path / "logs"
        registry = build_default_registry(codergen_backend=_StubBackend())
        runner = HandlerRunner(graph, registry, logs_root=logs_root)

        outcome = runner("manager", "", Context())

        assert outcome.status == OutcomeStatus.FAIL
        telemetry_path = logs_root / "manager" / "manager_telemetry.jsonl"
        assert telemetry_path.exists()
        lines = telemetry_path.read_text(encoding="utf-8").splitlines()
        payloads = [json.loads(line) for line in lines]
        assert [entry["cycle"] for entry in payloads] == [1, 2]
        assert [entry["node_id"] for entry in payloads] == ["manager", "manager"]
        assert [entry["child_status"] for entry in payloads] == ["running-1", "running-2"]
        assert [entry["child_active_stage"] for entry in payloads] == ["stage-1", "stage-2"]
        assert [entry["child_retry_count"] for entry in payloads] == [1, 2]

    def test_manager_loop_observe_and_steer_actions_skip_wait_when_wait_not_enabled(self, monkeypatch):
        observed = []
        steered = []
        sleep_calls = []

        def _fake_observe(context: Context, node_id: str, cycle: int) -> None:
            observed.append((node_id, cycle, dict(context.values)))

        def _fake_steer(context: Context, node_id: str, cycle: int) -> None:
            steered.append((node_id, cycle, dict(context.values)))

        def _fake_sleep(seconds: float) -> None:
            sleep_calls.append(seconds)

        monkeypatch.setattr("attractor.handlers.builtin.manager_loop._ingest_child_telemetry", _fake_observe)
        monkeypatch.setattr("attractor.handlers.builtin.manager_loop._steer_child", _fake_steer)
        monkeypatch.setattr("attractor.handlers.builtin.manager_loop.time.sleep", _fake_sleep)

        graph = parse_dot(
            """
            digraph G {
                manager [shape=house, manager.poll_interval=25ms, manager.max_cycles=2, manager.actions="observe,steer"]
            }
            """
        )
        registry = build_default_registry(codergen_backend=_StubBackend())
        runner = HandlerRunner(graph, registry)
        outcome = runner("manager", "", Context(values={"context.stack.child.status": "running"}))

        assert outcome.status == OutcomeStatus.FAIL
        assert outcome.failure_reason == "Max cycles exceeded"
        assert [entry[:2] for entry in observed] == [("manager", 1), ("manager", 2)]
        assert [entry[:2] for entry in steered] == [("manager", 1), ("manager", 2)]
        assert sleep_calls == []

    def test_manager_loop_returns_fail_when_child_status_is_failed(self, monkeypatch):
        def _fake_sleep(seconds: float) -> None:
            raise AssertionError(f"unexpected wait call: {seconds}")

        monkeypatch.setattr("attractor.handlers.builtin.manager_loop.time.sleep", _fake_sleep)
        graph = parse_dot(
            """
            digraph G {
                manager [
                    shape=house,
                    manager.poll_interval=25ms,
                    manager.max_cycles=5,
                    manager.actions="wait"
                ]
            }
            """
        )
        registry = build_default_registry(codergen_backend=_StubBackend())
        runner = HandlerRunner(graph, registry)
        context = Context(
            values={
                "context.stack.child.status": "failed",
                "context.stack.child.outcome": "failure",
            }
        )

        outcome = runner("manager", "", context)

        assert outcome.status == OutcomeStatus.FAIL
        assert outcome.failure_reason == "Child failed"

    def test_manager_loop_does_not_autostart_duplicate_child_when_existing_child_is_running(self, tmp_path):
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

        graph = parse_dot(
            f"""
            digraph G {{
                graph [stack.child_dotfile="{child_dot_path}"]
                manager [shape=house, manager.poll_interval=0ms, manager.max_cycles=1, manager.actions=""]
            }}
            """
        )
        backend = _StubBackend(ok=True)
        registry = build_default_registry(codergen_backend=backend)
        runner = HandlerRunner(graph, registry, logs_root=tmp_path / "logs")
        context = Context(values={"context.stack.child.status": "running"})

        outcome = runner("manager", "", context)

        assert outcome.status == OutcomeStatus.FAIL
        assert outcome.failure_reason == "Max cycles exceeded"
        assert backend.calls == []

    def test_manager_loop_returns_success_when_child_is_completed_with_success(self, monkeypatch):
        def _fake_sleep(seconds: float) -> None:
            raise AssertionError(f"unexpected wait call: {seconds}")

        monkeypatch.setattr("attractor.handlers.builtin.manager_loop.time.sleep", _fake_sleep)
        graph = parse_dot(
            """
            digraph G {
                manager [
                    shape=house,
                    manager.poll_interval=25ms,
                    manager.max_cycles=5,
                    manager.actions="wait"
                ]
            }
            """
        )
        registry = build_default_registry(codergen_backend=_StubBackend())
        runner = HandlerRunner(graph, registry)
        context = Context(
            values={
                "context.stack.child.status": "completed",
                "context.stack.child.outcome": "success",
            }
        )

        outcome = runner("manager", "", context)

        assert outcome.status == OutcomeStatus.SUCCESS
        assert outcome.notes == "Child completed"

    def test_manager_loop_non_autostart_resolves_prepopulated_terminal_child_state(self, tmp_path):
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

        graph = parse_dot(
            f"""
            digraph G {{
                graph [stack.child_dotfile="{child_dot_path}"]
                manager [
                    shape=house,
                    stack.child_autostart=false,
                    manager.poll_interval=0ms,
                    manager.max_cycles=5,
                    manager.actions="wait"
                ]
            }}
            """
        )
        backend = _StubBackend(ok=True)
        registry = build_default_registry(codergen_backend=backend)
        runner = HandlerRunner(graph, registry)
        context = Context(
            values={
                "context.stack.child.status": "completed",
                "context.stack.child.outcome": "success",
            }
        )

        outcome = runner("manager", "", context)

        assert outcome.status == OutcomeStatus.SUCCESS
        assert outcome.notes == "Child completed"
        assert backend.calls == []

    def test_manager_loop_returns_fail_when_child_completes_with_failure_outcome(self, monkeypatch):
        def _fake_sleep(seconds: float) -> None:
            raise AssertionError(f"unexpected wait call: {seconds}")

        monkeypatch.setattr("attractor.handlers.builtin.manager_loop.time.sleep", _fake_sleep)
        graph = parse_dot(
            """
            digraph G {
                manager [
                    shape=house,
                    manager.poll_interval=25ms,
                    manager.max_cycles=5,
                    manager.actions="wait"
                ]
            }
            """
        )
        registry = build_default_registry(codergen_backend=_StubBackend())
        runner = HandlerRunner(graph, registry)
        context = Context(
            values={
                "context.stack.child.status": "completed",
                "context.stack.child.outcome": "failure",
                "context.stack.child.outcome_reason_message": "blocked on human approval",
            }
        )

        outcome = runner("manager", "", context)

        assert outcome.status == OutcomeStatus.FAIL
        assert outcome.failure_reason == "blocked on human approval"

    def test_manager_loop_runs_autostarted_child_pipeline_from_stack_child_workdir(self, tmp_path):
        child_workdir = tmp_path / "child-workdir"
        child_workdir.mkdir(parents=True, exist_ok=True)
        child_dot_path = child_workdir / "child.dot"
        child_dot_path.write_text(
            """
            digraph Child {
                start [shape=Mdiamond]
                task [shape=parallelogram, tool.command="pwd"]
                done [shape=Msquare]

                start -> task -> done
            }
            """,
            encoding="utf-8",
        )

        graph = parse_dot(
            f"""
            digraph G {{
                graph [stack.child_dotfile="child.dot", stack.child_workdir="{child_workdir}"]
                manager [shape=house, manager.poll_interval=0ms, manager.max_cycles=1, manager.actions=""]
            }}
            """
        )
        registry = build_default_registry(codergen_backend=_StubBackend(ok=True))
        logs_root = tmp_path / "logs"
        runner = HandlerRunner(graph, registry, logs_root=logs_root)

        outcome = runner("manager", "", Context())

        assert outcome.status == OutcomeStatus.SUCCESS
        child_tool_output = (logs_root / "manager" / "child" / "task" / "tool_output.txt").read_text(
            encoding="utf-8"
        )
        assert Path(child_tool_output.strip()) == child_workdir

    def test_manager_loop_resolves_relative_child_dotfile_from_flow_source_dir_and_runs_in_parent_workdir(
        self, tmp_path
    ):
        flow_source_dir = tmp_path / "flows"
        flow_source_dir.mkdir(parents=True, exist_ok=True)
        child_dot_path = flow_source_dir / "child.dot"
        child_dot_path.write_text(
            """
            digraph Child {
                start [shape=Mdiamond]
                task [shape=parallelogram, tool.command="pwd"]
                done [shape=Msquare]

                start -> task -> done
            }
            """,
            encoding="utf-8",
        )
        run_workdir = tmp_path / "project"
        run_workdir.mkdir(parents=True, exist_ok=True)

        graph = parse_dot(
            """
            digraph G {
                graph [stack.child_dotfile="child.dot"]
                manager [shape=house, manager.poll_interval=0ms, manager.max_cycles=1, manager.actions=""]
            }
            """
        )
        registry = build_default_registry(codergen_backend=_StubBackend(ok=True))
        logs_root = tmp_path / "logs"
        runner = HandlerRunner(graph, registry, logs_root=logs_root)
        context = Context(
            values={
                "internal.flow_source_dir": str(flow_source_dir),
                "internal.run_workdir": str(run_workdir),
            }
        )

        outcome = runner("manager", "", context)

        assert outcome.status == OutcomeStatus.SUCCESS
        child_tool_output = (logs_root / "manager" / "child" / "task" / "tool_output.txt").read_text(
            encoding="utf-8"
        )
        assert Path(child_tool_output.strip()) == run_workdir

    def test_manager_loop_steer_action_honors_cooldown(self, monkeypatch):
        steered = []
        clock = iter([0.0, 1.0, 2.0])

        def _fake_steer(context: Context, node_id: str, cycle: int) -> None:
            steered.append((node_id, cycle, dict(context.values)))

        monkeypatch.setattr("attractor.handlers.builtin.manager_loop._steer_child", _fake_steer)
        monkeypatch.setattr("attractor.handlers.builtin.manager_loop.time.monotonic", lambda: next(clock))
        graph = parse_dot(
            """
            digraph G {
                manager [
                    shape=house,
                    manager.poll_interval=0ms,
                    manager.max_cycles=3,
                    manager.actions="steer",
                    manager.steer_cooldown=2s
                ]
            }
            """
        )
        registry = build_default_registry(codergen_backend=_StubBackend())
        runner = HandlerRunner(graph, registry)

        outcome = runner("manager", "", Context())

        assert outcome.status == OutcomeStatus.FAIL
        assert outcome.failure_reason == "Max cycles exceeded"
        assert [entry[:2] for entry in steered] == [("manager", 1), ("manager", 3)]

    def test_manager_loop_steer_action_writes_intervention_artifacts(self, monkeypatch, tmp_path):
        def _fake_steer(context: Context, node_id: str, cycle: int) -> None:
            del node_id
            context.set("context.stack.child.active_stage", f"active-{cycle}")
            context.set("context.stack.child.intervention", f"instruction-{cycle}")
            context.set("context.stack.child.status", "running")
            context.set("context.stack.child.outcome", "")

        monkeypatch.setattr("attractor.handlers.builtin.manager_loop._steer_child", _fake_steer)
        graph = parse_dot(
            """
            digraph G {
                manager [shape=house, manager.poll_interval=0ms, manager.max_cycles=2, manager.actions="steer"]
            }
            """
        )
        logs_root = tmp_path / "logs"
        registry = build_default_registry(codergen_backend=_StubBackend())
        runner = HandlerRunner(graph, registry, logs_root=logs_root)

        outcome = runner("manager", "", Context())

        assert outcome.status == OutcomeStatus.FAIL
        interventions_path = logs_root / "manager" / "manager_interventions.jsonl"
        assert interventions_path.exists()
        lines = interventions_path.read_text(encoding="utf-8").splitlines()
        payloads = [json.loads(line) for line in lines]
        assert [entry["cycle"] for entry in payloads] == [1, 2]
        assert [entry["node_id"] for entry in payloads] == ["manager", "manager"]
        assert [entry["child_active_stage"] for entry in payloads] == ["active-1", "active-2"]
        assert [entry["instruction"] for entry in payloads] == ["instruction-1", "instruction-2"]

    def test_manager_loop_stop_condition_returns_success_when_satisfied(self, monkeypatch):
        def _fake_observe(context: Context, node_id: str, cycle: int) -> None:
            del node_id, cycle
            context.set("context.stack.child.ready", True)

        def _fake_sleep(seconds: float) -> None:
            raise AssertionError(f"unexpected wait call: {seconds}")

        monkeypatch.setattr("attractor.handlers.builtin.manager_loop._ingest_child_telemetry", _fake_observe)
        monkeypatch.setattr("attractor.handlers.builtin.manager_loop.time.sleep", _fake_sleep)
        graph = parse_dot(
            """
            digraph G {
                manager [
                    shape=house,
                    manager.poll_interval=25ms,
                    manager.max_cycles=5,
                    manager.actions="observe,wait",
                    manager.stop_condition="context.stack.child.ready=true"
                ]
            }
            """
        )
        registry = build_default_registry(codergen_backend=_StubBackend())
        runner = HandlerRunner(graph, registry)

        outcome = runner("manager", "", Context())

        assert outcome.status == OutcomeStatus.SUCCESS
        assert outcome.notes == "Stop condition satisfied"

    def test_manager_loop_uses_configured_poll_interval_and_max_cycles(self, monkeypatch):
        sleep_calls = []

        def _fake_sleep(seconds: float) -> None:
            sleep_calls.append(seconds)

        monkeypatch.setattr("attractor.handlers.builtin.manager_loop.time.sleep", _fake_sleep)
        graph = parse_dot(
            """
            digraph G {
                manager [shape=house, manager.poll_interval=25ms, manager.max_cycles=3]
            }
            """
        )
        registry = build_default_registry(codergen_backend=_StubBackend())
        runner = HandlerRunner(graph, registry)

        outcome = runner("manager", "", Context())

        assert outcome.status == OutcomeStatus.FAIL
        assert outcome.failure_reason == "Max cycles exceeded"
        assert sleep_calls == pytest.approx([0.025, 0.025, 0.025])

    def test_manager_loop_revisiting_same_node_autostarts_new_child_with_updated_milestone_context(self, tmp_path):
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

        graph = parse_dot(
            f"""
            digraph G {{
                graph [stack.child_dotfile="{child_dot_path}"]
                manager [shape=house, manager.poll_interval=0ms, manager.max_cycles=1, manager.actions=""]
            }}
            """
        )
        backend = _MilestoneRecordingBackend()
        registry = build_default_registry(codergen_backend=backend)
        runner = HandlerRunner(graph, registry, logs_root=tmp_path / "logs")
        context = Context(values={"context.milestone.id": "M-ONE"})

        first = runner("manager", "", context)
        context.set("context.milestone.id", "M-TWO")
        second = runner("manager", "", context)

        assert first.status == OutcomeStatus.SUCCESS
        assert second.status == OutcomeStatus.SUCCESS
        assert backend.milestone_ids == ["M-ONE", "M-TWO"]
