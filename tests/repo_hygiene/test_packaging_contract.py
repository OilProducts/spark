from __future__ import annotations

from pathlib import Path
import tomllib


def test_pyproject_uses_standard_setuptools_backend() -> None:
    repo_root = Path(__file__).resolve().parents[2]
    pyproject = tomllib.loads((repo_root / "pyproject.toml").read_text(encoding="utf-8"))

    build_system = pyproject["build-system"]
    assert build_system["build-backend"] == "setuptools.build_meta"
    assert "backend-path" not in build_system


def test_custom_build_backend_files_are_absent() -> None:
    repo_root = Path(__file__).resolve().parents[2]

    assert not (repo_root / "build_backend.py").exists()
    assert not (repo_root / "MANIFEST.in").exists()


def test_packaged_ui_dist_stays_ignored() -> None:
    repo_root = Path(__file__).resolve().parents[2]
    gitignore = (repo_root / ".gitignore").read_text(encoding="utf-8")

    assert "src/spark_app/ui_dist/" in gitignore
