from __future__ import annotations

from pathlib import Path

import pytest

from attractor.engine.artifacts import ArtifactInfo, ArtifactStore


def test_write_text_registers_filesystem_backed_artifact(tmp_path: Path) -> None:
    store = ArtifactStore(base_dir=tmp_path)

    info = store.write_text("tool_node", "stdout.txt", "hello")

    assert isinstance(info, ArtifactInfo)
    assert info.path == "artifacts/tool_node/stdout.txt"
    assert info.size_bytes == 5
    assert info.media_type == "text/plain"
    assert (tmp_path / "tool_node" / "stdout.txt").read_text(encoding="utf-8") == "hello"


def test_copy_path_copies_file_into_node_artifact_directory(tmp_path: Path) -> None:
    store = ArtifactStore(base_dir=tmp_path)
    source_path = tmp_path / "source.log"
    source_path.write_text("copied", encoding="utf-8")

    info = store.copy_path("tool_node", source_path, "captured/source.log")

    assert info.path == "artifacts/tool_node/captured/source.log"
    assert (tmp_path / "tool_node" / "captured" / "source.log").read_text(encoding="utf-8") == "copied"


def test_copy_matches_copies_relative_glob_matches(tmp_path: Path) -> None:
    store = ArtifactStore(base_dir=tmp_path / "artifacts")
    cwd = tmp_path / "workdir"
    (cwd / "reports" / "nested").mkdir(parents=True, exist_ok=True)
    (cwd / "reports" / "summary.json").write_text("{}", encoding="utf-8")
    (cwd / "reports" / "nested" / "detail.txt").write_text("detail", encoding="utf-8")

    infos = store.copy_matches("tool_node", cwd, ["reports/**"])

    assert [info.path for info in infos] == [
        "artifacts/tool_node/captured/reports/nested/detail.txt",
        "artifacts/tool_node/captured/reports/summary.json",
    ]
    assert (tmp_path / "artifacts" / "tool_node" / "captured" / "reports" / "summary.json").exists()
    assert (tmp_path / "artifacts" / "tool_node" / "captured" / "reports" / "nested" / "detail.txt").exists()


def test_copy_matches_allows_zero_match_patterns(tmp_path: Path) -> None:
    store = ArtifactStore(base_dir=tmp_path / "artifacts")
    cwd = tmp_path / "workdir"
    cwd.mkdir(parents=True, exist_ok=True)

    infos = store.copy_matches("tool_node", cwd, ["missing/**/*.json"])

    assert infos == []


def test_write_text_rejects_invalid_relative_paths(tmp_path: Path) -> None:
    store = ArtifactStore(base_dir=tmp_path)

    with pytest.raises(ValueError, match="must be relative"):
        store.write_text("tool_node", "/tmp/output.txt", "hello")

    with pytest.raises(ValueError, match="must not escape"):
        store.write_text("tool_node", "../output.txt", "hello")


def test_copy_matches_rejects_absolute_and_parent_patterns(tmp_path: Path) -> None:
    store = ArtifactStore(base_dir=tmp_path / "artifacts")
    cwd = tmp_path / "workdir"
    cwd.mkdir(parents=True, exist_ok=True)

    with pytest.raises(ValueError, match="must be relative"):
        store.copy_matches("tool_node", cwd, ["/tmp/*.json"])

    with pytest.raises(ValueError, match="must stay within the tool working directory"):
        store.copy_matches("tool_node", cwd, ["../*.json"])


def test_list_returns_registered_artifacts_in_write_order(tmp_path: Path) -> None:
    store = ArtifactStore(base_dir=tmp_path)
    first = store.write_text("tool_node", "stdout.txt", "one")
    second_path = tmp_path / "report.json"
    second_path.write_text("{}", encoding="utf-8")
    second = store.copy_path("tool_node", second_path, "captured/report.json")

    assert store.list() == [first, second]
