from __future__ import annotations

from dataclasses import dataclass
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import threading
import uuid
from typing import Any, Callable, Iterable, Protocol

from attractor.dsl.models import (
    DotAttribute,
    DotEdge,
    DotGraph,
    DotNode,
    DotScopeDefaults,
    DotSubgraphScope,
    DotValueType,
    Duration,
)
from attractor.engine.context import Context
from attractor.engine.outcome import FailureKind, Outcome, OutcomeStatus
from attractor.execution import (
    EXECUTION_MODE_LOCAL_CONTAINER,
    ExecutionLaunchError,
)
from attractor.handlers.base import (
    ChildInterventionRequest,
    ChildInterventionResult,
    ChildRunRequest,
    ChildRunResult,
)
from attractor.interviewer import Answer, Interviewer, Question, QuestionOption, QuestionType
from spark_common.codex_runtime import build_codex_runtime_environment

from .defaults import build_default_registry
from .runner import HandlerRunner


PROVIDER_ENV_ALLOWLIST = (
    "OPENAI_API_KEY",
    "OPENAI_BASE_URL",
    "OPENAI_ORG_ID",
    "OPENAI_PROJECT_ID",
    "ANTHROPIC_API_KEY",
    "ANTHROPIC_BASE_URL",
    "GEMINI_API_KEY",
    "GEMINI_BASE_URL",
    "GOOGLE_API_KEY",
    "OPENROUTER_API_KEY",
    "OPENROUTER_BASE_URL",
    "OPENROUTER_HTTP_REFERER",
    "OPENROUTER_TITLE",
    "LITELLM_BASE_URL",
    "LITELLM_API_KEY",
    "OPENAI_COMPATIBLE_BASE_URL",
    "OPENAI_COMPATIBLE_API_KEY",
)


class ContainerExecutionError(ExecutionLaunchError):
    pass


class ContainerTransport(Protocol):
    def run_node(self, request: dict[str, Any], callbacks: "WorkerCallbacks") -> dict[str, Any]:
        ...

    def request_child_intervention(self, request: ChildInterventionRequest) -> ChildInterventionResult:
        ...

    def close(self) -> None:
        ...

    def cancel(self) -> None:
        ...


@dataclass
class WorkerCallbacks:
    emit_event: Callable[[str, dict[str, Any]], None] | None = None
    ask_human: Callable[[dict[str, Any]], dict[str, Any]] | None = None
    launch_child: Callable[[dict[str, Any]], dict[str, Any]] | None = None
    resolve_child_status: Callable[[str], dict[str, Any] | None] | None = None
    request_child_intervention: Callable[[dict[str, Any]], dict[str, Any]] | None = None


@dataclass
class _ControlResponseWaiter:
    event: threading.Event
    request: ChildInterventionRequest
    payload: dict[str, Any] | None = None


def _rejected_child_intervention_result(
    request: ChildInterventionRequest,
    *,
    reason: str,
    delivery_mode: str = "local_container",
    message: str = "",
) -> ChildInterventionResult:
    return ChildInterventionResult(
        run_id=request.child_run_id,
        status="rejected",
        delivery_mode=delivery_mode,
        reason=reason,
        message=message,
        target_node_id=request.target_node_id,
    )


_WORKER_STDOUT_LOCK = threading.Lock()
_WORKER_PROTOCOL_LOCK = threading.Lock()
_WORKER_PROTOCOL: "_WorkerProtocolBridge | None" = None


def _emit_worker_payload(payload: dict[str, Any]) -> None:
    with _WORKER_STDOUT_LOCK:
        print(json.dumps(payload, default=str, sort_keys=True), flush=True)


def _set_worker_protocol(protocol: "_WorkerProtocolBridge | None") -> None:
    global _WORKER_PROTOCOL
    with _WORKER_PROTOCOL_LOCK:
        _WORKER_PROTOCOL = protocol


def _worker_protocol() -> "_WorkerProtocolBridge | None":
    with _WORKER_PROTOCOL_LOCK:
        return _WORKER_PROTOCOL


def _worker_request_response(request: dict[str, Any], response_type: str) -> dict[str, Any]:
    protocol = _worker_protocol()
    if protocol is None:
        _emit_worker_payload(request)
        return _decode_json_line(input())
    return protocol.request_response(request, response_type)


