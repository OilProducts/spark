from __future__ import annotations

from dataclasses import asdict, dataclass
import hashlib
import json
from pathlib import Path
import re
import tomllib
from typing import Any, Iterable, Mapping


JsonMap = dict[str, Any]
HTTP_OBSERVED_RESPONSE_HEADERS = (
    "content-type",
    "cache-control",
    "connection",
    "x-spark-flow-name",
)


@dataclass(frozen=True)
class ProcessObservation:
    argv: tuple[str, ...]
    cwd: str
    returncode: int
    stdout: str
    stderr: str
    environment: Mapping[str, str]

    def to_dict(self) -> JsonMap:
        return asdict(self)


def observe_process(
    *,
    argv: Iterable[str],
    cwd: Path | str,
    returncode: int,
    stdout: str = "",
    stderr: str = "",
    environment: Mapping[str, str] | None = None,
) -> ProcessObservation:
    return ProcessObservation(
        argv=tuple(str(part) for part in argv),
        cwd=str(Path(cwd).resolve(strict=False)),
        returncode=returncode,
        stdout=stdout,
        stderr=stderr,
        environment=dict(environment or {}),
    )


def assert_process_observation(
    observation: ProcessObservation | Mapping[str, Any],
    *,
    returncode: int | None = None,
    stdout_contains: str | None = None,
    stderr_contains: str | None = None,
    env_keys: Iterable[str] = (),
) -> None:
    payload = _observation_dict(observation)
    if returncode is not None:
        assert payload["returncode"] == returncode
    if stdout_contains is not None:
        assert stdout_contains in payload.get("stdout", "")
    if stderr_contains is not None:
        assert stderr_contains in payload.get("stderr", "")
    environment = payload.get("environment", {})
    for key in env_keys:
        assert key in environment


def assert_process_matches_golden(
    actual: Mapping[str, Any],
    expected: Mapping[str, Any],
    *,
    compare_argv: bool = True,
) -> None:
    actual_process = normalize_dynamic_values(actual["process"])
    expected_process = normalize_dynamic_values(expected["process"])
    assert actual_process == expected_process
    if compare_argv:
        actual_argv = actual["command"].get("normalized_argv", actual["command"]["argv"])
        expected_argv = expected["command"].get("normalized_argv", expected["command"]["argv"])
        assert normalize_dynamic_values(actual_argv) == normalize_dynamic_values(
            expected_argv
        )
    assert actual["fixture_id"] == expected["fixture_id"]
    assert actual["item_id"] == expected["item_id"]
    validate_manifest_coverage(
        actual,
        requirement_ids=expected.get("requirements", ()),
        decision_ids=expected.get("decisions", ()),
    )


def assert_filesystem_snapshot_matches_golden(
    actual: Mapping[str, Any],
    expected: Mapping[str, Any],
    snapshot_name: str,
) -> None:
    actual_snapshot = normalize_dynamic_values(actual["filesystem"]["after"][snapshot_name])
    expected_snapshot = normalize_dynamic_values(expected["filesystem"]["after"][snapshot_name])
    assert actual_snapshot == expected_snapshot


def assert_durable_state_matches_golden(
    actual: Mapping[str, Any],
    expected: Mapping[str, Any],
    state_name: str,
) -> None:
    actual_state = normalize_dynamic_values(actual["durable_state"]["after"][state_name])
    expected_state = normalize_dynamic_values(expected["durable_state"]["after"][state_name])
    assert actual_state == expected_state


def normalize_manifest_for_comparison(manifest: Mapping[str, Any]) -> JsonMap:
    payload = {
        "schema_version": manifest.get("schema_version"),
        "fixture_id": manifest.get("fixture_id"),
        "item_id": manifest.get("item_id"),
        "requirements": manifest.get("requirements", []),
        "decisions": manifest.get("decisions", []),
        "command": {
            "argv": manifest.get("command", {}).get(
                "normalized_argv",
                manifest.get("command", {}).get("argv", []),
            ),
        },
        "process": manifest.get("process", {}),
        "filesystem": manifest.get("filesystem", {}),
        "durable_state": manifest.get("durable_state", {}),
    }
    return normalize_dynamic_values(payload)


