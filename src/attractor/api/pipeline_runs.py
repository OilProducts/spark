from __future__ import annotations

import json
import mimetypes
import subprocess
import threading
from datetime import UTC, datetime
from pathlib import Path
from typing import Callable, Dict, List, Optional

from fastapi import HTTPException

from attractor.api.run_records import (
    RunRecord,
    extract_token_usage,
    normalize_run_status,
)
from attractor.api.token_usage import TokenUsageBreakdown, estimate_model_cost
from attractor.api.runtime_paths import AttractorRuntimePaths
from attractor.engine import load_checkpoint
from attractor.graph_prep import (
    DEFAULT_MAX_RETRIES_KEY,
    resolve_default_max_retries_value,
)
from spark_common.project_identity import build_project_id, normalize_project_path


def runs_root(get_runtime_paths: Callable[[], AttractorRuntimePaths]) -> Path:
    root = get_runtime_paths().runs_dir
    root.mkdir(parents=True, exist_ok=True)
    return root


def project_runs_dir(get_runtime_paths: Callable[[], AttractorRuntimePaths], project_path: str) -> Optional[Path]:
    normalized_project_path = normalize_project_path(project_path)
    if not normalized_project_path:
        return None
    project_id = build_project_id(normalized_project_path)
    return runs_root(get_runtime_paths) / project_id


def iter_run_roots(
    get_runtime_paths: Callable[[], AttractorRuntimePaths],
    *,
    project_path: Optional[str] = None,
) -> list[Path]:
    if project_path:
        run_dir = project_runs_dir(get_runtime_paths, project_path)
        if run_dir is None or not run_dir.exists():
            return []
        return sorted((path for path in run_dir.iterdir() if path.is_dir()), key=lambda item: item.name)

    run_roots: list[Path] = []
    projects_root = runs_root(get_runtime_paths)
    if not projects_root.exists():
        return run_roots
    for run_dir in sorted(projects_root.iterdir()):
        if not run_dir.is_dir():
            continue
        run_roots.extend(sorted((path for path in run_dir.iterdir() if path.is_dir()), key=lambda item: item.name))
    return run_roots


def find_run_root(get_runtime_paths: Callable[[], AttractorRuntimePaths], run_id: str) -> Optional[Path]:
    for run_root in iter_run_roots(get_runtime_paths):
        if run_root.name == run_id:
            return run_root
    return None


def ensure_run_root_for_project(
    get_runtime_paths: Callable[[], AttractorRuntimePaths],
    run_id: str,
    project_path: str,
) -> Path:
    normalized_project_path = normalize_project_path(project_path)
    if not normalized_project_path:
        raise ValueError("Run storage requires a project path.")
    run_root = project_runs_dir(get_runtime_paths, normalized_project_path) / run_id
    run_root.mkdir(parents=True, exist_ok=True)
    return run_root


def run_root(get_runtime_paths: Callable[[], AttractorRuntimePaths], run_id: str) -> Path:
    existing = find_run_root(get_runtime_paths, run_id)
    if existing is not None:
        return existing
    return get_runtime_paths().runtime_dir / "_missing-runs" / run_id


def resolve_start_node_id(graph) -> str:
    shape_starts = []
    for node in graph.nodes.values():
        shape_attr = node.attrs.get("shape")
        shape_value = str(shape_attr.value) if shape_attr is not None else ""
        if shape_value == "Mdiamond":
            shape_starts.append(node.node_id)

    candidates = shape_starts or [node_id for node_id in graph.nodes if node_id in {"start", "Start"}]
    if len(candidates) != 1:
        raise RuntimeError(f"Expected exactly one start node, found {len(candidates)}")
    return candidates[0]


def graph_attr_context_seed(graph) -> Dict[str, object]:
    seeded: Dict[str, object] = {}
    for key, attr in graph.graph_attrs.items():
        value = getattr(attr, "value", "")
        if hasattr(value, "raw"):
            value = value.raw
        seeded[f"graph.{key}"] = value
    seeded.setdefault("graph.goal", "")
    default_max_retries = resolve_default_max_retries_value(graph.graph_attrs, default=0)
    seeded[f"graph.{DEFAULT_MAX_RETRIES_KEY}"] = default_max_retries
    return seeded


