#!/usr/bin/env python3
from __future__ import annotations

import argparse
from datetime import UTC, datetime
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tomllib
from typing import Any, Mapping


RELEVANT_ENV_NAMES = {
    "ATTRACTOR_CODEX_RUNTIME_ROOT",
    "ATTRACTOR_HOST_REPO_ROOT",
    "ATTRACTOR_RUNTIME_REPO_ROOT",
    "ATTRACTOR_CODEX_SEED_DIR",
    "CODEX_HOME",
    "SPARK_HOME",
    "SPARK_FLOWS_DIR",
    "SPARK_PORT",
}
RELEVANT_ENV_PREFIXES = (
    "ANTHROPIC_",
    "ATTRACTOR_",
    "AZURE_",
    "CODEX_",
    "GEMINI_",
    "GOOGLE_",
    "MISTRAL_",
    "OPENAI_",
    "SPARK_",
    "UNIFIED_LLM_",
)
SECRET_ENV_MARKERS = ("KEY", "TOKEN", "SECRET", "PASSWORD", "AUTH", "CREDENTIAL")


def main(argv: list[str] | None = None) -> int:
    args = _parse_args(argv)
    command = list(args.command)
    if command and command[0] == "--":
        command = command[1:]
    if not command:
        print("compat_capture.py: missing command after --", file=sys.stderr)
        return 2

    repo_root = Path(__file__).resolve().parents[1]
    configured_worktree = repo_root.resolve(strict=False)
    detected_worktree = _detect_git_worktree(Path.cwd())
    if detected_worktree != configured_worktree:
        print(
            "compat_capture.py: refusing to capture outside the configured rewrite "
            f"worktree (cwd worktree={detected_worktree}, expected={configured_worktree})",
            file=sys.stderr,
        )
        return 2

    validation_root = _validation_root(args.validation_root, repo_root=repo_root)
    fixture_slug = _fixture_slug(args.fixture_id)
    generated_root = validation_root / "generated" / fixture_slug
    paths = _capture_paths(args, generated_root)
    for path in paths.values():
        path.mkdir(parents=True, exist_ok=True)

    output_path = (
        Path(args.output).resolve(strict=False)
        if args.output
        else validation_root / "generated" / f"{fixture_slug}.json"
    )
    output_path.parent.mkdir(parents=True, exist_ok=True)

    extra_normalize_paths = _parse_named_paths(
        args.normalize_path,
        paths=paths,
        configured_worktree=configured_worktree,
        output_path=output_path,
    )
    replacements = _normalization_replacements(
        configured_worktree=configured_worktree,
        output_path=output_path,
        paths=paths,
        extra_paths=extra_normalize_paths,
    )
    snapshot_roots = _parse_named_paths(
        args.snapshot_root,
        paths=paths,
        configured_worktree=configured_worktree,
        output_path=output_path,
        extra_paths=extra_normalize_paths,
    )
    state_files = _parse_named_paths(
        args.state_file,
        paths=paths,
        configured_worktree=configured_worktree,
        output_path=output_path,
        extra_paths=extra_normalize_paths,
    )

    command_env = os.environ.copy()
    if not args.no_default_path_env:
        command_env.update(
            {
                "SPARK_HOME": str(paths["spark_home"]),
                "SPARK_FLOWS_DIR": str(paths["spark_flows_dir"]),
                "ATTRACTOR_CODEX_RUNTIME_ROOT": str(paths["attractor_codex_runtime_root"]),
                "CODEX_HOME": str(paths["codex_home"]),
            }
        )
    for key in args.unset_command_env:
        command_env.pop(key, None)
    for assignment in args.command_env:
        key, value = _parse_env_assignment(assignment)
        command_env[key] = value

    before_filesystem = _filesystem_snapshots(snapshot_roots, replacements)
    before_state = _state_snapshots(state_files, replacements)

    try:
        completed = subprocess.run(
            command,
            cwd=configured_worktree,
            env=command_env,
            text=True,
            capture_output=True,
            timeout=args.timeout,
            check=False,
        )
    except OSError as exc:
        manifest = _manifest(
            args=args,
            configured_worktree=configured_worktree,
            validation_root=validation_root,
            output_path=output_path,
            paths=paths,
            command=command,
            replacements=replacements,
            selected_env=_selected_environment(command_env),
            filesystem={"before": before_filesystem, "after": _filesystem_snapshots(snapshot_roots, replacements)},
            durable_state={"before": before_state, "after": _state_snapshots(state_files, replacements)},
            process={
                "returncode": None,
                "stdout": "",
                "stderr": str(exc),
                "error": exc.__class__.__name__,
            },
        )
        _write_json(output_path, manifest)
        print(f"compat_capture.py: failed to execute command: {exc}", file=sys.stderr)
        return 1
    except subprocess.TimeoutExpired as exc:
        stdout = exc.stdout if isinstance(exc.stdout, str) else ""
        stderr = exc.stderr if isinstance(exc.stderr, str) else ""
        manifest = _manifest(
            args=args,
            configured_worktree=configured_worktree,
            validation_root=validation_root,
            output_path=output_path,
            paths=paths,
            command=command,
            replacements=replacements,
            selected_env=_selected_environment(command_env),
            filesystem={"before": before_filesystem, "after": _filesystem_snapshots(snapshot_roots, replacements)},
            durable_state={"before": before_state, "after": _state_snapshots(state_files, replacements)},
            process={
                "returncode": None,
                "stdout": stdout,
                "stderr": stderr,
                "timeout_seconds": args.timeout,
                "error": "TimeoutExpired",
            },
        )
        _write_json(output_path, manifest)
        print(f"compat_capture.py: command timed out after {args.timeout}s", file=sys.stderr)
        return 1

    manifest = _manifest(
        args=args,
        configured_worktree=configured_worktree,
        validation_root=validation_root,
        output_path=output_path,
        paths=paths,
        command=command,
        replacements=replacements,
        selected_env=_selected_environment(command_env),
        filesystem={"before": before_filesystem, "after": _filesystem_snapshots(snapshot_roots, replacements)},
        durable_state={"before": before_state, "after": _state_snapshots(state_files, replacements)},
        process={
            "returncode": completed.returncode,
            "stdout": completed.stdout,
            "stderr": completed.stderr,
        },
    )
    _write_json(output_path, manifest)
    if args.fail_on_command_error and completed.returncode != 0:
        return completed.returncode
    return 0