def assert_manifest_matches_golden(actual: Mapping[str, Any], expected: Mapping[str, Any]) -> None:
    assert normalize_manifest_for_comparison(actual) == normalize_manifest_for_comparison(expected)


def normalize_http_manifest_for_comparison(manifest: Mapping[str, Any]) -> JsonMap:
    payload = {
        "schema_version": manifest.get("schema_version"),
        "fixture_id": manifest.get("fixture_id"),
        "item_id": manifest.get("item_id"),
        "requirements": manifest.get("requirements", []),
        "decisions": manifest.get("decisions", []),
        "server": manifest.get("server", {}),
        "request": manifest.get("request", {}),
        "response": manifest.get("response", {}),
    }
    return normalize_dynamic_values(payload)


def assert_http_manifest_matches_golden(
    actual: Mapping[str, Any],
    expected: Mapping[str, Any],
) -> None:
    assert normalize_http_manifest_for_comparison(actual) == normalize_http_manifest_for_comparison(
        expected
    )
    validate_manifest_coverage(
        actual,
        requirement_ids=expected.get("requirements", ()),
        decision_ids=expected.get("decisions", ()),
    )


def normalize_sse_manifest_for_comparison(manifest: Mapping[str, Any]) -> JsonMap:
    payload = {
        "schema_version": manifest.get("schema_version"),
        "fixture_id": manifest.get("fixture_id"),
        "item_id": manifest.get("item_id"),
        "requirements": manifest.get("requirements", []),
        "decisions": manifest.get("decisions", []),
        "server": manifest.get("server", {}),
        "request": manifest.get("request", {}),
        "response": manifest.get("response", {}),
        "frames": manifest.get("frames", []),
    }
    return normalize_dynamic_values(payload)


def assert_sse_manifest_matches_golden(
    actual: Mapping[str, Any],
    expected: Mapping[str, Any],
) -> None:
    assert normalize_sse_manifest_for_comparison(actual) == normalize_sse_manifest_for_comparison(
        expected
    )
    validate_manifest_coverage(
        actual,
        requirement_ids=expected.get("requirements", ()),
        decision_ids=expected.get("decisions", ()),
    )


def normalize_dsl_manifest_for_comparison(manifest: Mapping[str, Any]) -> JsonMap:
    payload = {
        "schema_version": manifest.get("schema_version"),
        "fixture_id": manifest.get("fixture_id"),
        "item_id": manifest.get("item_id"),
        "requirements": manifest.get("requirements", []),
        "decisions": manifest.get("decisions", []),
        "operation": manifest.get("operation"),
        "input": manifest.get("input", {}),
        "observation": manifest.get("observation", {}),
    }
    return normalize_dynamic_values(payload)


def assert_dsl_manifest_matches_golden(
    actual: Mapping[str, Any],
    expected: Mapping[str, Any],
) -> None:
    assert normalize_dsl_manifest_for_comparison(actual) == normalize_dsl_manifest_for_comparison(
        expected
    )
    validate_manifest_coverage(
        actual,
        requirement_ids=expected.get("requirements", ()),
        decision_ids=expected.get("decisions", ()),
    )


def normalize_runtime_manifest_for_comparison(manifest: Mapping[str, Any]) -> JsonMap:
    payload = {
        "schema_version": manifest.get("schema_version"),
        "fixture_id": manifest.get("fixture_id"),
        "item_id": manifest.get("item_id"),
        "requirements": manifest.get("requirements", []),
        "decisions": manifest.get("decisions", []),
        "scenario": manifest.get("scenario"),
        "input": manifest.get("input", {}),
        "observation": manifest.get("observation", {}),
        "events": manifest.get("events", []),
        "durable_state": manifest.get("durable_state", {}),
        "filesystem": manifest.get("filesystem", {}),
        "api": manifest.get("api", {}),
    }
    return normalize_dynamic_values(payload)


def assert_runtime_manifest_matches_golden(
    actual: Mapping[str, Any],
    expected: Mapping[str, Any],
) -> None:
    assert normalize_runtime_manifest_for_comparison(
        actual
    ) == normalize_runtime_manifest_for_comparison(expected)
    validate_manifest_coverage(
        actual,
        requirement_ids=expected.get("requirements", ()),
        decision_ids=expected.get("decisions", ()),
    )