def run_meta_path(get_runtime_paths: Callable[[], AttractorRuntimePaths], run_id: str) -> Path:
    return run_root(get_runtime_paths, run_id) / "run.json"


def write_run_meta(get_runtime_paths: Callable[[], AttractorRuntimePaths], record: RunRecord) -> None:
    try:
        if record.project_path or record.working_directory:
            ensure_run_root_for_project(
                get_runtime_paths,
                record.run_id,
                record.project_path or record.working_directory,
            )
        meta_path = run_meta_path(get_runtime_paths, record.run_id)
        meta_path.parent.mkdir(parents=True, exist_ok=True)
        with meta_path.open("w", encoding="utf-8") as handle:
            json.dump(record.to_dict(), handle, indent=2, sort_keys=True)
    except Exception:
        pass


def read_run_meta(path: Path) -> Optional[RunRecord]:
    try:
        payload = json.loads(path.read_text(encoding="utf-8"))
        return RunRecord.from_dict(payload)
    except Exception:
        return None


def resolve_project_git_branch(directory_path: Path) -> Optional[str]:
    try:
        completed = subprocess.run(
            ["git", "-C", str(directory_path), "rev-parse", "--abbrev-ref", "HEAD"],
            capture_output=True,
            text=True,
            check=True,
        )
    except (subprocess.CalledProcessError, FileNotFoundError):
        return None
    branch = completed.stdout.strip()
    return branch or None


def resolve_project_git_commit(directory_path: Path) -> Optional[str]:
    try:
        completed = subprocess.run(
            ["git", "-C", str(directory_path), "rev-parse", "HEAD"],
            capture_output=True,
            text=True,
            check=True,
        )
    except (subprocess.CalledProcessError, FileNotFoundError):
        return None
    commit = completed.stdout.strip()
    return commit or None


def resolve_run_project_git_metadata(
    working_directory: str,
    *,
    resolve_runtime_workspace_path: Callable[[str], str],
) -> tuple[str, Optional[str], Optional[str]]:
    normalized_working_dir = resolve_runtime_workspace_path(working_directory)
    try:
        completed = subprocess.run(
            ["git", "-C", normalized_working_dir, "rev-parse", "--show-toplevel"],
            capture_output=True,
            text=True,
            check=True,
        )
    except (subprocess.CalledProcessError, FileNotFoundError):
        return normalized_working_dir, None, None

    project_path = completed.stdout.strip() or normalized_working_dir
    project_directory = Path(project_path)
    return (
        project_path,
        resolve_project_git_branch(project_directory),
        resolve_project_git_commit(project_directory),
    )


def record_run_start(
    get_runtime_paths: Callable[[], AttractorRuntimePaths],
    run_history_lock: threading.Lock,
    *,
    run_id: str,
    flow_name: str,
    working_directory: str,
    model: str,
    resolve_runtime_workspace_path: Callable[[str], str],
    llm_provider: str = "codex",
    llm_profile: Optional[str] = None,
    reasoning_effort: Optional[str] = None,
    spec_id: Optional[str] = None,
    plan_id: Optional[str] = None,
    continued_from_run_id: Optional[str] = None,
    continued_from_node: Optional[str] = None,
    continued_from_flow_mode: Optional[str] = None,
    continued_from_flow_name: Optional[str] = None,
    parent_run_id: Optional[str] = None,
    parent_node_id: Optional[str] = None,
    root_run_id: Optional[str] = None,
    child_invocation_index: Optional[int] = None,
    execution_mode: str = "native",
    execution_container_image: Optional[str] = None,
) -> None:
    project_path, git_branch, git_commit = resolve_run_project_git_metadata(
        working_directory,
        resolve_runtime_workspace_path=resolve_runtime_workspace_path,
    )
    record = RunRecord(
        run_id=run_id,
        flow_name=flow_name,
        status="running",
        outcome=None,
        outcome_reason_code=None,
        outcome_reason_message=None,
        working_directory=working_directory,
        model=model,
        llm_provider=llm_provider,
        llm_profile=llm_profile,
        reasoning_effort=reasoning_effort,
        started_at=datetime.now(UTC).isoformat(timespec="seconds").replace("+00:00", "Z"),
        project_path=project_path,
        git_branch=git_branch,
        git_commit=git_commit,
        spec_id=spec_id,
        plan_id=plan_id,
        continued_from_run_id=continued_from_run_id,
        continued_from_node=continued_from_node,
        continued_from_flow_mode=continued_from_flow_mode,
        continued_from_flow_name=continued_from_flow_name,
        parent_run_id=parent_run_id,
        parent_node_id=parent_node_id,
        root_run_id=root_run_id,
        child_invocation_index=child_invocation_index,
        execution_mode=execution_mode,
        execution_container_image=execution_container_image,
    )
    with run_history_lock:
        write_run_meta(get_runtime_paths, record)


