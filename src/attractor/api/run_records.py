from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path
import re
from typing import Dict, List, Optional

from attractor.api.token_usage import (
    EstimatedModelCost,
    TokenUsageBreakdown,
    estimate_model_cost,
)


@dataclass
class RunRecord:
    run_id: str
    flow_name: str
    status: str
    outcome: Optional[str]
    outcome_reason_code: Optional[str]
    outcome_reason_message: Optional[str]
    working_directory: str
    model: str
    started_at: str
    llm_provider: str = ""
    llm_profile: Optional[str] = None
    reasoning_effort: Optional[str] = None
    ended_at: Optional[str] = None
    project_path: str = ""
    git_branch: Optional[str] = None
    git_commit: Optional[str] = None
    spec_id: Optional[str] = None
    plan_id: Optional[str] = None
    continued_from_run_id: Optional[str] = None
    continued_from_node: Optional[str] = None
    continued_from_flow_mode: Optional[str] = None
    continued_from_flow_name: Optional[str] = None
    parent_run_id: Optional[str] = None
    parent_node_id: Optional[str] = None
    root_run_id: Optional[str] = None
    child_invocation_index: Optional[int] = None
    execution_mode: str = "native"
    execution_container_image: Optional[str] = None
    last_error: str = ""
    token_usage: Optional[int] = None
    token_usage_breakdown: Optional[TokenUsageBreakdown] = None
    estimated_model_cost: Optional[EstimatedModelCost] = None

    def to_dict(self) -> Dict[str, object]:
        provider = self.llm_provider or "codex"
        if self.llm_profile and provider == self.llm_profile:
            provider = "codex"
        payload = {
            "run_id": self.run_id,
            "flow_name": self.flow_name,
            "status": normalize_run_status(self.status),
            "outcome": self.outcome,
            "outcome_reason_code": self.outcome_reason_code,
            "outcome_reason_message": self.outcome_reason_message,
            "working_directory": self.working_directory,
            "model": self.model,
            "provider": provider,
            "llm_provider": provider,
            "llm_profile": self.llm_profile,
            "reasoning_effort": self.reasoning_effort,
            "started_at": self.started_at,
            "ended_at": self.ended_at,
            "project_path": self.project_path,
            "git_branch": self.git_branch,
            "git_commit": self.git_commit,
            "spec_id": self.spec_id,
            "plan_id": self.plan_id,
            "continued_from_run_id": self.continued_from_run_id,
            "continued_from_node": self.continued_from_node,
            "continued_from_flow_mode": self.continued_from_flow_mode,
            "continued_from_flow_name": self.continued_from_flow_name,
            "parent_run_id": self.parent_run_id,
            "parent_node_id": self.parent_node_id,
            "root_run_id": self.root_run_id,
            "child_invocation_index": self.child_invocation_index,
            "last_error": self.last_error,
            "token_usage": self.token_usage,
            "token_usage_breakdown": (
                self.token_usage_breakdown.to_dict()
                if self.token_usage_breakdown is not None
                else None
            ),
            "estimated_model_cost": (
                self.estimated_model_cost.to_dict()
                if self.estimated_model_cost is not None
                else None
            ),
        }
        if self.execution_mode and self.execution_mode != "native":
            payload["execution_mode"] = self.execution_mode
        if self.execution_container_image:
            payload["execution_container_image"] = self.execution_container_image
        return payload

    @classmethod
    def from_dict(cls, data: Dict[str, object]) -> "RunRecord":
        token_usage_breakdown = TokenUsageBreakdown.from_dict(
            data.get("token_usage_breakdown") if isinstance(data.get("token_usage_breakdown"), dict) else None
        )
        estimated_model_cost = EstimatedModelCost.from_dict(
            data.get("estimated_model_cost") if isinstance(data.get("estimated_model_cost"), dict) else None
        )
        if estimated_model_cost is None and token_usage_breakdown is not None:
            estimated_model_cost = estimate_model_cost(token_usage_breakdown)
        return cls(
            run_id=str(data.get("run_id", "")),
            flow_name=str(data.get("flow_name", "")),
            status=normalize_run_status(str(data.get("status", "unknown"))),
            outcome=data.get("outcome") if data.get("outcome") is not None else None,
            outcome_reason_code=(
                data.get("outcome_reason_code") if data.get("outcome_reason_code") is not None else None
            ),
            outcome_reason_message=(
                data.get("outcome_reason_message") if data.get("outcome_reason_message") is not None else None
            ),
            working_directory=str(data.get("working_directory", "")),
            model=str(data.get("model", "")),
            started_at=str(data.get("started_at", "")),
            llm_provider=str(data.get("llm_provider") or data.get("provider") or ""),
            llm_profile=str(data.get("llm_profile")) if data.get("llm_profile") is not None else None,
            reasoning_effort=(
                str(data.get("reasoning_effort")) if data.get("reasoning_effort") is not None else None
            ),
            ended_at=data.get("ended_at") if data.get("ended_at") is not None else None,
            project_path=str(data.get("project_path", "")),
            git_branch=str(data.get("git_branch")) if data.get("git_branch") is not None else None,
            git_commit=str(data.get("git_commit")) if data.get("git_commit") is not None else None,
            spec_id=str(data.get("spec_id")) if data.get("spec_id") is not None else None,
            plan_id=str(data.get("plan_id")) if data.get("plan_id") is not None else None,
            continued_from_run_id=(
                str(data.get("continued_from_run_id")) if data.get("continued_from_run_id") is not None else None
            ),
            continued_from_node=(
                str(data.get("continued_from_node")) if data.get("continued_from_node") is not None else None
            ),
            continued_from_flow_mode=(
                str(data.get("continued_from_flow_mode")) if data.get("continued_from_flow_mode") is not None else None
            ),
            continued_from_flow_name=(
                str(data.get("continued_from_flow_name")) if data.get("continued_from_flow_name") is not None else None
            ),
            parent_run_id=(
                str(data.get("parent_run_id")) if data.get("parent_run_id") is not None else None
            ),
            parent_node_id=(
                str(data.get("parent_node_id")) if data.get("parent_node_id") is not None else None
            ),
            root_run_id=(
                str(data.get("root_run_id")) if data.get("root_run_id") is not None else None
            ),
            child_invocation_index=(
                int(data["child_invocation_index"])
                if data.get("child_invocation_index") is not None
                else None
            ),
            execution_mode=str(data.get("execution_mode") or "native"),
            execution_container_image=(
                str(data.get("execution_container_image"))
                if data.get("execution_container_image") is not None
                else None
            ),
            last_error=str(data.get("last_error", "")),
            token_usage=(
                int(data["token_usage"])
                if data.get("token_usage") is not None
                else token_usage_breakdown.total_tokens if token_usage_breakdown is not None else None
            ),
            token_usage_breakdown=token_usage_breakdown,
            estimated_model_cost=estimated_model_cost,
        )