class DockerContainerTransport:
    def __init__(
        self,
        *,
        image: str,
        run_id: str,
        project_path: Path,
        run_root: Path,
        spark_runtime_root: Path | None = None,
        docker: str = "docker",
        labels: dict[str, str] | None = None,
        env: dict[str, str] | None = None,
    ) -> None:
        if not shutil.which(docker):
            raise ContainerExecutionError(
                "Container execution requires Docker, but the docker CLI was not found."
            )
        self.image = image
        self.run_id = run_id
        self.project_path = project_path.expanduser().resolve(strict=False)
        self.run_root = run_root.expanduser().resolve(strict=False)
        self.spark_runtime_root = spark_runtime_root.expanduser().resolve(strict=False) if spark_runtime_root else None
        self.docker = docker
        self.labels = dict(labels or {})
        self.env = dict(env or {})
        self.container_name = f"spark-run-{run_id[:32]}-{uuid.uuid4().hex[:8]}"
        self.container_id: str | None = None
        self._lock = threading.Lock()
        self._proc_lock = threading.Lock()
        self._stdin_lock = threading.Lock()
        self._control_lock = threading.Lock()
        self._control_waiters: dict[str, _ControlResponseWaiter] = {}
        self._active_proc: subprocess.Popen[str] | None = None

    def run_node(self, request: dict[str, Any], callbacks: WorkerCallbacks) -> dict[str, Any]:
        with self._lock:
            self._ensure_started()
            assert self.container_id is not None
            command = [
                self.docker,
                "exec",
                "-i",
                self.container_id,
                "spark-server",
                "worker",
                "run-node",
            ]
            proc = subprocess.Popen(
                command,
                stdin=subprocess.PIPE,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                text=True,
                encoding="utf-8",
            )
            with self._proc_lock:
                self._active_proc = proc
            assert proc.stdin is not None
            assert proc.stdout is not None
            try:
                self._write_proc_payload(proc, request)
                result: dict[str, Any] | None = None
                for line in proc.stdout:
                    payload = _decode_json_line(line)
                    kind = str(payload.get("type", ""))
                    if kind == "event":
                        event_type = str(payload.get("event_type", ""))
                        event_payload = payload.get("payload") if isinstance(payload.get("payload"), dict) else {}
                        if callbacks.emit_event is not None:
                            callbacks.emit_event(event_type, dict(event_payload))
                    elif kind == "human_gate_request":
                        answer = callbacks.ask_human(payload) if callbacks.ask_human is not None else {"value": "SKIPPED"}
                        self._write_proc_payload(proc, {"type": "human_gate_answer", "answer": answer})
                    elif kind == "child_run_request":
                        response = callbacks.launch_child(payload) if callbacks.launch_child is not None else {
                            "run_id": str(payload.get("child_run_id", "")),
                            "status": "failed",
                            "failure_reason": "child run delegation is unavailable",
                        }
                        self._write_proc_payload(proc, {"type": "child_run_result", "result": response})
                    elif kind == "child_status_request":
                        child_run_id = str(payload.get("run_id", ""))
                        response = callbacks.resolve_child_status(child_run_id) if callbacks.resolve_child_status is not None else None
                        self._write_proc_payload(proc, {"type": "child_status_result", "result": response})
                    elif kind == "child_intervention_request":
                        response = callbacks.request_child_intervention(payload) if callbacks.request_child_intervention is not None else {
                            "run_id": str(payload.get("child_run_id", "")),
                            "status": "rejected",
                            "delivery_mode": "unsupported",
                            "reason": "backend_steering_unsupported",
                            "message": "child intervention delegation is unavailable",
                            "target_node_id": payload.get("target_node_id"),
                        }
                        self._write_proc_payload(proc, {"type": "child_intervention_result", "result": response})
                    elif kind == "child_intervention_control_result":
                        self._resolve_control_response(str(payload.get("request_id") or ""), payload)
                    elif kind == "result":
                        result = payload
                stderr = proc.stderr.read() if proc.stderr is not None else ""
                return_code = proc.wait()
                if return_code != 0:
                    raise ContainerExecutionError(
                        f"Container node worker failed with exit code {return_code}: {stderr.strip()}"
                    )
                if result is None:
                    raise ContainerExecutionError("Container node worker exited without a result payload.")
                return result
            finally:
                with self._proc_lock:
                    if self._active_proc is proc:
                        self._active_proc = None
                self._reject_control_waiters(
                    reason="no_active_container_worker",
                    message="Container worker is no longer active.",
                )

    def request_child_intervention(
        self,
        request: ChildInterventionRequest,
    ) -> ChildInterventionResult:
        request_id = uuid.uuid4().hex
        with self._proc_lock:
            proc = self._active_proc
        if proc is None or proc.poll() is not None or proc.stdin is None:
            return _rejected_child_intervention_result(
                request,
                reason="no_active_container_worker",
                message="No active local-container worker is available for intervention.",
            )

        waiter = _ControlResponseWaiter(event=threading.Event(), request=request)
        with self._control_lock:
            self._control_waiters[request_id] = waiter
        try:
            self._write_proc_payload(
                proc,
                {
                    "type": "child_intervention_control_request",
                    "request_id": request_id,
                    "request": child_intervention_request_to_payload(request),
                },
            )
        except Exception as exc:  # noqa: BLE001
            with self._control_lock:
                self._control_waiters.pop(request_id, None)
            return _rejected_child_intervention_result(
                request,
                reason="no_active_container_worker",
                message=str(exc),
            )

        timeout_seconds = _container_steer_timeout_seconds()
        if not waiter.event.wait(timeout_seconds):
            with self._control_lock:
                self._control_waiters.pop(request_id, None)
            return _rejected_child_intervention_result(
                request,
                reason="intervention_request_failed",
                message="Timed out waiting for local-container worker intervention result.",
            )

        payload = waiter.payload or {}
        result = payload.get("result") if isinstance(payload.get("result"), dict) else {}
        if not isinstance(result, dict) or not result:
            return _rejected_child_intervention_result(
                request,
                reason="intervention_request_failed",
                message="Local-container worker returned an invalid intervention result.",
            )
        return child_intervention_result_from_payload(result)

    def _write_proc_payload(self, proc: subprocess.Popen[str], payload: dict[str, Any]) -> None:
        if proc.stdin is None:
            raise ContainerExecutionError("Container node worker stdin is unavailable.")
        with self._stdin_lock:
            proc.stdin.write(json.dumps(payload, default=str, sort_keys=True) + "\n")
            proc.stdin.flush()

    def _resolve_control_response(self, request_id: str, payload: dict[str, Any]) -> None:
        with self._control_lock:
            waiter = self._control_waiters.pop(request_id, None)
        if waiter is None:
            return
        waiter.payload = payload
        waiter.event.set()

    def _reject_control_waiters(self, *, reason: str, message: str) -> None:
        with self._control_lock:
            waiters = list(self._control_waiters.values())
            self._control_waiters.clear()
        for waiter in waiters:
            waiter.payload = {
                "type": "child_intervention_control_result",
                "result": {
                    "run_id": waiter.request.child_run_id,
                    "status": "rejected",
                    "delivery_mode": "local_container",
                    "reason": reason,
                    "message": message,
                    "target_node_id": waiter.request.target_node_id,
                },
            }
            waiter.event.set()

    def close(self) -> None:
        if not self.container_id:
            return
        subprocess.run([self.docker, "rm", "-f", self.container_id], check=False, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        self.container_id = None

    def cancel(self) -> None:
        with self._proc_lock:
            proc = self._active_proc
        if proc is not None and proc.poll() is None:
            proc.terminate()
            try:
                proc.wait(timeout=2)
            except subprocess.TimeoutExpired:
                proc.kill()
        self.close()

    def _ensure_started(self) -> None:
        if self.container_id:
            return
        mounts = [
            _mount_arg(self.project_path, self.project_path),
            _mount_arg(self.run_root.parent, self.run_root.parent),
            _mount_arg(self.run_root, self.run_root),
        ]
        if self.spark_runtime_root is not None:
            mounts.append(_mount_arg(self.spark_runtime_root, self.spark_runtime_root))
        args = [
            self.docker,
            "run",
            "-d",
            "--name",
            self.container_name,
        ]
        for key, value in {"spark.run_id": self.run_id, **self.labels}.items():
            if str(value).strip():
                args.extend(["--label", f"{key}={value}"])
        container_user = _container_user(
            project_path=self.project_path,
            run_root=self.run_root,
            spark_runtime_root=self.spark_runtime_root,
        )
        if container_user:
            args.extend(["--user", container_user])
        for mount in _dedupe(mounts):
            args.extend(["-v", mount])
        for key, value in self.env.items():
            args.extend(["-e", f"{key}={value}"])
        args.extend([self.image, "tail", "-f", "/dev/null"])
        started = subprocess.run(args, check=False, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        if started.returncode != 0:
            raise ContainerExecutionError(
                f"Unable to start execution container from image {self.image}: {started.stderr.strip()}"
            )
        self.container_id = started.stdout.strip() or self.container_name


class ContainerizedHandlerRunner:
    def __init__(
        self,
        graph: DotGraph,
        *,
        image: str,
        run_id: str,
        working_dir: str | Path,
        run_root: str | Path,
        transport: ContainerTransport | None = None,
        control: Callable[[], str | None] | None = None,
        child_run_launcher: Callable[[ChildRunRequest], ChildRunResult] | None = None,
        child_status_resolver: Callable[[str], ChildRunResult | None] | None = None,
        child_intervention_requester: Callable[
            [ChildInterventionRequest], ChildInterventionResult
        ] | None = None,
        interviewer: Interviewer | None = None,
    ) -> None:
        self.graph = graph
        self.image = image
        self.run_id = run_id
        self.working_dir = Path(working_dir).expanduser().resolve(strict=False)
        self.run_root = Path(run_root).expanduser().resolve(strict=False)
        self.logs_root: Path | None = None
        self.control = control
        self.child_run_launcher = child_run_launcher
        self.child_status_resolver = child_status_resolver
        self.child_intervention_requester = child_intervention_requester
        self.interviewer = interviewer
        self._owns_transport = transport is None
        self.transport = transport or DockerContainerTransport(
            image=image,
            run_id=run_id,
            project_path=self.working_dir,
            run_root=self.run_root,
            spark_runtime_root=_spark_runtime_root(),
            labels={
                "spark.execution_mode": EXECUTION_MODE_LOCAL_CONTAINER,
                "spark.project_path": str(self.working_dir),
            },
            env=_container_env(),
        )

    def __call__(
        self,
        node_id: str,
        prompt: str,
        context: Context,
        *,
        emit_event: Callable[..., None] | None = None,
    ) -> Outcome | None:
        request = {
            "run_id": self.run_id,
            "graph": graph_to_payload(self.graph),
            "node_id": node_id,
            "prompt": prompt,
            "context": dict(context.snapshot()),
            "context_logs": list(context.logs),
            "logs_root": str(self.logs_root) if self.logs_root is not None else None,
            "working_dir": str(self.working_dir),
            "backend_name": "provider-router",
            "model": str(context.get("_attractor.runtime.launch_model", "") or "") or None,
            "config_dir": os.environ.get("SPARK_CONFIG_DIR"),
        }
        callbacks = WorkerCallbacks(
            emit_event=(lambda event_type, payload: emit_event(event_type, **payload)) if emit_event else None,
            ask_human=self._ask_human,
            launch_child=self._launch_child,
            resolve_child_status=self._resolve_child_status,
            request_child_intervention=self._request_child_intervention_payload,
        )
        result = self.transport.run_node(request, callbacks)
        if isinstance(result.get("context"), dict):
            context.apply_updates(dict(result["context"]))
        return outcome_from_payload(result.get("outcome"))

    def set_logs_root(self, logs_root: str | Path | None) -> None:
        self.logs_root = Path(logs_root) if logs_root is not None else None

    def set_control(self, control: Callable[[], str | None] | None) -> None:
        self.control = control

    def close(self) -> None:
        if self._owns_transport:
            self.transport.close()

    def _ask_human(self, payload: dict[str, Any]) -> dict[str, Any]:
        if self.interviewer is None:
            return {"value": "SKIPPED", "selected_values": []}
        question_payload = payload.get("question") if isinstance(payload.get("question"), dict) else {}
        return answer_to_payload(self.interviewer.ask(question_from_payload(question_payload)))

    def _launch_child(self, payload: dict[str, Any]) -> dict[str, Any]:
        if self.child_run_launcher is None:
            return {
                "run_id": str(payload.get("child_run_id", "")),
                "status": "failed",
                "failure_reason": "child run launcher is unavailable",
            }
        request = child_run_request_from_payload(payload)
        return child_run_result_to_payload(self.child_run_launcher(request))

    def _resolve_child_status(self, run_id: str) -> dict[str, Any] | None:
        if self.child_status_resolver is None:
            return None
        result = self.child_status_resolver(run_id)
        return child_run_result_to_payload(result) if result is not None else None

    def _request_child_intervention_payload(self, payload: dict[str, Any]) -> dict[str, Any]:
        request = child_intervention_request_from_payload(payload)
        if self.child_intervention_requester is None:
            return child_intervention_result_to_payload(
                _rejected_child_intervention_result(
                    request,
                    reason="backend_steering_unsupported",
                    delivery_mode="unsupported",
                    message="child intervention requester is unavailable",
                )
            )
        return child_intervention_result_to_payload(self.child_intervention_requester(request))

    def request_child_intervention(
        self,
        request: ChildInterventionRequest,
    ) -> ChildInterventionResult:
        requester = getattr(self.transport, "request_child_intervention", None)
        if not callable(requester):
            return _rejected_child_intervention_result(
                request,
                reason="backend_steering_unsupported",
                delivery_mode="unsupported",
                message="Active local-container transport does not support intervention.",
            )
        return requester(request)


@dataclass
class _WorkerResponseWaiter:
    event: threading.Event
    payload: dict[str, Any] | None = None


class _WorkerProtocolBridge:
    _CALLBACK_RESPONSE_TYPES = {
        "human_gate_answer",
        "child_run_result",
        "child_status_result",
        "child_intervention_result",
    }

    def __init__(
        self,
        intervention_handler: Callable[[ChildInterventionRequest], ChildInterventionResult],
    ) -> None:
        self._intervention_handler = intervention_handler
        self._lock = threading.Lock()
        self._pending: dict[str, list[_WorkerResponseWaiter]] = {}
        self._closed = False
        self._thread = threading.Thread(target=self._read_loop, name="spark-worker-protocol", daemon=True)

    def start(self) -> None:
        self._thread.start()

    def close(self) -> None:
        with self._lock:
            self._closed = True
            waiters = [waiter for waiters in self._pending.values() for waiter in waiters]
            self._pending.clear()
        for waiter in waiters:
            waiter.payload = {}
            waiter.event.set()

    def request_response(self, request: dict[str, Any], response_type: str) -> dict[str, Any]:
        waiter = _WorkerResponseWaiter(event=threading.Event())
        with self._lock:
            if self._closed:
                return {}
            self._pending.setdefault(response_type, []).append(waiter)
        _emit_worker_payload(request)
        waiter.event.wait()
        return waiter.payload or {}

    def _read_loop(self) -> None:
        try:
            try:
                for line in sys.stdin:
                    try:
                        payload = _decode_json_line(line)
                    except Exception:  # noqa: BLE001
                        continue
                    kind = str(payload.get("type", ""))
                    if kind in self._CALLBACK_RESPONSE_TYPES:
                        self._dispatch_callback_response(kind, payload)
                    elif kind == "child_intervention_control_request":
                        self._handle_control_request(payload)
            except OSError:
                return
        finally:
            self.close()

    def _dispatch_callback_response(self, response_type: str, payload: dict[str, Any]) -> None:
        with self._lock:
            waiters = self._pending.get(response_type) or []
            waiter = waiters.pop(0) if waiters else None
            if not waiters:
                self._pending.pop(response_type, None)
            if waiter is None:
                return
        waiter.payload = payload
        waiter.event.set()

    def _handle_control_request(self, payload: dict[str, Any]) -> None:
        request_id = str(payload.get("request_id") or "")
        request_payload = payload.get("request") if isinstance(payload.get("request"), dict) else {}
        request = child_intervention_request_from_payload(request_payload)
        try:
            result = self._intervention_handler(request)
        except Exception as exc:  # noqa: BLE001
            result = _rejected_child_intervention_result(
                request,
                reason="intervention_request_failed",
                delivery_mode="error",
                message=str(exc),
            )
        _emit_worker_payload(
            {
                "type": "child_intervention_control_result",
                "request_id": request_id,
                "result": child_intervention_result_to_payload(result),
            }
        )


def run_worker_node() -> int:
    from attractor.api.codex_backends import build_codergen_backend

    request = _decode_json_line(input())
    graph = graph_from_payload(request["graph"])
    context = Context(values=dict(request.get("context") or {}), logs=list(request.get("context_logs") or []))
    working_dir = str(request.get("working_dir") or ".")
    worker_run_id = str(request.get("run_id") or context.get("internal.run_id") or "")

    def emit(message: dict[str, Any]) -> None:
        _emit_worker_payload({"type": "event", "event_type": "WorkerLog", "payload": message})

    backend = build_codergen_backend(
        str(request.get("backend_name") or "provider-router"),
        working_dir,
        emit,
        model=request.get("model") if isinstance(request.get("model"), str) else None,
        config_dir=request.get("config_dir") if isinstance(request.get("config_dir"), str) else None,
    )
    registry = build_default_registry(
        codergen_backend=backend,
        interviewer=WorkerProtocolInterviewer(),
    )

    def request_child_intervention(
        intervention_request: ChildInterventionRequest,
    ) -> ChildInterventionResult:
        if worker_run_id and intervention_request.child_run_id == worker_run_id:
            requester = getattr(backend, "request_child_intervention", None)
            if not callable(requester):
                return _rejected_child_intervention_result(
                    intervention_request,
                    reason="backend_steering_unsupported",
                    delivery_mode="unsupported",
                    message="Active worker backend does not support intervention.",
                )
            return requester(intervention_request)
        return worker_child_intervention_requester(intervention_request)

    runner = HandlerRunner(
        graph,
        registry,
        logs_root=Path(str(request["logs_root"])) if request.get("logs_root") else None,
        child_run_launcher=worker_child_run_launcher,
        child_status_resolver=worker_child_status_resolver,
        child_intervention_requester=request_child_intervention,
    )
    protocol = _WorkerProtocolBridge(runner.request_child_intervention)
    _set_worker_protocol(protocol)
    protocol.start()

    def emit_event(event_type: str, **payload: object) -> None:
        _emit_worker_payload({"type": "event", "event_type": event_type, "payload": payload})

    try:
        outcome = runner(str(request["node_id"]), str(request.get("prompt") or ""), context, emit_event=emit_event)
        _emit_worker_payload(
            {
                "type": "result",
                "outcome": outcome_to_payload(outcome),
                "context": dict(context.values),
            }
        )
        return 0
    except Exception as exc:  # noqa: BLE001
        _emit_worker_payload(
            {
                "type": "result",
                "outcome": outcome_to_payload(
                    Outcome(
                        status=OutcomeStatus.FAIL,
                        failure_reason=str(exc),
                        retryable=False,
                        failure_kind=FailureKind.RUNTIME,
                    )
                ),
                "context": dict(context.values),
            }
        )
        return 0
    finally:
        protocol.close()
        _set_worker_protocol(None)


class WorkerProtocolInterviewer(Interviewer):
    def ask(self, question: Question) -> Answer:
        payload = {
            "type": "human_gate_request",
            "question": question_to_payload(question),
        }
        response = _worker_request_response(payload, "human_gate_answer")
        answer = response.get("answer") if isinstance(response.get("answer"), dict) else {}
        return answer_from_payload(answer)


def worker_child_run_launcher(request: ChildRunRequest) -> ChildRunResult:
    payload = {"type": "child_run_request", **child_run_request_to_payload(request)}
    response = _worker_request_response(payload, "child_run_result")
    result = response.get("result") if isinstance(response.get("result"), dict) else {}
    return child_run_result_from_payload(result)


def worker_child_status_resolver(run_id: str) -> ChildRunResult | None:
    response = _worker_request_response({"type": "child_status_request", "run_id": run_id}, "child_status_result")
    result = response.get("result")
    return child_run_result_from_payload(result) if isinstance(result, dict) else None


def worker_child_intervention_requester(
    request: ChildInterventionRequest,
) -> ChildInterventionResult:
    payload = {"type": "child_intervention_request", **child_intervention_request_to_payload(request)}
    response = _worker_request_response(payload, "child_intervention_result")
    result = response.get("result") if isinstance(response.get("result"), dict) else {}
    return child_intervention_result_from_payload(result)


def _container_env() -> dict[str, str]:
    env = {key: os.environ[key] for key in PROVIDER_ENV_ALLOWLIST if os.environ.get(key)}
    codex_env = build_codex_runtime_environment()
    for key in ("HOME", "CODEX_HOME", "XDG_CONFIG_HOME", "XDG_DATA_HOME", "ATTRACTOR_CODEX_RUNTIME_ROOT", "SPARK_HOME"):
        value = codex_env.get(key) or os.environ.get(key)
        if value:
            env[key] = value
    return env


def _container_steer_timeout_seconds() -> float:
    raw = str(os.environ.get("SPARK_CONTAINER_STEER_TIMEOUT_SECONDS", "")).strip()
    if not raw:
        return 30.0
    try:
        value = float(raw)
    except ValueError:
        return 30.0
    return value if value > 0 else 30.0


def _spark_runtime_root() -> Path | None:
    value = os.environ.get("ATTRACTOR_CODEX_RUNTIME_ROOT")
    if value:
        return Path(value).expanduser()
    spark_home = os.environ.get("SPARK_HOME")
    if spark_home:
        return Path(spark_home).expanduser() / "runtime" / "codex"
    return None


def _container_user(
    *,
    project_path: Path,
    run_root: Path,
    spark_runtime_root: Path | None,
) -> str | None:
    for uid_key, gid_key in (
        ("SPARK_CONTAINER_UID", "SPARK_CONTAINER_GID"),
        ("SPARK_DOCKER_HOST_UID", "SPARK_DOCKER_HOST_GID"),
        ("HOST_UID", "HOST_GID"),
    ):
        user = _uid_gid_from_env(uid_key, gid_key)
        if user is not None:
            return user

    for path in (spark_runtime_root, run_root.parent, project_path):
        owner = _uid_gid_from_path(path) if path is not None else None
        if owner is not None and owner[0] != 0:
            return f"{owner[0]}:{owner[1]}"

    uid = os.getuid()
    gid = os.getgid()
    if uid == 0:
        return None
    return f"{uid}:{gid}"


def _uid_gid_from_env(uid_key: str, gid_key: str) -> str | None:
    raw_uid = str(os.environ.get(uid_key, "")).strip()
    raw_gid = str(os.environ.get(gid_key, "")).strip()
    if not raw_uid and not raw_gid:
        return None
    if not raw_uid or not raw_gid:
        raise ContainerExecutionError(f"{uid_key} and {gid_key} must be provided together.")
    try:
        uid = int(raw_uid)
        gid = int(raw_gid)
    except ValueError as exc:
        raise ContainerExecutionError(f"{uid_key} and {gid_key} must be integer values.") from exc
    if uid < 0 or gid < 0:
        raise ContainerExecutionError(f"{uid_key} and {gid_key} must be non-negative integer values.")
    return f"{uid}:{gid}"


def _uid_gid_from_path(path: Path) -> tuple[int, int] | None:
    candidate = path.expanduser()
    while True:
        try:
            stat_result = candidate.stat()
            return stat_result.st_uid, stat_result.st_gid
        except OSError:
            parent = candidate.parent
            if parent == candidate:
                return None
            candidate = parent


def _mount_arg(source: Path, target: Path) -> str:
    return f"{_host_path(source)}:{target}:rw"


def _host_path(path: Path) -> Path:
    resolved = path.expanduser().resolve(strict=False)
    mappings = [
        (Path("/projects"), os.environ.get("SPARK_PROJECTS_HOST_DIR")),
        (Path("/spark"), os.environ.get("SPARK_DOCKER_HOME")),
    ]
    for container_prefix, host_prefix in mappings:
        try:
            rel = resolved.relative_to(container_prefix)
        except ValueError:
            continue
        if not host_prefix:
            raise ContainerExecutionError(
                f"Container execution path {resolved} requires host path mapping for {container_prefix}."
            )
        return Path(host_prefix).expanduser().resolve(strict=False) / rel
    return resolved


def _dedupe(values: Iterable[str]) -> list[str]:
    result: list[str] = []
    seen: set[str] = set()
    for value in values:
        if value in seen:
            continue
        seen.add(value)
        result.append(value)
    return result


def _decode_json_line(line: str) -> dict[str, Any]:
    payload = json.loads(line)
    if not isinstance(payload, dict):
        raise ContainerExecutionError("Worker protocol payload must be a JSON object.")
    return payload


def outcome_to_payload(outcome: Outcome | None) -> dict[str, Any] | None:
    if outcome is None:
        return None
    payload = outcome.to_payload()
    payload["retryable"] = outcome.retryable
    payload["raw_response_text"] = outcome.raw_response_text
    return payload


def outcome_from_payload(payload: Any) -> Outcome | None:
    if payload is None:
        return None
    if not isinstance(payload, dict):
        return Outcome(status=OutcomeStatus.FAIL, failure_reason="worker returned invalid outcome")
    status = OutcomeStatus(str(payload.get("status") or "fail"))
    failure_kind = payload.get("failure_kind")
    return Outcome(
        status=status,
        preferred_label=str(payload.get("preferred_label") or ""),
        suggested_next_ids=[str(item) for item in payload.get("suggested_next_ids") or []],
        context_updates=dict(payload.get("context_updates") or {}),
        failure_reason=str(payload.get("failure_reason") or ""),
        notes=str(payload.get("notes") or ""),
        retryable=payload.get("retryable") if isinstance(payload.get("retryable"), bool) else None,
        failure_kind=FailureKind(str(failure_kind)) if failure_kind else None,
        raw_response_text=str(payload.get("raw_response_text") or ""),
    )


def question_to_payload(question: Question) -> dict[str, Any]:
    return {
        "text": question.text,
        "type": question.type.value,
        "options": [
            {"label": option.label, "value": option.value, "key": option.key}
            for option in question.options
        ],
        "stage": question.stage,
        "metadata": dict(question.metadata),
    }


def question_from_payload(payload: dict[str, Any]) -> Question:
    return Question(
        text=str(payload.get("text") or ""),
        type=QuestionType(str(payload.get("type") or QuestionType.FREEFORM.value)),
        options=[
            QuestionOption(
                label=str(option.get("label") or ""),
                value=str(option.get("value") or ""),
                key=str(option.get("key") or ""),
            )
            for option in payload.get("options") or []
            if isinstance(option, dict)
        ],
        stage=str(payload.get("stage") or ""),
        metadata=dict(payload.get("metadata") or {}),
    )


def answer_to_payload(answer: Answer) -> dict[str, Any]:
    return {
        "value": answer.value,
        "text": answer.text,
        "selected_values": answer.selected_values,
    }


def answer_from_payload(payload: dict[str, Any]) -> Answer:
    return Answer(
        value=str(payload.get("value") or ""),
        text=str(payload.get("text") or ""),
        selected_values=[str(item) for item in payload.get("selected_values") or []],
    )


def graph_to_payload(graph: DotGraph) -> dict[str, Any]:
    return {
        "graph_id": graph.graph_id,
        "graph_attrs": attrs_to_payload(graph.graph_attrs),
        "nodes": {node_id: node_to_payload(node) for node_id, node in graph.nodes.items()},
        "edges": [edge_to_payload(edge) for edge in graph.edges],
        "defaults": defaults_to_payload(graph.defaults),
        "subgraphs": [subgraph_to_payload(scope) for scope in graph.subgraphs],
    }


def graph_from_payload(payload: dict[str, Any]) -> DotGraph:
    return DotGraph(
        graph_id=str(payload.get("graph_id") or ""),
        graph_attrs=attrs_from_payload(payload.get("graph_attrs") or {}),
        nodes={str(key): node_from_payload(value) for key, value in dict(payload.get("nodes") or {}).items()},
        edges=[edge_from_payload(value) for value in payload.get("edges") or []],
        defaults=defaults_from_payload(payload.get("defaults") or {}),
        subgraphs=[subgraph_from_payload(value) for value in payload.get("subgraphs") or []],
    )


def attrs_to_payload(attrs: dict[str, DotAttribute]) -> dict[str, Any]:
    return {key: attr_to_payload(attr) for key, attr in attrs.items()}


def attrs_from_payload(payload: dict[str, Any]) -> dict[str, DotAttribute]:
    return {str(key): attr_from_payload(value) for key, value in dict(payload).items()}


def attr_to_payload(attr: DotAttribute) -> dict[str, Any]:
    return {
        "key": attr.key,
        "value": value_to_payload(attr.value),
        "value_type": attr.value_type.value,
        "line": attr.line,
    }


def attr_from_payload(payload: dict[str, Any]) -> DotAttribute:
    value_type = DotValueType(str(payload.get("value_type") or DotValueType.STRING.value))
    return DotAttribute(
        key=str(payload.get("key") or ""),
        value=value_from_payload(payload.get("value"), value_type),
        value_type=value_type,
        line=int(payload.get("line") or 0),
    )


def value_to_payload(value: Any) -> Any:
    if isinstance(value, Duration):
        return {"raw": value.raw, "value": value.value, "unit": value.unit}
    return value


def value_from_payload(value: Any, value_type: DotValueType) -> Any:
    if value_type == DotValueType.DURATION and isinstance(value, dict):
        return Duration(raw=str(value.get("raw") or ""), value=int(value.get("value") or 0), unit=str(value.get("unit") or "s"))
    return value


def node_to_payload(node: DotNode) -> dict[str, Any]:
    return {
        "node_id": node.node_id,
        "attrs": attrs_to_payload(node.attrs),
        "line": node.line,
        "explicit_attr_keys": sorted(node.explicit_attr_keys),
    }


def node_from_payload(payload: dict[str, Any]) -> DotNode:
    return DotNode(
        node_id=str(payload.get("node_id") or ""),
        attrs=attrs_from_payload(payload.get("attrs") or {}),
        line=int(payload.get("line") or 0),
        explicit_attr_keys={str(item) for item in payload.get("explicit_attr_keys") or []},
    )


def edge_to_payload(edge: DotEdge) -> dict[str, Any]:
    return {"source": edge.source, "target": edge.target, "attrs": attrs_to_payload(edge.attrs), "line": edge.line}


def edge_from_payload(payload: dict[str, Any]) -> DotEdge:
    return DotEdge(
        source=str(payload.get("source") or ""),
        target=str(payload.get("target") or ""),
        attrs=attrs_from_payload(payload.get("attrs") or {}),
        line=int(payload.get("line") or 0),
    )


def defaults_to_payload(defaults: DotScopeDefaults) -> dict[str, Any]:
    return {"node": attrs_to_payload(defaults.node), "edge": attrs_to_payload(defaults.edge)}


def defaults_from_payload(payload: dict[str, Any]) -> DotScopeDefaults:
    return DotScopeDefaults(
        node=attrs_from_payload(payload.get("node") or {}),
        edge=attrs_from_payload(payload.get("edge") or {}),
    )


def subgraph_to_payload(scope: DotSubgraphScope) -> dict[str, Any]:
    return {
        "id": scope.id,
        "attrs": attrs_to_payload(scope.attrs),
        "node_ids": list(scope.node_ids),
        "defaults": defaults_to_payload(scope.defaults),
        "subgraphs": [subgraph_to_payload(child) for child in scope.subgraphs],
    }


def subgraph_from_payload(payload: dict[str, Any]) -> DotSubgraphScope:
    return DotSubgraphScope(
        id=str(payload["id"]) if payload.get("id") is not None else None,
        attrs=attrs_from_payload(payload.get("attrs") or {}),
        node_ids=[str(item) for item in payload.get("node_ids") or []],
        defaults=defaults_from_payload(payload.get("defaults") or {}),
        subgraphs=[subgraph_from_payload(child) for child in payload.get("subgraphs") or []],
    )


def child_run_request_to_payload(request: ChildRunRequest) -> dict[str, Any]:
    return {
        "child_run_id": request.child_run_id,
        "child_graph": graph_to_payload(request.child_graph),
        "child_flow_name": request.child_flow_name,
        "child_flow_path": str(request.child_flow_path),
        "child_workdir": str(request.child_workdir),
        "parent_context": dict(request.parent_context.values),
        "parent_run_id": request.parent_run_id,
        "parent_node_id": request.parent_node_id,
        "root_run_id": request.root_run_id,
    }


def child_run_request_from_payload(payload: dict[str, Any]) -> ChildRunRequest:
    return ChildRunRequest(
        child_run_id=str(payload.get("child_run_id") or ""),
        child_graph=graph_from_payload(payload.get("child_graph") or {}),
        child_flow_name=str(payload.get("child_flow_name") or ""),
        child_flow_path=Path(str(payload.get("child_flow_path") or "")),
        child_workdir=Path(str(payload.get("child_workdir") or ".")),
        parent_context=Context(values=dict(payload.get("parent_context") or {})),
        parent_run_id=str(payload.get("parent_run_id") or ""),
        parent_node_id=str(payload.get("parent_node_id") or ""),
        root_run_id=str(payload.get("root_run_id") or ""),
    )


def child_run_result_to_payload(result: ChildRunResult) -> dict[str, Any]:
    return {
        "run_id": result.run_id,
        "status": result.status,
        "outcome": result.outcome,
        "outcome_reason_code": result.outcome_reason_code,
        "outcome_reason_message": result.outcome_reason_message,
        "current_node": result.current_node,
        "completed_nodes": list(result.completed_nodes),
        "route_trace": list(result.route_trace),
        "failure_reason": result.failure_reason,
        "retry_count": result.retry_count,
        "retry_counts": dict(result.retry_counts),
        "artifact_count": result.artifact_count,
        "event_count": result.event_count,
        "checkpoint_timestamp": result.checkpoint_timestamp,
        "latest_event_at": result.latest_event_at,
        "started_at": result.started_at,
        "ended_at": result.ended_at,
    }


def child_run_result_from_payload(payload: dict[str, Any]) -> ChildRunResult:
    return ChildRunResult(
        run_id=str(payload.get("run_id") or ""),
        status=str(payload.get("status") or ""),
        outcome=str(payload["outcome"]) if payload.get("outcome") is not None else None,
        outcome_reason_code=str(payload["outcome_reason_code"]) if payload.get("outcome_reason_code") is not None else None,
        outcome_reason_message=str(payload["outcome_reason_message"]) if payload.get("outcome_reason_message") is not None else None,
        current_node=str(payload.get("current_node") or ""),
        completed_nodes=[str(item) for item in payload.get("completed_nodes") or []],
        route_trace=[str(item) for item in payload.get("route_trace") or []],
        failure_reason=str(payload.get("failure_reason") or ""),
        retry_count=_optional_int(payload.get("retry_count")),
        retry_counts={str(key): int(value) for key, value in dict(payload.get("retry_counts") or {}).items()},
        artifact_count=_optional_int(payload.get("artifact_count")),
        event_count=_optional_int(payload.get("event_count")),
        checkpoint_timestamp=str(payload.get("checkpoint_timestamp") or ""),
        latest_event_at=str(payload.get("latest_event_at") or ""),
        started_at=str(payload.get("started_at") or ""),
        ended_at=str(payload["ended_at"]) if payload.get("ended_at") is not None else None,
    )


def _optional_int(value: object) -> int | None:
    if value is None or value == "":
        return None
    return int(value)


def child_intervention_request_to_payload(request: ChildInterventionRequest) -> dict[str, Any]:
    return {
        "child_run_id": request.child_run_id,
        "message": request.message,
        "parent_run_id": request.parent_run_id,
        "parent_node_id": request.parent_node_id,
        "root_run_id": request.root_run_id,
        "reason": request.reason,
        "source": request.source,
        "cycle": request.cycle,
        "target_node_id": request.target_node_id,
    }


def child_intervention_request_from_payload(payload: dict[str, Any]) -> ChildInterventionRequest:
    raw_cycle = payload.get("cycle")
    cycle = int(raw_cycle) if isinstance(raw_cycle, (int, float, str)) and str(raw_cycle).strip() else None
    return ChildInterventionRequest(
        child_run_id=str(payload.get("child_run_id") or ""),
        message=str(payload.get("message") or ""),
        parent_run_id=str(payload.get("parent_run_id") or ""),
        parent_node_id=str(payload.get("parent_node_id") or ""),
        root_run_id=str(payload.get("root_run_id") or ""),
        reason=str(payload.get("reason") or ""),
        source=str(payload.get("source") or "manager_loop"),
        cycle=cycle,
        target_node_id=str(payload["target_node_id"]) if payload.get("target_node_id") is not None else None,
    )


def child_intervention_result_to_payload(result: ChildInterventionResult) -> dict[str, Any]:
    return {
        "run_id": result.run_id,
        "status": result.status,
        "delivery_mode": result.delivery_mode,
        "reason": result.reason,
        "message": result.message,
        "target_node_id": result.target_node_id,
    }


def child_intervention_result_from_payload(payload: dict[str, Any]) -> ChildInterventionResult:
    return ChildInterventionResult(
        run_id=str(payload.get("run_id") or ""),
        status=str(payload.get("status") or ""),
        delivery_mode=str(payload.get("delivery_mode") or ""),
        reason=str(payload.get("reason") or ""),
        message=str(payload.get("message") or ""),
        target_node_id=str(payload["target_node_id"]) if payload.get("target_node_id") is not None else None,
    )