def ensure_known_pipeline(
    get_runtime_paths: Callable[[], AttractorRuntimePaths],
    active_run: object | None,
    pipeline_id: str,
) -> None:
    if not active_run and not read_run_meta(run_meta_path(get_runtime_paths, pipeline_id)):
        raise HTTPException(status_code=404, detail="Unknown pipeline")


def artifact_media_type(path: Path) -> str:
    guessed, _ = mimetypes.guess_type(path.name)
    return guessed or "application/octet-stream"


def artifact_is_viewable(*, media_type: str, path: Path) -> bool:
    if media_type.startswith("text/"):
        return True
    if media_type in {"application/json", "application/xml", "image/svg+xml"}:
        return True
    return path.suffix.lower() in {".json", ".txt", ".md", ".log", ".dot", ".yaml", ".yml", ".csv"}


def resolve_artifact_path(run_root: Path, artifact_path: str) -> Path:
    normalized = artifact_path.strip()
    if not normalized:
        raise HTTPException(status_code=400, detail="Invalid artifact path")

    candidate = Path(normalized)
    if candidate.is_absolute():
        raise HTTPException(status_code=400, detail="Invalid artifact path")

    resolved_run_root = run_root.resolve()
    resolved_target = (resolved_run_root / candidate).resolve()
    try:
        resolved_target.relative_to(resolved_run_root)
    except ValueError as exc:
        raise HTTPException(status_code=400, detail="Invalid artifact path") from exc
    return resolved_target


def list_run_output_artifacts(run_root: Path) -> List[Dict[str, object]]:
    files: Dict[str, Path] = {}

    def add_file(path: Path) -> None:
        if not path.is_file():
            return
        try:
            relative_path = path.relative_to(run_root).as_posix()
        except ValueError:
            return
        files[relative_path] = path

    add_file(run_root / "manifest.json")
    add_file(run_root / "checkpoint.json")
    add_file(run_root / "run.log")

    logs_root = run_root / "logs"
    if logs_root.exists():
        for file_path in logs_root.rglob("*"):
            add_file(file_path)
    if run_root.exists():
        for child in run_root.iterdir():
            if not child.is_dir() or child.name in {"artifacts", "logs"}:
                continue
            add_file(child / "prompt.md")
            add_file(child / "response.md")
            add_file(child / "status.json")

    artifacts_root = run_root / "artifacts"
    if artifacts_root.exists():
        for file_path in artifacts_root.rglob("*"):
            add_file(file_path)

    entries: List[Dict[str, object]] = []
    for relative_path in sorted(files):
        absolute_path = files[relative_path]
        media_type = artifact_media_type(absolute_path)
        entries.append(
            {
                "path": relative_path,
                "size_bytes": absolute_path.stat().st_size,
                "media_type": media_type,
                "viewable": artifact_is_viewable(media_type=media_type, path=absolute_path),
            }
        )
    return entries


def record_run_end(
    get_runtime_paths: Callable[[], AttractorRuntimePaths],
    run_history_lock: threading.Lock,
    *,
    run_id: str,
    working_directory: str,
    status: str,
    last_error: str = "",
    outcome: str | None = None,
    outcome_reason_code: str | None = None,
    outcome_reason_message: str | None = None,
) -> None:
    normalized_status = normalize_run_status(status)
    with run_history_lock:
        record = read_run_meta(run_meta_path(get_runtime_paths, run_id))
        if not record:
            record = RunRecord(
                run_id=run_id,
                flow_name="",
                status=normalized_status,
                outcome=outcome,
                outcome_reason_code=outcome_reason_code,
                outcome_reason_message=outcome_reason_message,
                working_directory=working_directory,
                model="",
                started_at="",
                project_path=working_directory,
            )
        record.status = normalized_status
        record.outcome = outcome
        record.outcome_reason_code = outcome_reason_code
        record.outcome_reason_message = outcome_reason_message
        record.ended_at = datetime.now(UTC).isoformat(timespec="seconds").replace("+00:00", "Z")
        record.last_error = last_error
        if record.token_usage_breakdown is not None:
            record.token_usage = record.token_usage_breakdown.total_tokens
            record.estimated_model_cost = estimate_model_cost(record.token_usage_breakdown)
        elif record.token_usage is None:
            record.token_usage = extract_token_usage(run_root(get_runtime_paths, run_id), run_id)
        write_run_meta(get_runtime_paths, record)