def normalize_run_status(status: str) -> str:
    if status == "success":
        return "completed"
    if status == "fail":
        return "failed"
    if status in {"aborted", "abort_requested"}:
        return {"aborted": "canceled", "abort_requested": "cancel_requested"}[status]
    if status == "cancelled":
        return "canceled"
    return status


def normalize_scope_path(value: str) -> str:
    trimmed = value.strip()
    if not trimmed:
        return ""
    slash_normalized = re.sub(r"/{2,}", "/", trimmed.replace("\\", "/"))
    prefix = "/" if slash_normalized.startswith("/") else ""
    raw_body = slash_normalized[1:] if prefix else slash_normalized
    parts = [part for part in raw_body.split("/") if part]
    segments: List[str] = []
    for part in parts:
        if part == ".":
            continue
        if part == "..":
            if segments:
                segments.pop()
            continue
        segments.append(part)
    normalized_body = "/".join(segments)
    if not normalized_body and prefix:
        return prefix
    return f"{prefix}{normalized_body}"


def path_in_scope(candidate_path: str, project_scope_path: str) -> bool:
    if not candidate_path or not project_scope_path:
        return False
    if candidate_path == project_scope_path:
        return True
    if project_scope_path == "/":
        return candidate_path.startswith("/")
    return candidate_path.startswith(f"{project_scope_path}/")


def run_matches_project_scope(record: RunRecord, project_path: str) -> bool:
    normalized_scope = normalize_scope_path(project_path)
    if not normalized_scope:
        return True
    candidate_paths = [
        normalize_scope_path(record.project_path),
        normalize_scope_path(record.working_directory),
    ]
    return any(path_in_scope(candidate_path, normalized_scope) for candidate_path in candidate_paths)


TOKEN_LINE_RE = re.compile(r"tokens used\s*[:=]?\s*(\d[\d,]*)", re.IGNORECASE)
TOKEN_NUMBER_ONLY_RE = re.compile(r"^\d[\d,]*$")
RUN_LOG_TIMESTAMP_RE = re.compile(r"^\[(\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}) UTC\]")


def extract_token_usage(run_root: Path, run_id: str) -> Optional[int]:
    run_log_path = run_root / "run.log"
    if not run_log_path.exists():
        return None
    try:
        lines = run_log_path.read_text(encoding="utf-8", errors="ignore").splitlines()
    except Exception:
        return None
    total = 0
    index = 0
    while index < len(lines):
        line = lines[index].strip()
        match = TOKEN_LINE_RE.search(line)
        if match:
            total += int(match.group(1).replace(",", ""))
        elif line.lower() == "tokens used" and index + 1 < len(lines):
            next_line = lines[index + 1].strip()
            if TOKEN_NUMBER_ONLY_RE.match(next_line):
                total += int(next_line.replace(",", ""))
                index += 1
        index += 1
    return total if total > 0 else None


def hydrate_run_record_from_log(record: RunRecord, run_root: Path) -> None:
    run_log_path = run_root / "run.log"
    if not run_log_path.exists():
        return
    if record.token_usage_breakdown is not None:
        record.token_usage = record.token_usage_breakdown.total_tokens
        record.estimated_model_cost = estimate_model_cost(record.token_usage_breakdown)
    elif record.token_usage is None:
        record.token_usage = extract_token_usage(run_root, record.run_id)
    try:
        lines = run_log_path.read_text(encoding="utf-8", errors="ignore").splitlines()
    except Exception:
        return
    if not lines:
        return

    first_timestamp = RUN_LOG_TIMESTAMP_RE.search(lines[0])
    if first_timestamp and not record.started_at:
        record.started_at = f"{first_timestamp.group(1).replace(' ', 'T')}Z"

    log_status = None
    for line in reversed(lines):
        status_match = re.search(r"Pipeline\s+(\w+)", line)
        if status_match:
            log_status = normalize_run_status(status_match.group(1))
            break
        if "Pipeline Aborted" in line:
            log_status = "canceled"
            break

    if log_status and record.status in {"", "unknown", "running"}:
        record.status = log_status
    if log_status and not record.ended_at:
        last_timestamp = RUN_LOG_TIMESTAMP_RE.search(lines[-1])
        if last_timestamp:
            record.ended_at = f"{last_timestamp.group(1).replace(' ', 'T')}Z"
    if not log_status and record.status == "unknown":
        record.status = "running"