def _parse_args(argv: list[str] | None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Capture observable Spark behavior from the isolated rewrite worktree.",
    )
    parser.add_argument("--fixture-id", required=True)
    parser.add_argument("--item-id", required=True)
    parser.add_argument("--requirement", action="append", default=[])
    parser.add_argument("--decision", action="append", default=[])
    parser.add_argument("--output")
    parser.add_argument("--spark-home")
    parser.add_argument("--flows-dir")
    parser.add_argument("--runtime-root")
    parser.add_argument("--codex-home")
    parser.add_argument(
        "--validation-root",
        help=(
            "Root for generated captures when --spark-home, --flows-dir, "
            "--runtime-root, --codex-home, or --output are omitted. Defaults "
            "to tests/compat/.tmp/capture-validation."
        ),
    )
    parser.add_argument("--timeout", type=float, default=60.0)
    parser.add_argument("--fail-on-command-error", action="store_true")
    parser.add_argument(
        "--command-env",
        action="append",
        default=[],
        metavar="KEY=VALUE",
        help="Set one environment variable only for the captured command.",
    )
    parser.add_argument(
        "--unset-command-env",
        action="append",
        default=[],
        metavar="KEY",
        help="Remove one inherited environment variable for the captured command.",
    )
    parser.add_argument(
        "--no-default-path-env",
        action="store_true",
        help="Do not set SPARK_HOME, SPARK_FLOWS_DIR, ATTRACTOR_CODEX_RUNTIME_ROOT, or CODEX_HOME automatically.",
    )
    parser.add_argument(
        "--snapshot-root",
        action="append",
        default=[],
        metavar="NAME=PATH",
        help="Record a normalized before/after filesystem snapshot for PATH.",
    )
    parser.add_argument(
        "--state-file",
        action="append",
        default=[],
        metavar="NAME=PATH",
        help="Record a normalized before/after JSON, TOML, or JSONL durable-state snapshot.",
    )
    parser.add_argument(
        "--normalize-path",
        action="append",
        default=[],
        metavar="TOKEN=PATH",
        help="Replace PATH with __TOKEN__ in captured process output and snapshots.",
    )
    parser.add_argument("command", nargs=argparse.REMAINDER)
    return parser.parse_args(argv)


def _write_json(path: Path, payload: dict[str, Any]) -> None:
    path.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def _validation_root(raw_root: str | None, *, repo_root: Path) -> Path:
    if raw_root:
        return Path(raw_root).expanduser().resolve(strict=False)
    return (repo_root / "tests" / "compat" / ".tmp" / "capture-validation").resolve(
        strict=False
    )


def _detect_git_worktree(cwd: Path) -> Path:
    result = subprocess.run(
        ["git", "rev-parse", "--show-toplevel"],
        cwd=cwd,
        text=True,
        capture_output=True,
        check=False,
    )
    if result.returncode == 0:
        return Path(result.stdout.strip()).resolve(strict=False)
    return cwd.resolve(strict=False)