def normalize_frontend_manifest_for_comparison(manifest: Mapping[str, Any]) -> JsonMap:
    payload = {
        "schema_version": manifest.get("schema_version"),
        "fixture_id": manifest.get("fixture_id"),
        "item_id": manifest.get("item_id"),
        "requirements": manifest.get("requirements", []),
        "decisions": manifest.get("decisions", []),
        "provenance": manifest.get("provenance", {}),
        "scenario": manifest.get("scenario"),
        "input": manifest.get("input", {}),
        "observation": manifest.get("observation", {}),
    }
    return normalize_dynamic_values(payload)


def assert_frontend_manifest_matches_golden(
    actual: Mapping[str, Any],
    expected: Mapping[str, Any],
) -> None:
    assert normalize_frontend_manifest_for_comparison(
        actual
    ) == normalize_frontend_manifest_for_comparison(expected)
    validate_manifest_coverage(
        actual,
        requirement_ids=expected.get("requirements", ()),
        decision_ids=expected.get("decisions", ()),
    )


def normalize_packaging_manifest_for_comparison(manifest: Mapping[str, Any]) -> JsonMap:
    payload = {
        "schema_version": manifest.get("schema_version"),
        "fixture_id": manifest.get("fixture_id"),
        "item_id": manifest.get("item_id"),
        "requirements": manifest.get("requirements", []),
        "decisions": manifest.get("decisions", []),
        "provenance": manifest.get("provenance", {}),
        "scenario": manifest.get("scenario"),
        "command": manifest.get("command", {}),
        "process": manifest.get("process", {}),
        "server": manifest.get("server", {}),
        "requests": manifest.get("requests", []),
        "artifacts": manifest.get("artifacts", {}),
        "resources": manifest.get("resources", {}),
        "observations": manifest.get("observations", {}),
        "skipped_prerequisite": manifest.get("skipped_prerequisite"),
    }
    return normalize_dynamic_values(payload)


def assert_packaging_manifest_matches_golden(
    actual: Mapping[str, Any],
    expected: Mapping[str, Any],
) -> None:
    assert normalize_packaging_manifest_for_comparison(
        actual
    ) == normalize_packaging_manifest_for_comparison(expected)
    validate_manifest_coverage(
        actual,
        requirement_ids=expected.get("requirements", ()),
        decision_ids=expected.get("decisions", ()),
    )


def normalize_agent_manifest_for_comparison(manifest: Mapping[str, Any]) -> JsonMap:
    payload = {
        "schema_version": manifest.get("schema_version"),
        "fixture_id": manifest.get("fixture_id"),
        "item_id": manifest.get("item_id"),
        "requirements": manifest.get("requirements", []),
        "decisions": manifest.get("decisions", []),
        "provenance": manifest.get("provenance", {}),
        "scenario": manifest.get("scenario"),
        "input": manifest.get("input", {}),
        "observation": manifest.get("observation", {}),
    }
    return normalize_dynamic_values(payload)


def assert_agent_manifest_matches_golden(
    actual: Mapping[str, Any],
    expected: Mapping[str, Any],
) -> None:
    assert normalize_agent_manifest_for_comparison(
        actual
    ) == normalize_agent_manifest_for_comparison(expected)
    validate_manifest_coverage(
        actual,
        requirement_ids=expected.get("requirements", ()),
        decision_ids=expected.get("decisions", ()),
    )


def normalize_provider_manifest_for_comparison(manifest: Mapping[str, Any]) -> JsonMap:
    payload = {
        "schema_version": manifest.get("schema_version"),
        "fixture_id": manifest.get("fixture_id"),
        "item_id": manifest.get("item_id"),
        "requirements": manifest.get("requirements", []),
        "decisions": manifest.get("decisions", []),
        "provenance": manifest.get("provenance", {}),
        "scenario": manifest.get("scenario"),
        "input": manifest.get("input", {}),
        "observation": manifest.get("observation", {}),
    }
    return normalize_dynamic_values(payload)


def assert_provider_manifest_matches_golden(
    actual: Mapping[str, Any],
    expected: Mapping[str, Any],
) -> None:
    assert normalize_provider_manifest_for_comparison(
        actual
    ) == normalize_provider_manifest_for_comparison(expected)
    validate_manifest_coverage(
        actual,
        requirement_ids=expected.get("requirements", ()),
        decision_ids=expected.get("decisions", ()),
    )


