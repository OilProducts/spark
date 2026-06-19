from __future__ import annotations

import os
from pathlib import Path
import shutil
import sys
import tempfile


RUNTIME_REPO_ROOT = Path(__file__).resolve().parents[2]


def _venv_bin_dir(venv_root: Path) -> Path:
    return venv_root / ("Scripts" if os.name == "nt" else "bin")


def _first_party_tool_bin_dirs() -> list[Path]:
    candidates = [
        Path(sys.executable).resolve(strict=False).parent,
        _venv_bin_dir(RUNTIME_REPO_ROOT / ".venv"),
    ]
    tool_bin_dirs: list[Path] = []
    seen: set[str] = set()
    for candidate in candidates:
        normalized = candidate.expanduser().resolve(strict=False)
        if not normalized.exists():
            continue
        key = os.path.normcase(str(normalized))
        if key in seen:
            continue
        seen.add(key)
        tool_bin_dirs.append(normalized)
    return tool_bin_dirs


def _ensure_directory(path: Path) -> None:
    path.mkdir(parents=True, exist_ok=True)


def _copy_file_if_changed(source: Path, destination: Path, *, preserve_metadata: bool = True) -> None:
    if destination.exists():
        try:
            if source.read_bytes() == destination.read_bytes():
                return
        except OSError:
            pass
    if preserve_metadata:
        shutil.copy2(source, destination)
    else:
        shutil.copyfile(source, destination)


def _copy_tree_contents(source: Path, destination: Path) -> None:
    if not source.exists() or not source.is_dir():
        return
    _ensure_directory(destination)
    for entry in source.iterdir():
        target = destination / entry.name
        if entry.is_dir():
            shutil.copytree(
                entry,
                target,
                dirs_exist_ok=True,
                copy_function=lambda src, dst: shutil.copyfile(src, dst),
            )
        elif entry.is_file():
            _copy_file_if_changed(entry, target, preserve_metadata=False)


def _default_spark_home(env: dict[str, str]) -> Path:
    configured_spark_home = str(env.get("SPARK_HOME", "")).strip()
    if configured_spark_home:
        return Path(configured_spark_home).expanduser()
    return Path(env.get("HOME", str(Path.home()))).expanduser() / ".spark"


def _resolve_runtime_root(env: dict[str, str]) -> Path:
    configured_runtime_root = str(env.get("ATTRACTOR_CODEX_RUNTIME_ROOT", "")).strip()
    if configured_runtime_root:
        return Path(configured_runtime_root).expanduser()
    return _default_spark_home(env) / "runtime" / "codex"


def build_codex_runtime_environment() -> dict[str, str]:
    env = os.environ.copy()
    original_home = Path(env.get("HOME", str(Path.home()))).expanduser()
    original_codex_home = Path(env.get("CODEX_HOME", str(original_home / ".codex"))).expanduser()
    runtime_root = _resolve_runtime_root(env)
    try:
        _ensure_directory(runtime_root)
    except OSError:
        runtime_root = Path(tempfile.gettempdir()) / "spark-codex-runtime"
        _ensure_directory(runtime_root)
    codex_home = Path(env.get("CODEX_HOME", str(runtime_root / ".codex"))).expanduser()
    xdg_config_home = Path(env.get("XDG_CONFIG_HOME", str(runtime_root / ".config"))).expanduser()
    xdg_data_home = Path(env.get("XDG_DATA_HOME", str(runtime_root / ".local/share"))).expanduser()
    for directory in (runtime_root, codex_home, xdg_config_home, xdg_data_home):
        _ensure_directory(directory)

    explicit_seed_dir = Path(env.get("ATTRACTOR_CODEX_SEED_DIR", "/codex-seed")).expanduser()
    seed_candidates: list[Path] = []
    for candidate in (explicit_seed_dir, original_codex_home):
        normalized_candidate = candidate.expanduser()
        if normalized_candidate == codex_home:
            continue
        if normalized_candidate in seed_candidates:
            continue
        seed_candidates.append(normalized_candidate)
    for file_name in ("auth.json", "config.toml"):
        source = next((candidate / file_name for candidate in seed_candidates if (candidate / file_name).exists()), None)
        if source is None:
            continue
        destination = codex_home / file_name
        try:
            _copy_file_if_changed(source, destination)
        except OSError:
            pass
    for candidate in seed_candidates:
        try:
            _copy_tree_contents(candidate / "plugins" / "cache", codex_home / "plugins" / "cache")
        except (OSError, shutil.Error):
            pass

    env.update(
        {
            "HOME": str(runtime_root),
            "CODEX_HOME": str(codex_home),
            "XDG_CONFIG_HOME": str(xdg_config_home),
            "XDG_DATA_HOME": str(xdg_data_home),
        }
    )
    tool_path_prefix = os.pathsep.join(str(path) for path in _first_party_tool_bin_dirs())
    if tool_path_prefix:
        existing_path = env.get("PATH", "")
        env["PATH"] = tool_path_prefix if not existing_path else f"{tool_path_prefix}{os.pathsep}{existing_path}"
    return env