def _fixture_slug(fixture_id: str) -> str:
    parts = [part for part in re.split(r"[\\/]+", fixture_id.strip()) if part]
    if not parts:
        raise ValueError("fixture id must contain at least one non-separator character")
    safe_parts = [re.sub(r"[^A-Za-z0-9_.-]+", "-", part).strip(".-") or "fixture" for part in parts]
    return "/".join(safe_parts)


def _capture_paths(args: argparse.Namespace, generated_root: Path) -> dict[str, Path]:
    return {
        "spark_home": (
            Path(args.spark_home).expanduser().resolve(strict=False)
            if args.spark_home
            else generated_root / "spark-home"
        ),
        "spark_flows_dir": (
            Path(args.flows_dir).expanduser().resolve(strict=False)
            if args.flows_dir
            else generated_root / "flows"
        ),
        "attractor_codex_runtime_root": (
            Path(args.runtime_root).expanduser().resolve(strict=False)
            if args.runtime_root
            else generated_root / "codex-runtime"
        ),
        "codex_home": (
            Path(args.codex_home).expanduser().resolve(strict=False)
            if args.codex_home
            else generated_root / "codex-home"
        ),
    }


def _parse_env_assignment(assignment: str) -> tuple[str, str]:
    key, separator, value = assignment.partition("=")
    if not separator or not key.strip():
        raise ValueError(f"environment assignment must be KEY=VALUE: {assignment!r}")
    return key.strip(), value


def _parse_named_paths(
    values: list[str],
    *,
    paths: Mapping[str, Path],
    configured_worktree: Path,
    output_path: Path,
    extra_paths: Mapping[str, Path] | None = None,
) -> dict[str, Path]:
    parsed: dict[str, Path] = {}
    for value in values:
        name, separator, raw_path = value.partition("=")
        if not separator or not name.strip() or not raw_path.strip():
            raise ValueError(f"path option must be NAME=PATH: {value!r}")
        parsed[name.strip()] = _resolve_capture_path(
            raw_path.strip(),
            paths=paths,
            configured_worktree=configured_worktree,
            output_path=output_path,
            extra_paths=extra_paths,
        )
    return parsed


def _resolve_capture_path(
    raw_path: str,
    *,
    paths: Mapping[str, Path],
    configured_worktree: Path,
    output_path: Path,
    extra_paths: Mapping[str, Path] | None = None,
) -> Path:
    token_paths = {
        "__WORKTREE__": configured_worktree,
        "__OUTPUT__": output_path,
        "__OUTPUT_DIR__": output_path.parent,
        "__SPARK_HOME__": paths["spark_home"],
        "__FLOWS_DIR__": paths["spark_flows_dir"],
        "__CODEX_RUNTIME_ROOT__": paths["attractor_codex_runtime_root"],
        "__CODEX_HOME__": paths["codex_home"],
    }
    for name, path in (extra_paths or {}).items():
        token_paths[_normalize_token_name(name)] = path

    resolved_value = raw_path
    for token, path in token_paths.items():
        resolved_value = resolved_value.replace(token, str(path))

    candidate = Path(resolved_value).expanduser()
    if not candidate.is_absolute():
        candidate = configured_worktree / candidate
    return candidate.resolve(strict=False)


def _normalize_token_name(name: str) -> str:
    stripped = name.strip()
    if stripped.startswith("__") and stripped.endswith("__") and len(stripped) > 4:
        return stripped
    normalized = re.sub(r"[^A-Za-z0-9]+", "_", stripped).strip("_").upper()
    if not normalized:
        raise ValueError(f"normalization token is empty: {name!r}")
    return f"__{normalized}__"


def _normalization_replacements(
    *,
    configured_worktree: Path,
    output_path: Path,
    paths: Mapping[str, Path],
    extra_paths: Mapping[str, Path],
) -> dict[str, str]:
    token_paths = {
        "__WORKTREE__": configured_worktree,
        "__OUTPUT__": output_path,
        "__OUTPUT_DIR__": output_path.parent,
        "__SPARK_HOME__": paths["spark_home"],
        "__FLOWS_DIR__": paths["spark_flows_dir"],
        "__CODEX_RUNTIME_ROOT__": paths["attractor_codex_runtime_root"],
        "__CODEX_HOME__": paths["codex_home"],
    }
    for name, path in extra_paths.items():
        token_paths[_normalize_token_name(name)] = path

    replacements: dict[str, str] = {}
    for token, path in token_paths.items():
        for spelling in {str(path), path.as_posix(), str(path.resolve(strict=False))}:
            if spelling and spelling != "/":
                replacements[spelling] = token
    return dict(sorted(replacements.items(), key=lambda item: len(item[0]), reverse=True))


