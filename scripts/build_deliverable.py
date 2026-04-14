from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path
import os
import shutil
import subprocess
import tempfile
import zipfile


FRONTEND_BINARIES = ("tsc", "vite")
REQUIRED_WHEEL_ENTRIES = (
    "spark/ui_dist/index.html",
    "spark/guides/dot-authoring.md",
    "spark/guides/spark-operations.md",
    "spark/flows/examples/simple-linear.dot",
    "spark/flows/spec-implementation/implement-spec.dot",
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
    _ensure_git_checkout(repo_root)
    _ensure_frontend_deps(repo_root)
    _run(["npm", "--prefix", "frontend", "run", "build"], cwd=repo_root)

    with tempfile.TemporaryDirectory(prefix="spark-deliverable-stage-") as stage_str:
        stage_root = Path(stage_str)
        _copy_tracked_worktree(repo_root, stage_root)
        _stage_packaged_ui(repo_root / "frontend" / "dist", stage_root / "src" / "spark" / "ui_dist")
        _run(["uv", "build"], cwd=stage_root)
        artifacts = _locate_artifacts(stage_root / "dist")
        _verify_wheel_contents(artifacts.wheel)
        _publish_artifacts(repo_root / "dist", artifacts)

    print(f"deliverable ready: {repo_root / 'dist'}")
    return 0


def _ensure_git_checkout(repo_root: Path) -> None:
    result = _run(["git", "rev-parse", "--show-toplevel"], cwd=repo_root, capture_output=True)
    top_level = Path(result.stdout.strip()).resolve(strict=False)
    if top_level != repo_root:
        raise RuntimeError(f"deliverable build must run from the repository root: {repo_root}")


def _ensure_frontend_deps(repo_root: Path) -> None:
    if all((repo_root / "frontend" / "node_modules" / ".bin" / binary).exists() for binary in FRONTEND_BINARIES):
        return
    _run(["npm", "--prefix", "frontend", "ci"], cwd=repo_root)


def _copy_tracked_worktree(repo_root: Path, stage_root: Path) -> None:
    result = _run(["git", "ls-files", "-z"], cwd=repo_root, capture_output=True)
    for raw_path in result.stdout.encode("utf-8").split(b"\0"):
        if not raw_path:
            continue
        relative_path = Path(raw_path.decode("utf-8"))
        source = repo_root / relative_path
        if not source.exists():
            continue
        target = stage_root / relative_path
        target.parent.mkdir(parents=True, exist_ok=True)
        if source.is_symlink():
            if target.exists() or target.is_symlink():
                target.unlink()
            target.symlink_to(os.readlink(source))
            continue
        shutil.copy2(source, target)


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
    for old_artifact in list(repo_dist.glob("spark-*.whl")) + list(repo_dist.glob("spark-*.tar.gz")):
        old_artifact.unlink()
    shutil.copy2(artifacts.wheel, repo_dist / artifacts.wheel.name)
    shutil.copy2(artifacts.sdist, repo_dist / artifacts.sdist.name)


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
