import json

import pytest

from attractor.dsl import parse_dot
from attractor.engine.context import Context
from attractor.engine.outcome import OutcomeStatus
from attractor.handlers.base import ChildInterventionRequest, ChildInterventionResult, ChildRunRequest, ChildRunResult
from attractor.handlers import HandlerRunner, build_default_registry

from tests.handlers._support.fakes import _StubBackend


def _completed_child_result(request: ChildRunRequest) -> ChildRunResult:
    return ChildRunResult(
        run_id=request.child_run_id,
        status="completed",
        outcome="success",
        current_node="done",
        completed_nodes=["start", "task", "done"],
        route_trace=["start", "task", "done"],
    )


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
        launched_requests: list[ChildRunRequest] = []

        def launch_child(request: ChildRunRequest) -> ChildRunResult:
            launched_requests.append(request)
            return _completed_child_result(request)

        registry = build_default_registry(codergen_backend=_StubBackend(ok=True))
        runner = HandlerRunner(
            graph,
            registry,
            logs_root=tmp_path / "logs",
            child_run_launcher=launch_child,
        )
        context = Context()

        outcome = runner("manager", "", context)

        assert outcome.status == OutcomeStatus.SUCCESS
        assert outcome.notes == "Child completed"
        assert len(launched_requests) == 1
        assert launched_requests[0].child_flow_path == child_dot_path
        assert launched_requests[0].child_flow_name == "child.dot"
        assert launched_requests[0].parent_node_id == "manager"
        assert context.get("context.stack.child.status") == "completed"
        assert context.get("context.stack.child.outcome") == "success"
        assert context.get("context.stack.child.active_stage") == "done"

    def test_manager_loop_emits_first_class_child_lifecycle_events(self, tmp_path):
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
        registry = build_default_registry(codergen_backend=_StubBackend(ok=True))
        runner = HandlerRunner(
            graph,
            registry,
            logs_root=tmp_path / "logs",
            child_run_launcher=_completed_child_result,
        )
        captured_events: list[dict[str, object]] = []

        outcome = runner(
            "manager",
            "",
            Context(),
            emit_event=lambda event_type, **payload: captured_events.append({"type": event_type, **payload}),
        )

        assert outcome is not None
        assert outcome.status == OutcomeStatus.SUCCESS
        event_types = [event["type"] for event in captured_events]
        assert event_types == ["ChildRunStarted", "ChildRunCompleted"]
        assert captured_events[0]["parent_node_id"] == "manager"
        assert captured_events[0]["child_flow_name"] == "child.dot"
        assert captured_events[1]["status"] == "completed"

    def test_manager_loop_propagates_parent_cancel_control_to_child_launcher(self, tmp_path):
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
        control_calls = 0

        def control() -> str | None:
            return "abort"

        def launch_child(request: ChildRunRequest) -> ChildRunResult:
            nonlocal control_calls
            if request.control is not None:
                control_calls += 1
                assert request.control() == "abort"
            return ChildRunResult(
                run_id=request.child_run_id,
                status="aborted",
                failure_reason="aborted_by_user",
            )

        registry = build_default_registry(codergen_backend=_StubBackend(ok=True))
        runner = HandlerRunner(
            graph,
            registry,
            logs_root=tmp_path / "logs",
            child_run_launcher=launch_child,
        )
        runner.set_control(control)
        context = Context()

        outcome = runner("manager", "", context)

        assert outcome.status == OutcomeStatus.FAIL
        assert outcome.failure_reason == "aborted_by_user"
        assert control_calls == 1
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
        registry = build_default_registry(codergen_backend=_StubBackend(ok=True))
        runner = HandlerRunner(
            graph,
            registry,
            logs_root=tmp_path / "logs",
            child_run_launcher=_completed_child_result,
        )
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
        registry = build_default_registry(codergen_backend=_StubBackend(ok=True))
        runner = HandlerRunner(
            graph,
            registry,
            logs_root=tmp_path / "logs",
            child_run_launcher=_completed_child_result,
        )
        context = Context(
            values={
                "context.stack.child.status": "failed",
                "context.stack.child.outcome": "failure",
                "context.stack.child.failure_reason": "stale failure",
            }
        )

        outcome = runner("manager", "", context)

        assert outcome.status == OutcomeStatus.SUCCESS
        assert context.get("context.stack.child.status") == "completed"
        assert context.get("context.stack.child.outcome") == "success"
        assert context.get("context.stack.child.failure_reason") == ""

    def test_manager_loop_applies_child_graph_transforms_before_launch(self, tmp_path):
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
        launched_requests: list[ChildRunRequest] = []

        def launch_child(request: ChildRunRequest) -> ChildRunResult:
            launched_requests.append(request)
            return _completed_child_result(request)

        registry = build_default_registry(codergen_backend=_StubBackend(ok=True))
        runner = HandlerRunner(
            graph,
            registry,
            logs_root=tmp_path / "logs",
            child_run_launcher=launch_child,
        )

        outcome = runner("manager", "", Context())

        assert outcome.status == OutcomeStatus.SUCCESS
        assert len(launched_requests) == 1
        assert launched_requests[0].child_graph.nodes["task"].attrs["prompt"].value == "Plan for Ship child"

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

    def test_manager_loop_autostart_without_child_launcher_is_wiring_error(self, tmp_path):
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
        registry = build_default_registry(codergen_backend=_StubBackend(ok=True))
        runner = HandlerRunner(graph, registry, logs_root=tmp_path / "logs")

        with pytest.raises(AssertionError):
            runner("manager", "", Context())

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

    def test_manager_loop_observe_ingests_child_status_resolver_telemetry_without_steering(self, tmp_path):
        def resolve_child(run_id: str) -> ChildRunResult:
            return ChildRunResult(
                run_id=run_id,
                status="running",
                current_node="work",
                completed_nodes=["start"],
                route_trace=["start", "work"],
                retry_count=2,
                retry_counts={"work": 2},
                artifact_count=0,
                event_count=7,
                checkpoint_timestamp="2026-06-23T10:00:00Z",
                latest_event_at="2026-06-23T10:00:05Z",
                started_at="2026-06-23T09:59:00Z",
            )

        def request_intervention(request: ChildInterventionRequest) -> ChildInterventionResult:
            raise AssertionError(f"unexpected intervention request: {request}")

        graph = parse_dot(
            """
            digraph G {
                manager [
                    shape=house,
                    manager.poll_interval=0ms,
                    manager.max_cycles=1,
                    manager.actions="observe,steer",
                    manager.stop_condition="context.stack.child.event_count=7"
                ]
            }
            """
        )
        logs_root = tmp_path / "logs"
        registry = build_default_registry(codergen_backend=_StubBackend())
        runner = HandlerRunner(
            graph,
            registry,
            logs_root=logs_root,
            child_status_resolver=resolve_child,
            child_intervention_requester=request_intervention,
        )
        context = Context(values={"context.stack.child.run_id": "child-progress"})

        outcome = runner("manager", "", context)

        assert outcome.status == OutcomeStatus.SUCCESS
        assert outcome.notes == "Stop condition satisfied"
        assert context.get("context.stack.child.status") == "running"
        assert context.get("context.stack.child.active_stage") == "work"
        assert context.get("context.stack.child.retry_count") == 2
        assert context.get("context.stack.child.retry_counts") == {"work": 2}
        assert context.get("context.stack.child.artifact_count") == 0
        assert context.get("context.stack.child.event_count") == 7
        assert context.get("context.stack.child.checkpoint_timestamp") == "2026-06-23T10:00:00Z"
        assert context.get("context.stack.child.latest_event_at") == "2026-06-23T10:00:05Z"
        assert (logs_root / "manager" / "manager_telemetry.jsonl").exists()
        assert not (logs_root / "manager" / "manager_interventions.jsonl").exists()

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
        outcome = runner(
            "manager",
            "",
            Context(
                values={
                    "context.stack.child.run_id": "child-1",
                    "context.stack.child.status": "running",
                    "context.stack.child.failure_reason": "tests failed",
                }
            ),
        )

        assert outcome.status == OutcomeStatus.FAIL
        assert outcome.failure_reason == "Max cycles exceeded"
        assert [entry[:2] for entry in observed] == [("manager", 1), ("manager", 2)]
        assert [entry[:2] for entry in steered] == [("manager", 1)]
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

    def test_manager_loop_launches_autostarted_child_pipeline_from_stack_child_workdir(self, tmp_path):
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
        launched_requests: list[ChildRunRequest] = []

        def launch_child(request: ChildRunRequest) -> ChildRunResult:
            launched_requests.append(request)
            return _completed_child_result(request)

        registry = build_default_registry(codergen_backend=_StubBackend(ok=True))
        logs_root = tmp_path / "logs"
        runner = HandlerRunner(graph, registry, logs_root=logs_root, child_run_launcher=launch_child)

        outcome = runner("manager", "", Context())

        assert outcome.status == OutcomeStatus.SUCCESS
        assert len(launched_requests) == 1
        assert launched_requests[0].child_flow_path == child_dot_path
        assert launched_requests[0].child_workdir == child_workdir

    def test_manager_loop_resolves_relative_child_dotfile_from_flow_source_dir_and_launches_in_parent_workdir(
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
        launched_requests: list[ChildRunRequest] = []

        def launch_child(request: ChildRunRequest) -> ChildRunResult:
            launched_requests.append(request)
            return _completed_child_result(request)

        registry = build_default_registry(codergen_backend=_StubBackend(ok=True))
        logs_root = tmp_path / "logs"
        runner = HandlerRunner(graph, registry, logs_root=logs_root, child_run_launcher=launch_child)
        context = Context(
            values={
                "internal.flow_source_dir": str(flow_source_dir),
                "internal.run_workdir": str(run_workdir),
            }
        )

        outcome = runner("manager", "", context)

        assert outcome.status == OutcomeStatus.SUCCESS
        assert len(launched_requests) == 1
        assert launched_requests[0].child_flow_path == child_dot_path
        assert launched_requests[0].child_workdir == run_workdir

    def test_manager_loop_steer_action_honors_cooldown(self, monkeypatch):
        requests: list[ChildInterventionRequest] = []
        clock = iter([0.0, 1.0, 2.0])

        def _fake_observe(context: Context, node_id: str, cycle: int) -> None:
            del node_id
            context.set("context.stack.child.failure_reason", f"failure-{cycle}")

        def request_intervention(request: ChildInterventionRequest) -> ChildInterventionResult:
            requests.append(request)
            return ChildInterventionResult(
                run_id=request.child_run_id,
                status="delivered",
                delivery_mode="test",
                reason=request.reason,
                message="queued",
                target_node_id=request.target_node_id,
            )

        monkeypatch.setattr("attractor.handlers.builtin.manager_loop._ingest_child_telemetry", _fake_observe)
        monkeypatch.setattr("attractor.handlers.builtin.manager_loop.time.monotonic", lambda: next(clock))
        graph = parse_dot(
            """
            digraph G {
                manager [
                    shape=house,
                    manager.poll_interval=0ms,
                    manager.max_cycles=3,
                    manager.actions="observe,steer",
                    manager.steer_cooldown=2s
                ]
            }
            """
        )
        registry = build_default_registry(codergen_backend=_StubBackend())
        runner = HandlerRunner(graph, registry, child_intervention_requester=request_intervention)

        outcome = runner(
            "manager",
            "",
            Context(
                values={
                    "context.stack.child.run_id": "child-1",
                    "context.stack.child.status": "running",
                    "context.stack.child.active_stage": "task",
                }
            ),
        )

        assert outcome.status == OutcomeStatus.FAIL
        assert outcome.failure_reason == "Max cycles exceeded"
        assert [request.cycle for request in requests] == [1, 3]
        assert [request.reason for request in requests] == ["failure-1", "failure-3"]

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

        outcome = runner(
            "manager",
            "",
            Context(
                values={
                    "context.stack.child.run_id": "child-1",
                    "context.stack.child.status": "running",
                    "context.stack.child.failure_reason": "tests failed",
                }
            ),
        )

        assert outcome.status == OutcomeStatus.FAIL
        interventions_path = logs_root / "manager" / "manager_interventions.jsonl"
        assert interventions_path.exists()
        lines = interventions_path.read_text(encoding="utf-8").splitlines()
        payloads = [json.loads(line) for line in lines]
        assert [entry["cycle"] for entry in payloads] == [1, 2]
        assert [entry["node_id"] for entry in payloads] == ["manager", "manager"]
        assert [entry["child_active_stage"] for entry in payloads] == ["active-1", "active-2"]
        assert [entry["instruction"] for entry in payloads] == ["instruction-1", "instruction-2"]
        assert [entry["intervention_status"] for entry in payloads] == ["rejected", "rejected"]
        assert [entry["intervention_reason"] for entry in payloads] == [
            "backend_steering_unsupported",
            "backend_steering_unsupported",
        ]
        assert [entry["child_run_id"] for entry in payloads] == ["child-1", "child-1"]
        assert [entry["failure_reason"] for entry in payloads] == ["tests failed", "tests failed"]

    def test_manager_loop_requests_child_intervention_and_records_delivered_result(self, tmp_path):
        graph = parse_dot(
            """
            digraph G {
                manager [shape=house, manager.poll_interval=0ms, manager.max_cycles=1, manager.actions="steer"]
            }
            """
        )
        requests: list[ChildInterventionRequest] = []

        def request_intervention(request: ChildInterventionRequest) -> ChildInterventionResult:
            requests.append(request)
            return ChildInterventionResult(
                run_id=request.child_run_id,
                status="delivered",
                delivery_mode="unified_agent_session",
                reason=request.reason,
                message="queued",
                target_node_id=request.target_node_id,
            )

        registry = build_default_registry(codergen_backend=_StubBackend())
        runner = HandlerRunner(
            graph,
            registry,
            logs_root=tmp_path / "logs",
            child_intervention_requester=request_intervention,
        )
        events: list[dict[str, object]] = []
        context = Context(
            values={
                "internal.run_id": "parent-1",
                "internal.root_run_id": "root-1",
                "context.stack.child.run_id": "child-1",
                "context.stack.child.status": "running",
                "context.stack.child.active_stage": "task",
                "context.stack.child.failure_reason": "unit tests failed",
            }
        )

        outcome = runner(
            "manager",
            "",
            context,
            emit_event=lambda event_type, **payload: events.append({"type": event_type, **payload}),
        )

        assert outcome.status == OutcomeStatus.FAIL
        assert len(requests) == 1
        assert requests[0].child_run_id == "child-1"
        assert requests[0].parent_run_id == "parent-1"
        assert requests[0].parent_node_id == "manager"
        assert requests[0].root_run_id == "root-1"
        assert requests[0].target_node_id == "task"
        assert requests[0].reason == "unit tests failed"
        assert requests[0].source == "manager_loop"
        assert "unit tests failed" in requests[0].message
        assert context.get("context.stack.child.intervention_status") == "delivered"
        assert context.get("context.stack.child.intervention_delivery_mode") == "unified_agent_session"
        assert context.get("context.stack.child.intervention_reason") == "unit tests failed"
        assert events == [
            {
                "type": "ChildInterventionRequested",
                "child_run_id": "child-1",
                "parent_run_id": "parent-1",
                "parent_node_id": "manager",
                "root_run_id": "root-1",
                "target_node_id": "task",
                "status": "delivered",
                "delivery_mode": "unified_agent_session",
                "reason": "unit tests failed",
                "child_failure_reason": "unit tests failed",
                "message": "queued",
            }
        ]

    def test_manager_loop_suppresses_repeated_automatic_intervention_for_same_failure_point(self, tmp_path):
        graph = parse_dot(
            """
            digraph G {
                manager [shape=house, manager.poll_interval=0ms, manager.max_cycles=2, manager.actions="steer"]
            }
            """
        )
        requests: list[ChildInterventionRequest] = []

        def request_intervention(request: ChildInterventionRequest) -> ChildInterventionResult:
            requests.append(request)
            return ChildInterventionResult(
                run_id=request.child_run_id,
                status="delivered",
                delivery_mode="test",
                reason=request.reason,
                message="queued",
                target_node_id=request.target_node_id,
            )

        registry = build_default_registry(codergen_backend=_StubBackend())
        runner = HandlerRunner(
            graph,
            registry,
            logs_root=tmp_path / "logs",
            child_intervention_requester=request_intervention,
        )
        events: list[dict[str, object]] = []
        context = Context(
            values={
                "context.stack.child.run_id": "child-1",
                "context.stack.child.status": "running",
                "context.stack.child.active_stage": "task",
                "context.stack.child.failure_reason": "unit tests failed",
            }
        )

        outcome = runner(
            "manager",
            "",
            context,
            emit_event=lambda event_type, **payload: events.append({"type": event_type, **payload}),
        )

        assert outcome.status == OutcomeStatus.FAIL
        assert len(requests) == 1
        assert requests[0].cycle == 1
        assert context.get("context.stack.child.intervention_status") == "skipped"
        assert context.get("context.stack.child.intervention_reason") == "auto_steer_limit_reached"
        assert [event["status"] for event in events] == ["delivered", "skipped"]
        assert events[1]["child_run_id"] == "child-1"
        assert events[1]["target_node_id"] == "task"
        assert events[1]["reason"] == "auto_steer_limit_reached"
        assert events[1]["child_failure_reason"] == "unit tests failed"
        interventions_path = tmp_path / "logs" / "manager" / "manager_interventions.jsonl"
        payloads = [json.loads(line) for line in interventions_path.read_text(encoding="utf-8").splitlines()]
        assert [entry["intervention_status"] for entry in payloads] == ["delivered", "skipped"]
        assert payloads[1]["child_run_id"] == "child-1"
        assert payloads[1]["target_node_id"] == "task"
        assert payloads[1]["failure_reason"] == "unit tests failed"

    def test_manager_loop_allows_automatic_intervention_for_different_target_node(self, monkeypatch):
        def _fake_observe(context: Context, node_id: str, cycle: int) -> None:
            del node_id
            context.set("context.stack.child.active_stage", f"task-{cycle}")

        monkeypatch.setattr("attractor.handlers.builtin.manager_loop._ingest_child_telemetry", _fake_observe)
        graph = parse_dot(
            """
            digraph G {
                manager [shape=house, manager.poll_interval=0ms, manager.max_cycles=2, manager.actions="observe,steer"]
            }
            """
        )
        requests: list[ChildInterventionRequest] = []

        def request_intervention(request: ChildInterventionRequest) -> ChildInterventionResult:
            requests.append(request)
            return ChildInterventionResult(
                run_id=request.child_run_id,
                status="delivered",
                delivery_mode="test",
                reason=request.reason,
                message="queued",
                target_node_id=request.target_node_id,
            )

        registry = build_default_registry(codergen_backend=_StubBackend())
        runner = HandlerRunner(graph, registry, child_intervention_requester=request_intervention)

        outcome = runner(
            "manager",
            "",
            Context(
                values={
                    "context.stack.child.run_id": "child-1",
                    "context.stack.child.status": "running",
                    "context.stack.child.failure_reason": "same failure",
                }
            ),
        )

        assert outcome.status == OutcomeStatus.FAIL
        assert [request.target_node_id for request in requests] == ["task-1", "task-2"]

    def test_manager_loop_allows_automatic_intervention_for_different_failure_reason(self, monkeypatch):
        def _fake_observe(context: Context, node_id: str, cycle: int) -> None:
            del node_id
            context.set("context.stack.child.failure_reason", f"failure-{cycle}")

        monkeypatch.setattr("attractor.handlers.builtin.manager_loop._ingest_child_telemetry", _fake_observe)
        graph = parse_dot(
            """
            digraph G {
                manager [shape=house, manager.poll_interval=0ms, manager.max_cycles=2, manager.actions="observe,steer"]
            }
            """
        )
        requests: list[ChildInterventionRequest] = []

        def request_intervention(request: ChildInterventionRequest) -> ChildInterventionResult:
            requests.append(request)
            return ChildInterventionResult(
                run_id=request.child_run_id,
                status="delivered",
                delivery_mode="test",
                reason=request.reason,
                message="queued",
                target_node_id=request.target_node_id,
            )

        registry = build_default_registry(codergen_backend=_StubBackend())
        runner = HandlerRunner(graph, registry, child_intervention_requester=request_intervention)

        outcome = runner(
            "manager",
            "",
            Context(
                values={
                    "context.stack.child.run_id": "child-1",
                    "context.stack.child.status": "running",
                    "context.stack.child.active_stage": "task",
                }
            ),
        )

        assert outcome.status == OutcomeStatus.FAIL
        assert [request.reason for request in requests] == ["failure-1", "failure-2"]

    def test_manager_loop_steer_action_waits_for_failure_context(self, tmp_path):
        graph = parse_dot(
            """
            digraph G {
                manager [shape=house, manager.poll_interval=0ms, manager.max_cycles=1, manager.actions="steer"]
            }
            """
        )

        def request_intervention(request: ChildInterventionRequest) -> ChildInterventionResult:
            raise AssertionError(f"unexpected intervention request: {request}")

        registry = build_default_registry(codergen_backend=_StubBackend())
        runner = HandlerRunner(
            graph,
            registry,
            logs_root=tmp_path / "logs",
            child_intervention_requester=request_intervention,
        )

        outcome = runner(
            "manager",
            "",
            Context(
                values={
                    "context.stack.child.run_id": "child-1",
                    "context.stack.child.status": "running",
                }
            ),
        )

        assert outcome.status == OutcomeStatus.FAIL
        assert not (tmp_path / "logs" / "manager" / "manager_interventions.jsonl").exists()

    def test_manager_loop_records_rejected_intervention_when_failure_has_no_child_run(self):
        graph = parse_dot(
            """
            digraph G {
                manager [shape=house, manager.poll_interval=0ms, manager.max_cycles=1, manager.actions="steer"]
            }
            """
        )
        registry = build_default_registry(codergen_backend=_StubBackend())
        runner = HandlerRunner(graph, registry)
        events: list[dict[str, object]] = []
        context = Context(values={"context.stack.child.failure_reason": "child failed before run id"})

        outcome = runner(
            "manager",
            "",
            context,
            emit_event=lambda event_type, **payload: events.append({"type": event_type, **payload}),
        )

        assert outcome.status == OutcomeStatus.FAIL
        assert context.get("context.stack.child.intervention_status") == "rejected"
        assert context.get("context.stack.child.intervention_delivery_mode") == "none"
        assert context.get("context.stack.child.intervention_reason") == "no_active_child_run"
        assert events[0]["type"] == "ChildInterventionRequested"
        assert events[0]["reason"] == "no_active_child_run"

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
        milestone_ids: list[str] = []

        def launch_child(request: ChildRunRequest) -> ChildRunResult:
            milestone_ids.append(str(request.parent_context.get("context.milestone.id", "")))
            return _completed_child_result(request)

        registry = build_default_registry(codergen_backend=_StubBackend(ok=True))
        runner = HandlerRunner(
            graph,
            registry,
            logs_root=tmp_path / "logs",
            child_run_launcher=launch_child,
        )
        context = Context(values={"context.milestone.id": "M-ONE"})

        first = runner("manager", "", context)
        context.set("context.milestone.id", "M-TWO")
        second = runner("manager", "", context)

        assert first.status == OutcomeStatus.SUCCESS
        assert second.status == OutcomeStatus.SUCCESS
        assert milestone_ids == ["M-ONE", "M-TWO"]
