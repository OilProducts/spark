from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path
import os
import shutil
import subprocess
import tempfile
import zipfile


FRONTEND_BINARIES = ("tsc", "vite")
REQUIRED_SOURCE_PATHS = (
    "pyproject.toml",
    "src",
    "frontend/package-lock.json",
    "scripts/build_deliverable.py",
)
EXCLUDED_DIR_NAMES = {
    ".git",
    ".pytest_cache",
    ".ruff_cache",
    ".spark",
    ".venv",
    "__pycache__",
    "dist",
}
EXCLUDED_RELATIVE_DIRS = {
    Path("frontend") / "dist",
    Path("frontend") / "node_modules",
}
EXCLUDED_FILE_SUFFIXES = (".pyc", ".pyo")
REQUIRED_WHEEL_ENTRIES = (
    "spark/ui_dist/index.html",
    "spark/guides/dot-authoring.md",
    "spark/guides/spark-operations.md",
    "spark/flows/examples/simple-linear.dot",
    "spark/flows/software-development/spec-implementation/implement-spec.dot",
)
FORBIDDEN_WHEEL_ENTRIES = (
    "spark/guides/attractor-spec.md",
    "spark/guides/spark-flow-extensions.md",
)


@dataclass(frozen=True)
class BuildArtifacts:
    wheel: Path
    sdist: Path


def main() -> int:
    repo_root = Path(__file__).resolve().parents[1]
    _ensure_source_tree(repo_root)
    _ensure_frontend_deps(repo_root)
    _run(["npm", "--prefix", "frontend", "run", "build"], cwd=repo_root)

    with tempfile.TemporaryDirectory(prefix="spark-deliverable-stage-") as stage_str:
        stage_root = Path(stage_str)
        _copy_source_tree(repo_root, stage_root)
        _stage_packaged_ui(
            repo_root / "frontend" / "dist",
            stage_root / "src" / "spark" / "ui_dist",
        )
        _run(["uv", "build"], cwd=stage_root)
        artifacts = _locate_artifacts(stage_root / "dist")
        _verify_wheel_contents(artifacts.wheel)
        output_dir = _resolve_output_dir(repo_root)
        _publish_artifacts(output_dir, artifacts)

    print(f"deliverable ready: {output_dir}")
    return 0


def _ensure_source_tree(repo_root: Path) -> None:
    missing = [
        relative_path
        for relative_path in REQUIRED_SOURCE_PATHS
        if not (repo_root / relative_path).exists()
    ]
    if missing:
        joined = "\n".join(missing)
        raise RuntimeError(f"deliverable build source tree is missing required paths:\n{joined}")


def _ensure_frontend_deps(repo_root: Path) -> None:
    if all((repo_root / "frontend" / "node_modules" / ".bin" / binary).exists() for binary in FRONTEND_BINARIES):
        return
    _run(["npm", "--prefix", "frontend", "ci"], cwd=repo_root)


def _copy_source_tree(repo_root: Path, stage_root: Path) -> None:
    for root_str, dir_names, file_names in os.walk(repo_root):
        root = Path(root_str)
        relative_root = root.relative_to(repo_root)
        dir_names[:] = [
            dir_name
            for dir_name in dir_names
            if not _is_excluded_source_path(
                _normalize_relative_path(relative_root / dir_name),
                is_dir=True,
            )
        ]
        for file_name in file_names:
            relative_path = _normalize_relative_path(relative_root / file_name)
            if _is_excluded_source_path(relative_path, is_dir=False):
                continue
            source = repo_root / relative_path
            target = stage_root / relative_path
            target.parent.mkdir(parents=True, exist_ok=True)
            if source.is_symlink():
                if target.exists() or target.is_symlink():
                    target.unlink()
                target.symlink_to(os.readlink(source))
                continue
            shutil.copy2(source, target)


def _normalize_relative_path(path: Path) -> Path:
    return Path() if path == Path(".") else path