def dot_graph_payload(graph: Any) -> JsonMap:
    return {
        "graph_id": str(getattr(graph, "graph_id", "")),
        "graph_attrs": _attrs_payload(getattr(graph, "graph_attrs", {})),
        "defaults": {
            "node": _attrs_payload(getattr(getattr(graph, "defaults", None), "node", {})),
            "edge": _attrs_payload(getattr(getattr(graph, "defaults", None), "edge", {})),
        },
        "nodes": {
            str(node_id): {
                "attrs": _attrs_payload(getattr(node, "attrs", {})),
                "explicit_attr_keys": sorted(str(key) for key in getattr(node, "explicit_attr_keys", set())),
            }
            for node_id, node in sorted(getattr(graph, "nodes", {}).items())
        },
        "edges": [
            {
                "source": str(getattr(edge, "source", "")),
                "target": str(getattr(edge, "target", "")),
                "attrs": _attrs_payload(getattr(edge, "attrs", {})),
            }
            for edge in getattr(graph, "edges", [])
        ],
        "subgraphs": [_subgraph_payload(subgraph) for subgraph in getattr(graph, "subgraphs", [])],
    }


def diagnostics_payload(diagnostics: Iterable[Any]) -> list[JsonMap]:
    payloads: list[JsonMap] = []
    for diagnostic in diagnostics:
        payload: JsonMap = {
            "rule": str(getattr(diagnostic, "rule_id", getattr(diagnostic, "rule", ""))),
            "severity": str(getattr(getattr(diagnostic, "severity", ""), "value", getattr(diagnostic, "severity", ""))),
            "message": str(getattr(diagnostic, "message", "")),
            "line": int(getattr(diagnostic, "line", 0) or 0),
            "node": getattr(diagnostic, "node_id", getattr(diagnostic, "node", None)),
        }
        edge = getattr(diagnostic, "edge", None)
        if edge is not None:
            payload["edge"] = list(edge)
        fix = getattr(diagnostic, "fix", None)
        if fix is not None:
            payload["fix"] = fix
        payloads.append(payload)
    return payloads


def outcome_payload(outcome: Any) -> JsonMap:
    status = getattr(outcome, "status", "")
    payload: JsonMap = {
        "status": str(getattr(status, "value", status)),
        "preferred_label": str(getattr(outcome, "preferred_label", "") or ""),
        "suggested_next_ids": [str(value) for value in getattr(outcome, "suggested_next_ids", []) or []],
        "context_updates": dict(getattr(outcome, "context_updates", {}) or {}),
        "failure_reason": str(getattr(outcome, "failure_reason", "") or ""),
        "notes": str(getattr(outcome, "notes", "") or ""),
    }
    failure_kind = getattr(outcome, "failure_kind", None)
    if failure_kind is not None:
        payload["failure_kind"] = str(getattr(failure_kind, "value", failure_kind))
    retryable = getattr(outcome, "retryable", None)
    if retryable is not None:
        payload["retryable"] = bool(retryable)
    return payload


def pipeline_result_payload(result: Any) -> JsonMap:
    return {
        "status": str(getattr(result, "status", "")),
        "current_node": str(getattr(result, "current_node", "")),
        "completed_nodes": [str(value) for value in getattr(result, "completed_nodes", [])],
        "route_trace": [str(value) for value in getattr(result, "route_trace", [])],
        "failure_reason": str(getattr(result, "failure_reason", "") or ""),
        "outcome": getattr(result, "outcome", None),
        "outcome_reason_code": getattr(result, "outcome_reason_code", None),
        "outcome_reason_message": getattr(result, "outcome_reason_message", None),
        "context": dict(getattr(result, "context", {}) or {}),
        "node_outcomes": {
            str(node_id): outcome_payload(outcome)
            for node_id, outcome in sorted((getattr(result, "node_outcomes", {}) or {}).items())
        },
    }


