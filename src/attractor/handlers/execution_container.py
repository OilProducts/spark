from __future__ import annotations

from dataclasses import dataclass
import json
import os
from pathlib import Path
import shutil
import subprocess
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
from attractor.handlers.base import ChildRunRequest, ChildRunResult
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
                proc.stdin.write(json.dumps(request, sort_keys=True) + "\n")
                proc.stdin.flush()
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
                        answer = callbacks.ask_human(payload) if callbacks.ask_human is not None else {"value": "TIMEOUT"}
                        proc.stdin.write(json.dumps({"type": "human_gate_answer", "answer": answer}) + "\n")
                        proc.stdin.flush()
                    elif kind == "child_run_request":
                        response = callbacks.launch_child(payload) if callbacks.launch_child is not None else {
                            "run_id": str(payload.get("child_run_id", "")),
                            "status": "failed",
                            "failure_reason": "child run delegation is unavailable",
                        }
                        proc.stdin.write(json.dumps({"type": "child_run_result", "result": response}) + "\n")
                        proc.stdin.flush()
                    elif kind == "child_status_request":
                        child_run_id = str(payload.get("run_id", ""))
                        response = callbacks.resolve_child_status(child_run_id) if callbacks.resolve_child_status is not None else None
                        proc.stdin.write(json.dumps({"type": "child_status_result", "result": response}) + "\n")
                        proc.stdin.flush()
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
            return {"value": "TIMEOUT", "selected_values": []}
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


def run_worker_node() -> int:
    from attractor.api.codex_backends import build_codergen_backend

    request = _decode_json_line(input())
    graph = graph_from_payload(request["graph"])
    context = Context(values=dict(request.get("context") or {}), logs=list(request.get("context_logs") or []))
    working_dir = str(request.get("working_dir") or ".")

    def emit(message: dict[str, Any]) -> None:
        print(json.dumps({"type": "event", "event_type": "WorkerLog", "payload": message}, sort_keys=True), flush=True)

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
    runner = HandlerRunner(
        graph,
        registry,
        logs_root=Path(str(request["logs_root"])) if request.get("logs_root") else None,
        child_run_launcher=worker_child_run_launcher,
        child_status_resolver=worker_child_status_resolver,
    )

    def emit_event(event_type: str, **payload: object) -> None:
        print(
            json.dumps(
                {"type": "event", "event_type": event_type, "payload": payload},
                default=str,
                sort_keys=True,
            ),
            flush=True,
        )

    try:
        outcome = runner(str(request["node_id"]), str(request.get("prompt") or ""), context, emit_event=emit_event)
        print(
            json.dumps(
                {
                    "type": "result",
                    "outcome": outcome_to_payload(outcome),
                    "context": dict(context.values),
                },
                default=str,
                sort_keys=True,
            ),
            flush=True,
        )
        return 0
    except Exception as exc:  # noqa: BLE001
        print(
            json.dumps(
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
                },
                default=str,
                sort_keys=True,
            ),
            flush=True,
        )
        return 0


class WorkerProtocolInterviewer(Interviewer):
    def ask(self, question: Question) -> Answer:
        payload = {
            "type": "human_gate_request",
            "question": question_to_payload(question),
        }
        print(json.dumps(payload, default=str, sort_keys=True), flush=True)
        response = _decode_json_line(input())
        answer = response.get("answer") if isinstance(response.get("answer"), dict) else {}
        return answer_from_payload(answer)


def worker_child_run_launcher(request: ChildRunRequest) -> ChildRunResult:
    payload = {"type": "child_run_request", **child_run_request_to_payload(request)}
    print(json.dumps(payload, default=str, sort_keys=True), flush=True)
    response = _decode_json_line(input())
    result = response.get("result") if isinstance(response.get("result"), dict) else {}
    return child_run_result_from_payload(result)


def worker_child_status_resolver(run_id: str) -> ChildRunResult | None:
    print(json.dumps({"type": "child_status_request", "run_id": run_id}, sort_keys=True), flush=True)
    response = _decode_json_line(input())
    result = response.get("result")
    return child_run_result_from_payload(result) if isinstance(result, dict) else None


def _container_env() -> dict[str, str]:
    env = {key: os.environ[key] for key in PROVIDER_ENV_ALLOWLIST if os.environ.get(key)}
    codex_env = build_codex_runtime_environment()
    for key in ("HOME", "CODEX_HOME", "XDG_CONFIG_HOME", "XDG_DATA_HOME", "ATTRACTOR_CODEX_RUNTIME_ROOT", "SPARK_HOME"):
        value = codex_env.get(key) or os.environ.get(key)
        if value:
            env[key] = value
    return env


def _spark_runtime_root() -> Path | None:
    value = os.environ.get("ATTRACTOR_CODEX_RUNTIME_ROOT")
    if value:
        return Path(value).expanduser()
    spark_home = os.environ.get("SPARK_HOME")
    if spark_home:
        return Path(spark_home).expanduser() / "runtime" / "codex"
    return None


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
        "timeout_seconds": question.timeout_seconds,
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
        timeout_seconds=(
            float(payload["timeout_seconds"])
            if payload.get("timeout_seconds") is not None
            else None
        ),
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
    )
