from __future__ import annotations

from pathlib import Path

import pytest

from scripts import build_deliverable


def _write(path: Path, content: str = "") -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(content, encoding="utf-8")


def test_copy_source_tree_copies_filesystem_sources_without_generated_outputs(
    tmp_path: Path,
) -> None:
    source_root = tmp_path / "source"
    stage_root = tmp_path / "stage"
    _write(source_root / "pyproject.toml")
    _write(source_root / "scripts" / "build_deliverable.py")
    _write(source_root / "src" / "spark" / "__init__.py")
    _write(source_root / "frontend" / "package-lock.json")
    _write(source_root / "frontend" / "src" / "main.ts")
    _write(source_root / ".git" / "HEAD")
    _write(source_root / ".spark" / "state.json")
    _write(source_root / "dist" / "spark-0.1.0.whl")
    _write(source_root / "frontend" / "node_modules" / ".bin" / "vite")
    _write(source_root / "frontend" / "dist" / "index.html")
    _write(source_root / "src" / "spark" / "__pycache__" / "module.cpython-311.pyc")

    build_deliverable._copy_source_tree(source_root, stage_root)

    assert (stage_root / "pyproject.toml").exists()
    assert (stage_root / "scripts" / "build_deliverable.py").exists()
    assert (stage_root / "src" / "spark" / "__init__.py").exists()
    assert (stage_root / "frontend" / "package-lock.json").exists()
    assert (stage_root / "frontend" / "src" / "main.ts").exists()
    assert not (stage_root / ".git").exists()
    assert not (stage_root / ".spark").exists()
    assert not (stage_root / "dist").exists()
    assert not (stage_root / "frontend" / "node_modules").exists()
    assert not (stage_root / "frontend" / "dist").exists()
    assert not (stage_root / "src" / "spark" / "__pycache__").exists()


def test_ensure_source_tree_reports_missing_required_paths(tmp_path: Path) -> None:
    with pytest.raises(RuntimeError, match="pyproject.toml"):
        build_deliverable._ensure_source_tree(tmp_path)


def test_publish_artifacts_replaces_outputs_and_applies_host_ownership(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    built_dist = tmp_path / "built-dist"
    output_dir = tmp_path / "output"
    wheel = built_dist / "spark-0.1.0-py3-none-any.whl"
    sdist = built_dist / "spark-0.1.0.tar.gz"
    _write(wheel, "wheel")
    _write(sdist, "sdist")
    _write(output_dir / "spark-old.whl", "old wheel")
    _write(output_dir / "spark-old.tar.gz", "old sdist")
    chowned: list[tuple[Path, int, int]] = []

    monkeypatch.setenv("HOST_UID", "123")
    monkeypatch.setenv("HOST_GID", "456")
    monkeypatch.setattr(
        build_deliverable.os,
        "chown",
        lambda path, uid, gid: chowned.append((Path(path), uid, gid)),
    )

    build_deliverable._publish_artifacts(
        output_dir,
        build_deliverable.BuildArtifacts(wheel=wheel, sdist=sdist),
    )

    assert sorted(path.name for path in output_dir.iterdir()) == [
        "spark-0.1.0-py3-none-any.whl",
        "spark-0.1.0.tar.gz",
    ]
    assert chowned == [
        (output_dir / "spark-0.1.0-py3-none-any.whl", 123, 456),
        (output_dir / "spark-0.1.0.tar.gz", 123, 456),
    ]