def runtime_directory_snapshot(
    root: Path | str,
    *,
    parse_json: Iterable[str] = (),
    parse_jsonl: Iterable[str] = (),
    read_text: Iterable[str] = (),
) -> JsonMap:
    root_path = Path(root)
    parsed: JsonMap = {}
    for relative in sorted(set(parse_json)):
        path = root_path / relative
        parsed[relative] = _read_json_file(path)
    for relative in sorted(set(parse_jsonl)):
        path = root_path / relative
        parsed[relative] = _read_jsonl_file(path)
    for relative in sorted(set(read_text)):
        path = root_path / relative
        parsed[relative] = {
            "exists": path.exists(),
            "text": path.read_text(encoding="utf-8") if path.exists() else "",
        }
    return {
        "root": str(root_path.resolve(strict=False)),
        "tree": filesystem_snapshot(root_path),
        "parsed": parsed,
    }


def selected_http_headers(headers: Mapping[str, str]) -> JsonMap:
    lowered = {str(key).lower(): str(value) for key, value in headers.items()}
    return {
        key: lowered[key]
        for key in HTTP_OBSERVED_RESPONSE_HEADERS
        if key in lowered
    }


def http_body_observation(
    *,
    content: bytes,
    headers: Mapping[str, str],
) -> JsonMap:
    content_type = str(headers.get("content-type") or headers.get("Content-Type") or "")
    if "application/json" in content_type:
        return {
            "kind": "json",
            "json": json.loads(content.decode("utf-8")) if content else None,
        }
    if (
        content_type.startswith("text/")
        or "text/vnd.graphviz" in content_type
        or "application/javascript" in content_type
        or "application/xml" in content_type
        or "text/html" in content_type
    ):
        return {"kind": "text", "text": content.decode("utf-8")}
    return {
        "kind": "bytes",
        "size": len(content),
        "sha256": hashlib.sha256(content).hexdigest(),
    }


def http_response_observation(
    *,
    method: str,
    path: str,
    status_code: int,
    headers: Mapping[str, str] | None = None,
    body: Any = None,
) -> JsonMap:
    return {
        "request": {"method": method.upper(), "path": path},
        "response": {
            "status_code": status_code,
            "headers": dict(headers or {}),
            "body": body,
        },
    }


def assert_http_response(
    observation: Mapping[str, Any],
    *,
    status_code: int | None = None,
    header: tuple[str, str] | None = None,
    body_key: str | None = None,
) -> None:
    response = dict(observation["response"])
    if status_code is not None:
        assert response["status_code"] == status_code
    if header is not None:
        key, expected_value = header
        assert response.get("headers", {}).get(key) == expected_value
    if body_key is not None:
        body = response.get("body")
        assert isinstance(body, Mapping)
        assert body_key in body


def sse_envelope(
    *,
    event: str,
    data: Mapping[str, Any],
    event_id: str | None = None,
    retry: int | None = None,
) -> JsonMap:
    envelope: JsonMap = {"event": event, "data": dict(data)}
    if event_id is not None:
        envelope["id"] = event_id
    if retry is not None:
        envelope["retry"] = retry
    return envelope


def parse_sse_frames(raw_frames: Iterable[str]) -> list[JsonMap]:
    frames: list[JsonMap] = []
    for raw_frame in raw_frames:
        frame = raw_frame.strip("\r\n")
        if not frame:
            continue
        event_name = "message"
        event_id: str | None = None
        retry: int | None = None
        data_lines: list[str] = []
        comments: list[str] = []
        for raw_line in frame.splitlines():
            line = raw_line.rstrip("\r")
            if line.startswith(":"):
                comments.append(line[1:].strip())
                continue
            field, separator, value = line.partition(":")
            if separator:
                value = value[1:] if value.startswith(" ") else value
            if field == "event":
                event_name = value
            elif field == "id":
                event_id = value
            elif field == "retry":
                try:
                    retry = int(value)
                except ValueError:
                    retry = None
            elif field == "data":
                data_lines.append(value)
        if comments and not data_lines:
            frames.append({"kind": "comment", "comments": comments})
            continue
        data_text = "\n".join(data_lines)
        parsed_data: Any
        try:
            parsed_data = json.loads(data_text) if data_text else None
        except json.JSONDecodeError:
            parsed_data = None
        frame_payload: JsonMap = {
            "kind": "event",
            "event": event_name,
            "data_text": data_text,
        }
        if parsed_data is not None:
            frame_payload["data"] = parsed_data
        if event_id is not None:
            frame_payload["id"] = event_id
        if retry is not None:
            frame_payload["retry"] = retry
        frames.append(frame_payload)
    return frames