def _normalize_value(value: Any, replacements: Mapping[str, str]) -> Any:
    if isinstance(value, str):
        normalized = value
        for raw_path, token in replacements.items():
            normalized = normalized.replace(raw_path, token)
        return normalized
    if isinstance(value, list):
        return [_normalize_value(item, replacements) for item in value]
    if isinstance(value, tuple):
        return [_normalize_value(item, replacements) for item in value]
    if isinstance(value, dict):
        return {
            str(key): _normalize_value(child, replacements)
            for key, child in value.items()
        }
    return value


def _selected_environment(env: dict[str, str]) -> dict[str, str]:
    selected: dict[str, str] = {}
    for key, value in sorted(env.items()):
        if key in RELEVANT_ENV_NAMES or key.startswith(RELEVANT_ENV_PREFIXES):
            selected[key] = _mask_env_value(key, value)
    return selected


def _mask_env_value(key: str, value: str) -> str:
    upper_key = key.upper()
    if any(marker in upper_key for marker in SECRET_ENV_MARKERS):
        if not value:
            return ""
        return "<redacted>"
    return value


def _filesystem_snapshots(
    roots: Mapping[str, Path],
    replacements: Mapping[str, str],
) -> dict[str, Any]:
    return {
        name: _normalize_value(_filesystem_snapshot(path), replacements)
        for name, path in sorted(roots.items())
    }


def _filesystem_snapshot(root: Path) -> dict[str, Any]:
    entries: list[dict[str, Any]] = []
    if root.exists():
        for path in sorted(root.rglob("*")):
            relative_path = path.relative_to(root).as_posix()
            if path.is_dir():
                entries.append({"path": relative_path, "kind": "dir"})
            elif path.is_file():
                payload = path.read_bytes()
                entries.append(
                    {
                        "path": relative_path,
                        "kind": "file",
                        "size": len(payload),
                        "sha256": hashlib.sha256(payload).hexdigest(),
                    }
                )
    return {
        "root": str(root.resolve(strict=False)),
        "exists": root.exists(),
        "entries": entries,
    }


def _state_snapshots(
    files: Mapping[str, Path],
    replacements: Mapping[str, str],
) -> dict[str, Any]:
    return {
        name: _normalize_value(_state_snapshot(path), replacements)
        for name, path in sorted(files.items())
    }


def _state_snapshot(path: Path) -> dict[str, Any]:
    payload: dict[str, Any] = {
        "path": str(path.resolve(strict=False)),
        "exists": path.exists(),
    }
    if not path.exists():
        return payload
    suffix = path.suffix.lower()
    try:
        if suffix == ".json":
            payload.update({"format": "json", "data": json.loads(path.read_text(encoding="utf-8"))})
        elif suffix == ".jsonl":
            payload.update(
                {
                    "format": "jsonl",
                    "records": [
                        json.loads(line)
                        for line in path.read_text(encoding="utf-8").splitlines()
                        if line.strip()
                    ],
                }
            )
        elif suffix == ".toml":
            payload.update({"format": "toml", "data": tomllib.loads(path.read_text(encoding="utf-8"))})
        else:
            payload.update({"format": "bytes", "size": path.stat().st_size})
    except Exception as exc:  # noqa: BLE001 - fixture capture records parser failures as observations.
        payload.update({"error": exc.__class__.__name__, "message": str(exc)})
    return payload


def _manifest(
    *,
    args: argparse.Namespace,
    configured_worktree: Path,
    validation_root: Path,
    output_path: Path,
    paths: dict[str, Path],
    command: list[str],
    replacements: Mapping[str, str],
    selected_env: dict[str, str],
    filesystem: dict[str, Any],
    durable_state: dict[str, Any],
    process: dict[str, Any],
) -> dict[str, Any]:
    return {
        "schema_version": 1,
        "fixture_id": args.fixture_id,
        "item_id": args.item_id,
        "requirements": list(args.requirement),
        "decisions": list(args.decision),
        "provenance": {
            "captured_at_utc": datetime.now(UTC).isoformat().replace("+00:00", "Z"),
            "oracle": "python-worktree",
            "capture_tool": "scripts/compat_capture.py",
            "worktree_path": str(configured_worktree),
            "validation_root": str(validation_root),
        },
        "command": {
            "argv": command,
            "normalized_argv": _normalize_value(command, replacements),
            "cwd": str(configured_worktree),
        },
        "environment": {
            "selected_keys": sorted(selected_env),
            "selected": _normalize_value(selected_env, replacements),
        },
        "paths": {
            "output": str(output_path),
            **{key: str(value) for key, value in paths.items()},
        },
        "normalization": {
            "tokens": {
                token: raw_path
                for raw_path, token in sorted(replacements.items(), key=lambda item: item[1])
            },
        },
        "filesystem": filesystem,
        "durable_state": durable_state,
        "process": _normalize_value(process, replacements),
    }


if __name__ == "__main__":
    raise SystemExit(main())