def _is_excluded_source_path(relative_path: Path, *, is_dir: bool) -> bool:
    if relative_path == Path():
        return False
    if any(part in EXCLUDED_DIR_NAMES for part in relative_path.parts):
        return True
    if is_dir and relative_path in EXCLUDED_RELATIVE_DIRS:
        return True
    return not is_dir and relative_path.suffix in EXCLUDED_FILE_SUFFIXES


def _stage_packaged_ui(source_dist: Path, packaged_ui_dir: Path) -> None:
    index_path = source_dist / "index.html"
    if not index_path.exists():
        raise RuntimeError(f"frontend build did not produce index.html: {index_path}")
    if packaged_ui_dir.exists():
        shutil.rmtree(packaged_ui_dir)
    packaged_ui_dir.parent.mkdir(parents=True, exist_ok=True)
    shutil.copytree(source_dist, packaged_ui_dir)


def _locate_artifacts(dist_dir: Path) -> BuildArtifacts:
    wheels = sorted(dist_dir.glob("spark-*.whl"))
    sdists = sorted(dist_dir.glob("spark-*.tar.gz"))
    if len(wheels) != 1 or len(sdists) != 1:
        raise RuntimeError(f"expected exactly one spark wheel and one spark sdist in {dist_dir}")
    return BuildArtifacts(wheel=wheels[0], sdist=sdists[0])


def _verify_wheel_contents(wheel_path: Path) -> None:
    with zipfile.ZipFile(wheel_path) as wheel_file:
        names = set(wheel_file.namelist())
    missing = [entry for entry in REQUIRED_WHEEL_ENTRIES if entry not in names]
    if missing:
        joined = "\n".join(missing)
        raise RuntimeError(f"wheel is missing required packaged assets:\n{joined}")
    present_forbidden = [entry for entry in FORBIDDEN_WHEEL_ENTRIES if entry in names]
    if present_forbidden:
        joined = "\n".join(present_forbidden)
        raise RuntimeError(f"wheel unexpectedly contains removed packaged specs:\n{joined}")


def _publish_artifacts(repo_dist: Path, artifacts: BuildArtifacts) -> None:
    repo_dist.mkdir(parents=True, exist_ok=True)
    for old_artifact in list(repo_dist.glob("spark-*.whl")) + list(
        repo_dist.glob("spark-*.tar.gz")
    ):
        old_artifact.unlink()
    copied_artifacts = [
        shutil.copy2(artifacts.wheel, repo_dist / artifacts.wheel.name),
        shutil.copy2(artifacts.sdist, repo_dist / artifacts.sdist.name),
    ]
    _chown_artifacts(copied_artifacts)


def _resolve_output_dir(repo_root: Path) -> Path:
    output_dir = os.environ.get("SPARK_DELIVERABLE_OUT")
    if output_dir:
        return Path(output_dir)
    return repo_root / "dist"


def _chown_artifacts(artifact_paths: list[str | Path]) -> None:
    raw_uid = os.environ.get("HOST_UID")
    raw_gid = os.environ.get("HOST_GID")
    if raw_uid is None and raw_gid is None:
        return
    if raw_uid is None or raw_gid is None:
        raise RuntimeError("HOST_UID and HOST_GID must be provided together")
    try:
        uid = int(raw_uid)
        gid = int(raw_gid)
    except ValueError as exc:
        raise RuntimeError("HOST_UID and HOST_GID must be integer values") from exc
    for artifact_path in artifact_paths:
        os.chown(artifact_path, uid, gid)


def _run(command: list[str], *, cwd: Path, capture_output: bool = False) -> subprocess.CompletedProcess[str]:
    print("+", " ".join(command))
    return subprocess.run(
        command,
        cwd=cwd,
        check=True,
        text=True,
        capture_output=capture_output,
    )


if __name__ == "__main__":
    raise SystemExit(main())