def assert_sse_envelope(
    envelope: Mapping[str, Any],
    *,
    event: str | None = None,
    data_keys: Iterable[str] = (),
) -> None:
    if event is not None:
        assert envelope["event"] == event
    data = envelope.get("data")
    assert isinstance(data, Mapping)
    for key in data_keys:
        assert key in data


def filesystem_snapshot(root: Path | str) -> JsonMap:
    root_path = Path(root)
    entries: list[JsonMap] = []
    if root_path.exists():
        for path in sorted(root_path.rglob("*")):
            relative = path.relative_to(root_path).as_posix()
            if path.is_dir():
                entries.append({"path": relative, "kind": "dir"})
            elif path.is_file():
                payload = path.read_bytes()
                entries.append(
                    {
                        "path": relative,
                        "kind": "file",
                        "size": len(payload),
                        "sha256": hashlib.sha256(payload).hexdigest(),
                    }
                )
    return {
        "root": str(root_path.resolve(strict=False)),
        "entries": entries,
    }


def assert_filesystem_effect(
    before: Mapping[str, Any],
    after: Mapping[str, Any],
    *,
    created: Iterable[str] = (),
    removed: Iterable[str] = (),
    changed: Iterable[str] = (),
) -> None:
    before_entries = _entries_by_path(before)
    after_entries = _entries_by_path(after)
    for path in created:
        assert path not in before_entries
        assert path in after_entries
    for path in removed:
        assert path in before_entries
        assert path not in after_entries
    for path in changed:
        assert before_entries[path]["sha256"] != after_entries[path]["sha256"]


def durable_json_snapshot(path: Path | str) -> JsonMap:
    target = Path(path)
    return {
        "path": str(target.resolve(strict=False)),
        "format": "json",
        "data": json.loads(target.read_text(encoding="utf-8")),
    }


def durable_jsonl_snapshot(path: Path | str) -> JsonMap:
    target = Path(path)
    rows = [
        json.loads(line)
        for line in target.read_text(encoding="utf-8").splitlines()
        if line.strip()
    ]
    return {
        "path": str(target.resolve(strict=False)),
        "format": "jsonl",
        "records": rows,
    }


def durable_toml_snapshot(path: Path | str) -> JsonMap:
    target = Path(path)
    return {
        "path": str(target.resolve(strict=False)),
        "format": "toml",
        "data": tomllib.loads(target.read_text(encoding="utf-8")),
    }


def durable_state_snapshot(path: Path | str) -> JsonMap:
    target = Path(path)
    suffix = target.suffix.lower()
    if suffix == ".json":
        return durable_json_snapshot(target)
    if suffix == ".jsonl":
        return durable_jsonl_snapshot(target)
    if suffix == ".toml":
        return durable_toml_snapshot(target)
    raise ValueError(f"unsupported durable state format: {target}")


def normalize_path_tokens(value: Any, token_paths: Mapping[str, str | Path]) -> Any:
    replacements = {
        str(Path(path).resolve(strict=False)): token
        for token, path in token_paths.items()
        if str(path)
    }
    return _replace_strings(value, replacements)


def normalize_dynamic_values(value: Any) -> Any:
    normalized = _normalize_dynamic_value(value)
    return _replace_dynamic_text(normalized) if isinstance(normalized, str) else normalized


def write_manifest(path: Path | str, manifest: Mapping[str, Any]) -> None:
    target = Path(path)
    target.parent.mkdir(parents=True, exist_ok=True)
    target.write_text(json.dumps(dict(manifest), indent=2, sort_keys=True) + "\n", encoding="utf-8")


def load_manifest(path: Path | str) -> JsonMap:
    loaded = json.loads(Path(path).read_text(encoding="utf-8"))
    assert isinstance(loaded, dict)
    return loaded