def record_run_status(
    get_runtime_paths: Callable[[], AttractorRuntimePaths],
    run_history_lock: threading.Lock,
    *,
    run_id: str,
    status: str,
    last_error: str = "",
    outcome: str | None = None,
    outcome_reason_code: str | None = None,
    outcome_reason_message: str | None = None,
) -> None:
    normalized_status = normalize_run_status(status)
    with run_history_lock:
        record = read_run_meta(run_meta_path(get_runtime_paths, run_id))
        if not record:
            return
        record.status = normalized_status
        if outcome is not None or normalized_status != "running":
            record.outcome = outcome
        if outcome_reason_code is not None or normalized_status != "running":
            record.outcome_reason_code = outcome_reason_code
        if outcome_reason_message is not None or normalized_status != "running":
            record.outcome_reason_message = outcome_reason_message
        if last_error:
            record.last_error = last_error
        write_run_meta(get_runtime_paths, record)


def record_run_usage(
    get_runtime_paths: Callable[[], AttractorRuntimePaths],
    run_history_lock: threading.Lock,
    *,
    run_id: str,
    token_usage_breakdown: TokenUsageBreakdown,
) -> None:
    with run_history_lock:
        record = read_run_meta(run_meta_path(get_runtime_paths, run_id))
        if not record:
            return
        record.token_usage_breakdown = token_usage_breakdown.copy()
        record.token_usage = token_usage_breakdown.total_tokens
        record.estimated_model_cost = estimate_model_cost(token_usage_breakdown)
        write_run_meta(get_runtime_paths, record)


def append_run_log(get_runtime_paths: Callable[[], AttractorRuntimePaths], run_id: str, message: str) -> None:
    log_path = run_root(get_runtime_paths, run_id) / "run.log"
    try:
        log_path.parent.mkdir(parents=True, exist_ok=True)
        timestamp = datetime.now(UTC).strftime("%Y-%m-%d %H:%M:%S")
        with log_path.open("a", encoding="utf-8") as handle:
            handle.write(f"[{timestamp} UTC] {message}\n")
    except Exception:
        pass


def read_checkpoint_progress(
    get_runtime_paths: Callable[[], AttractorRuntimePaths],
    run_id: str,
) -> tuple[str, List[str]]:
    checkpoint = load_checkpoint(run_root(get_runtime_paths, run_id) / "state.json")
    if checkpoint is None:
        return "", []
    return checkpoint.current_node, list(checkpoint.completed_nodes)


def pipeline_progress_payload(current_node: str, completed_nodes: List[str]) -> Dict[str, object]:
    return {
        "current_node": current_node,
        "completed_count": len(completed_nodes),
    }


def read_pipeline_stage_response(
    get_runtime_paths: Callable[[], AttractorRuntimePaths],
    run_id: str,
    stage_id: str,
) -> str:
    candidate_paths = [
        run_root(get_runtime_paths, run_id) / "logs" / stage_id / "response.md",
        run_root(get_runtime_paths, run_id) / stage_id / "response.md",
    ]
    for path in candidate_paths:
        if not path.exists():
            continue
        text = path.read_text(encoding="utf-8")
        if text.strip():
            return text
    raise RuntimeError(f"Run {run_id} completed without a response artifact for stage {stage_id}.")


def record_run_plan_id(
    get_runtime_paths: Callable[[], AttractorRuntimePaths],
    run_history_lock: threading.Lock,
    run_id: str,
    plan_id: str,
) -> None:
    with run_history_lock:
        record = read_run_meta(run_meta_path(get_runtime_paths, run_id))
        if record is None:
            return
        record.plan_id = plan_id
        write_run_meta(get_runtime_paths, record)
