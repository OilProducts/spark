from __future__ import annotations

from dataclasses import dataclass
import json
from pathlib import Path
from typing import Callable, Optional

from attractor.dsl.models import DotGraph
from attractor.engine.checkpoint import Checkpoint


RESULT_DIR_NAME = "result"
RESULT_JSON_NAME = "result.json"
RESULT_MARKDOWN_NAME = "result.md"
DEFAULT_RESULT_SUMMARY_PROMPT = (
    "Summarize the following flow result for a user who needs the final outcome, "
    "important details, and any follow-up actions. Keep the answer concise and faithful "
    "to the source text."
)
TERMINAL_NODE_SHAPES = {"Mdiamond", "Msquare"}
SUCCESSFUL_SOURCE_OUTCOMES = {"success", "partial_success"}


@dataclass(frozen=True)
class RunResult:
    run_id: str
    status: str
    state: str
    source_node_id: str | None = None
    source_artifact_path: str | None = None
    display_mode: str | None = None
    body_markdown: str = ""
    summary_enabled: bool = False
    summary_prompt: str | None = None
    summary_error: str | None = None
    error: str | None = None

    def to_dict(self) -> dict[str, object]:
        return {
            "run_id": self.run_id,
            "status": self.status,
            "state": self.state,
            "source_node_id": self.source_node_id,
            "source_artifact_path": self.source_artifact_path,
            "display_mode": self.display_mode,
            "body_markdown": self.body_markdown,
            "summary_enabled": self.summary_enabled,
            "summary_prompt": self.summary_prompt,
            "summary_error": self.summary_error,
            "error": self.error,
        }

    @classmethod
    def from_dict(cls, payload: dict[str, object]) -> "RunResult":
        return cls(
            run_id=str(payload.get("run_id", "")),
            status=str(payload.get("status", "")),
            state=str(payload.get("state", "")),
            source_node_id=_optional_string(payload.get("source_node_id")),
            source_artifact_path=_optional_string(payload.get("source_artifact_path")),
            display_mode=_optional_string(payload.get("display_mode")),
            body_markdown=str(payload.get("body_markdown", "")),
            summary_enabled=payload.get("summary_enabled") is True,
            summary_prompt=_optional_string(payload.get("summary_prompt")),
            summary_error=_optional_string(payload.get("summary_error")),
            error=_optional_string(payload.get("error")),
        )


SummaryFn = Callable[[str, str], str]


def result_json_path(run_root: Path) -> Path:
    return run_root / RESULT_DIR_NAME / RESULT_JSON_NAME


def result_markdown_path(run_root: Path) -> Path:
    return run_root / RESULT_DIR_NAME / RESULT_MARKDOWN_NAME


def read_materialized_run_result(run_root: Path) -> RunResult | None:
    path = result_json_path(run_root)
    if not path.exists():
        return None
    try:
        payload = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError):
        return None
    if not isinstance(payload, dict):
        return None
    result = RunResult.from_dict(payload)
    markdown_path = result_markdown_path(run_root)
    if markdown_path.exists():
        try:
            return RunResult(
                **{
                    **result.to_dict(),
                    "body_markdown": markdown_path.read_text(encoding="utf-8"),
                }
            )
        except OSError:
            return result
    return result


def materialize_run_result(
    *,
    run_id: str,
    status: str,
    run_root: Path,
    graph: DotGraph,
    checkpoint: Checkpoint,
    summarize: SummaryFn | None = None,
) -> RunResult:
    try:
        result = _resolve_run_result(
            run_id=run_id,
            status=status,
            run_root=run_root,
            graph=graph,
            checkpoint=checkpoint,
            summarize=summarize,
        )
    except Exception as exc:  # noqa: BLE001
        result = RunResult(
            run_id=run_id,
            status=status,
            state="error",
            error=str(exc),
        )
    _write_run_result(run_root, result)
    return result


def pending_run_result(*, run_id: str, status: str) -> RunResult:
    return RunResult(run_id=run_id, status=status, state="pending")


def unavailable_run_result(*, run_id: str, status: str) -> RunResult:
    return RunResult(run_id=run_id, status=status, state="unavailable")