def validate_manifest_coverage(
    manifest: Mapping[str, Any],
    *,
    requirement_ids: Iterable[str] = (),
    decision_ids: Iterable[str] = (),
) -> JsonMap:
    requirements = {str(value) for value in manifest.get("requirements", []) if str(value).strip()}
    decisions = {str(value) for value in manifest.get("decisions", []) if str(value).strip()}
    assert requirements, "manifest must declare at least one requirement"
    assert decisions, "manifest must declare at least one contract decision"

    missing_requirements = set(requirement_ids) - requirements
    missing_decisions = set(decision_ids) - decisions
    assert not missing_requirements, f"missing requirements: {sorted(missing_requirements)}"
    assert not missing_decisions, f"missing decisions: {sorted(missing_decisions)}"

    return {
        "requirements": sorted(requirements),
        "decisions": sorted(decisions),
    }


def fixture_manifest_path(root: Path | str, fixture_id: str) -> Path:
    return Path(root) / f"{fixture_id}.json"


def _attrs_payload(attrs: Mapping[str, Any]) -> JsonMap:
    return {
        str(key): {
            "value": _dot_value_payload(getattr(attr, "value", None)),
            "value_type": str(getattr(getattr(attr, "value_type", ""), "value", getattr(attr, "value_type", ""))),
        }
        for key, attr in sorted(attrs.items())
    }


def _dot_value_payload(value: Any) -> Any:
    if hasattr(value, "raw") and hasattr(value, "value") and hasattr(value, "unit"):
        return {
            "raw": str(getattr(value, "raw")),
            "value": getattr(value, "value"),
            "unit": str(getattr(value, "unit")),
        }
    return value


def _subgraph_payload(subgraph: Any) -> JsonMap:
    return {
        "id": getattr(subgraph, "id", None),
        "attrs": _attrs_payload(getattr(subgraph, "attrs", {})),
        "node_ids": [str(node_id) for node_id in getattr(subgraph, "node_ids", [])],
        "defaults": {
            "node": _attrs_payload(getattr(getattr(subgraph, "defaults", None), "node", {})),
            "edge": _attrs_payload(getattr(getattr(subgraph, "defaults", None), "edge", {})),
        },
        "subgraphs": [
            _subgraph_payload(child)
            for child in getattr(subgraph, "subgraphs", [])
        ],
    }


def _read_json_file(path: Path) -> JsonMap:
    if not path.exists():
        return {"exists": False}
    return {
        "exists": True,
        "data": json.loads(path.read_text(encoding="utf-8")),
    }


def _read_jsonl_file(path: Path) -> JsonMap:
    if not path.exists():
        return {"exists": False, "records": []}
    return {
        "exists": True,
        "records": [
            json.loads(line)
            for line in path.read_text(encoding="utf-8").splitlines()
            if line.strip()
        ],
    }


def _observation_dict(observation: ProcessObservation | Mapping[str, Any]) -> JsonMap:
    if isinstance(observation, ProcessObservation):
        return observation.to_dict()
    return dict(observation)


def _entries_by_path(snapshot: Mapping[str, Any]) -> dict[str, Mapping[str, Any]]:
    return {
        str(entry["path"]): entry
        for entry in snapshot.get("entries", [])
        if isinstance(entry, Mapping)
    }


def _replace_strings(value: Any, replacements: Mapping[str, str]) -> Any:
    if isinstance(value, str):
        normalized = value
        for raw, token in sorted(replacements.items(), key=lambda item: len(item[0]), reverse=True):
            normalized = normalized.replace(raw, token)
        return normalized
    if isinstance(value, list):
        return [_replace_strings(item, replacements) for item in value]
    if isinstance(value, tuple):
        return [_replace_strings(item, replacements) for item in value]
    if isinstance(value, dict):
        return {str(key): _replace_strings(child, replacements) for key, child in value.items()}
    return value


