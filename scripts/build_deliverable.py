from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path
import os
import shutil
import subprocess
import sys
import tempfile
import zipfile


FRONTEND_BINARIES = ("tsc", "vite")
REQUIRED_WHEEL_ENTRIES = (
    "spark_app/ui_dist/index.html",
    "spark/guides/dot-authoring.md",
    "spark/guides/attractor-spec.md",
    "spark/guides/spark-flow-extensions.md",
    "spark/starter_flows/simple-linear.dot",
    "spark/starter_flows/spec-implementation/implement-spec.dot",
)
SMOKE_CODE = """
from pathlib import Path
from tempfile import TemporaryDirectory

from spark.authoring_assets import (
    attractor_spec_path,
    dot_authoring_guide_path,
    flow_extensions_spec_path,
)
from spark.starter_assets import load_starter_flow_assets
from spark_app.ui import resolve_default_ui_dir

with TemporaryDirectory() as tmp:
    ui_dir = resolve_default_ui_dir(Path(tmp))
    assert ui_dir is not None
    assert (ui_dir / "index.html").exists()

assets = {asset.name for asset in load_starter_flow_assets(project_root=Path("."))}
assert "simple-linear.dot" in assets
assert "spec-implementation/implement-spec.dot" in assets

for path in (dot_authoring_guide_path(), attractor_spec_path(), flow_extensions_spec_path()):
    assert path.exists(), path
"""


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
        _stage_packaged_ui(repo_root / "frontend" / "dist", stage_root / "src" / "spark_app" / "ui_dist")
        _run(["uv", "build"], cwd=stage_root)
        artifacts = _locate_artifacts(stage_root / "dist")
        _verify_wheel_contents(artifacts.wheel)
        _verify_installable(repo_root, artifacts.wheel, artifact_kind="wheel")
        _verify_installable(repo_root, artifacts.sdist, artifact_kind="sdist")
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


def _verify_installable(repo_root: Path, artifact_path: Path, *, artifact_kind: str) -> None:
    with tempfile.TemporaryDirectory(prefix=f"spark-{artifact_kind}-venv-") as venv_str:
        venv_dir = Path(venv_str)
        _run(["uv", "venv", "--seed", str(venv_dir)], cwd=repo_root)
        python_executable = _venv_python(venv_dir)
        if artifact_kind == "sdist":
            _run(["uv", "pip", "install", "--python", str(python_executable), "setuptools", "wheel"], cwd=repo_root)
        install_command = [str(python_executable), "-m", "pip", "install", "--no-deps"]
        if artifact_kind == "sdist":
            install_command.append("--no-build-isolation")
        install_command.append(str(artifact_path))
        _run(install_command, cwd=repo_root)
        _run([str(python_executable), "-c", SMOKE_CODE], cwd=repo_root)


def _publish_artifacts(repo_dist: Path, artifacts: BuildArtifacts) -> None:
    repo_dist.mkdir(parents=True, exist_ok=True)
    for old_artifact in list(repo_dist.glob("spark-*.whl")) + list(repo_dist.glob("spark-*.tar.gz")):
        old_artifact.unlink()
    shutil.copy2(artifacts.wheel, repo_dist / artifacts.wheel.name)
    shutil.copy2(artifacts.sdist, repo_dist / artifacts.sdist.name)


def _venv_python(venv_dir: Path) -> Path:
    if sys.platform == "win32":
        return venv_dir / "Scripts" / "python.exe"
    return venv_dir / "bin" / "python"


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