def _resolve_run_result(
    *,
    run_id: str,
    status: str,
    run_root: Path,
    graph: DotGraph,
    checkpoint: Checkpoint,
    summarize: SummaryFn | None,
) -> RunResult:
    source_node_id = _select_source_node_id(graph, checkpoint)
    if source_node_id is None:
        return unavailable_run_result(run_id=run_id, status=status)

    source_artifact_path = _source_artifact_path(run_root, source_node_id)
    if source_artifact_path is None:
        return unavailable_run_result(run_id=run_id, status=status)

    source_text = (run_root / source_artifact_path).read_text(encoding="utf-8")
    summary_enabled = _graph_attr_bool(graph, "spark.result_summary_enabled", default=False)
    summary_prompt = _graph_attr_text(graph, "spark.result_summary_prompt")
    if summary_enabled and not summary_prompt:
        summary_prompt = DEFAULT_RESULT_SUMMARY_PROMPT

    body_markdown = source_text
    display_mode = "raw"
    summary_error: str | None = None
    if summary_enabled:
        if summarize is None:
            summary_error = "Result summarizer is unavailable."
        else:
            try:
                summarized = summarize(summary_prompt or DEFAULT_RESULT_SUMMARY_PROMPT, source_text)
                if summarized.strip():
                    body_markdown = summarized
                    display_mode = "summary"
                else:
                    summary_error = "Result summarizer returned an empty response."
            except Exception as exc:  # noqa: BLE001
                summary_error = str(exc)

    return RunResult(
        run_id=run_id,
        status=status,
        state="ready",
        source_node_id=source_node_id,
        source_artifact_path=source_artifact_path.as_posix(),
        display_mode=display_mode,
        body_markdown=body_markdown,
        summary_enabled=summary_enabled,
        summary_prompt=summary_prompt,
        summary_error=summary_error,
    )


def _write_run_result(run_root: Path, result: RunResult) -> None:
    result_dir = run_root / RESULT_DIR_NAME
    result_dir.mkdir(parents=True, exist_ok=True)
    result_markdown_path(run_root).write_text(result.body_markdown, encoding="utf-8")
    result_json_path(run_root).write_text(
        json.dumps(result.to_dict(), indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )


def _select_source_node_id(graph: DotGraph, checkpoint: Checkpoint) -> str | None:
    explicit = _graph_attr_text(graph, "spark.result_node")
    if explicit:
        return explicit if _source_node_is_valid(graph, checkpoint, explicit) else None

    current_node = checkpoint.active_node
    completed_nodes = list(checkpoint.completed_nodes)
    node_outcomes = _node_outcomes(checkpoint)
    exit_predecessors = {
        edge.source
        for edge in graph.edges
        if _node_shape(graph, edge.target) == "Msquare"
        and (not current_node or edge.target == current_node or current_node not in graph.nodes)
    }
    for node_id in reversed(completed_nodes):
        if node_id not in exit_predecessors:
            continue
        if _source_node_is_valid(graph, checkpoint, node_id, node_outcomes=node_outcomes):
            return node_id
    for node_id in reversed(completed_nodes):
        if _source_node_is_valid(graph, checkpoint, node_id, node_outcomes=node_outcomes):
            return node_id
    return None


def _source_node_is_valid(
    graph: DotGraph,
    checkpoint: Checkpoint,
    node_id: str,
    *,
    node_outcomes: dict[str, str] | None = None,
) -> bool:
    if node_id not in graph.nodes:
        return False
    if _node_shape(graph, node_id) in TERMINAL_NODE_SHAPES:
        return False
    outcomes = node_outcomes if node_outcomes is not None else _node_outcomes(checkpoint)
    if outcomes:
        return outcomes.get(node_id) in SUCCESSFUL_SOURCE_OUTCOMES
    return node_id in checkpoint.completed_nodes


def _source_artifact_path(run_root: Path, node_id: str) -> Path | None:
    candidates = [
        Path("logs") / node_id / "response.md",
        Path(node_id) / "response.md",
    ]
    for candidate in candidates:
        path = run_root / candidate
        if path.exists() and path.is_file():
            return candidate
    return None


def _node_outcomes(checkpoint: Checkpoint) -> dict[str, str]:
    raw = checkpoint.context.get("_attractor.node_outcomes", {})
    if not isinstance(raw, dict):
        return {}
    return {str(key): str(value) for key, value in raw.items()}


def _node_shape(graph: DotGraph, node_id: str) -> str:
    node = graph.nodes.get(node_id)
    if node is None:
        return ""
    attr = node.attrs.get("shape")
    return str(attr.value) if attr is not None else ""


def _graph_attr_text(graph: DotGraph, key: str) -> str:
    attr = graph.graph_attrs.get(key)
    if attr is None:
        return ""
    return str(attr.value).strip()


def _graph_attr_bool(graph: DotGraph, key: str, *, default: bool) -> bool:
    value = _graph_attr_text(graph, key).lower()
    if not value:
        return default
    return value in {"1", "true", "yes", "on"}


def _optional_string(value: object) -> Optional[str]:
    if value is None:
        return None
    text = str(value)
    return text if text else None