def _normalize_dynamic_value(value: Any, key: str | None = None) -> Any:
    if isinstance(value, dict):
        return {
            str(child_key): _normalize_dynamic_value(child, str(child_key))
            for child_key, child in value.items()
        }
    if isinstance(value, list):
        normalized_list: list[Any] = []
        previous: Any = None
        for item in value:
            if previous == "--conversation" and isinstance(item, str):
                normalized_list.append("__CONVERSATION_HANDLE__")
            else:
                normalized_list.append(_normalize_dynamic_value(item, key))
            previous = item
        return normalized_list
    if isinstance(value, tuple):
        return [_normalize_dynamic_value(item, key) for item in value]
    if isinstance(value, str):
        lower_key = key.lower() if key is not None else ""
        if "webhook-secret" in lower_key:
            return "__WEBHOOK_SECRET__" if value else value
        if "webhook-key" in lower_key:
            return "__WEBHOOK_KEY__" if value else value
        if key in {
            "created_at",
            "updated_at",
            "captured_at_utc",
            "completed_at",
            "finished_at",
            "last_opened_at",
            "last_accessed_at",
            "last_fired_at",
            "next_run_at",
            "started_at",
            "submitted_at",
            "timestamp",
        }:
            return "__TIMESTAMP__" if value else value
        if key in {"webhook_secret"}:
            return "__WEBHOOK_SECRET__" if value else value
        if key in {"webhook_key"}:
            return "__WEBHOOK_KEY__" if value else value
        if key in {"secret_hash"}:
            return "__SECRET_HASH__" if value else value
        if key in {"conversation_handle"}:
            return "__CONVERSATION_HANDLE__" if value else value
        if key in {"run_id", "pipeline_id"}:
            return "__RUN_ID__" if value else value
        if key in {"child_run_id"}:
            return "__CHILD_RUN_ID__" if value else value
        if key in {"wheel", "sdist", "artifact_name", "filename"}:
            return _replace_artifact_text(value)
        return _replace_dynamic_text(value)
    if key == "size" and isinstance(value, int):
        return "__SIZE__"
    if key in {"duration", "duration_seconds", "elapsed", "delay"} and isinstance(value, (int, float)):
        return "__DURATION__"
    return value


def _replace_dynamic_text(value: str) -> str:
    replacements = (
        (r"http://127\.0\.0\.1:\d+", "__API_BASE_URL__"),
        (r"\brun-[0-9a-f]{12,32}\b", "run-__ID__"),
        (r"\bpipeline-[0-9a-f]{12,32}\b", "pipeline-__ID__"),
        (r"\btrigger-[0-9a-f]{12}\b", "trigger-__ID__"),
        (r"\bflow-run-request-[0-9a-f]{12}\b", "flow-run-request-__ID__"),
        (r"\bsegment-artifact-flow-run-request-[0-9a-f]{12}\b", "segment-artifact-flow-run-request-__ID__"),
        (r"\bturn-[0-9a-f]{32}\b", "turn-__ID__"),
        (r"\bconversation-[0-9a-f]{12}\b", "conversation-__ID__"),
        (r"\b[a-z0-9-]+-[0-9a-f]{12}\b", "__PROJECT_ID__"),
        (r"\b[0-9a-f]{64}\b", "__SHA256__"),
        (r"\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?(?:Z|\+00:00)", "__TIMESTAMP__"),
    )
    normalized = value
    for pattern, replacement in replacements:
        normalized = re.sub(pattern, replacement, normalized)
    normalized = re.sub(
        r'("webhook_key":\s*")[^"]+(")',
        r"\1__WEBHOOK_KEY__\2",
        normalized,
    )
    normalized = re.sub(
        r'("webhook_secret":\s*")[^"]+(")',
        r"\1__WEBHOOK_SECRET__\2",
        normalized,
    )
    normalized = re.sub(
        r'("conversation_handle":\s*")[^"]+(")',
        r"\1__CONVERSATION_HANDLE__\2",
        normalized,
    )
    return normalized


def _replace_artifact_text(value: str) -> str:
    normalized = _replace_dynamic_text(value)
    normalized = re.sub(
        r"spark-[0-9][A-Za-z0-9_.!+-]*-[A-Za-z0-9_]+-[A-Za-z0-9_]+-[A-Za-z0-9_.]+\.whl",
        "spark-__VERSION__-__WHEEL_TAG__.whl",
        normalized,
    )
    normalized = re.sub(
        r"spark-[0-9][A-Za-z0-9_.!+-]*\.tar\.gz",
        "spark-__VERSION__.tar.gz",
        normalized,
    )
    normalized = re.sub(
        r"assets/[^/._-]+-[A-Za-z0-9_-]{8,}\.(js|css|png|svg|ico)",
        r"assets/__HASHED_ASSET__.\1",
        normalized,
    )
    normalized = re.sub(
        r"index-[A-Za-z0-9_-]{8,}\.(js|css)",
        r"index-__HASH__.\1",
        normalized,
    )
    return normalized
