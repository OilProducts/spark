from __future__ import annotations

from datetime import datetime, timezone
import json
from pathlib import Path

import pytest

from attractor.engine.artifacts import ArtifactInfo, ArtifactStore


def test_store_registers_artifact_with_metadata() -> None:
    store = ArtifactStore()

    info = store.store("artifact-1", "plan-output", {"status": "ok"})

    assert isinstance(info, ArtifactInfo)
    assert info.id == "artifact-1"
    assert info.name == "plan-output"
    assert info.size_bytes > 0
    assert isinstance(info.stored_at, datetime)
    assert info.stored_at.tzinfo == timezone.utc
    assert info.is_file_backed is False


def test_retrieve_supports_optional_type_assertion() -> None:
    store = ArtifactStore()
    payload = {"result": "ready"}
    store.store("artifact-2", "review-output", payload)

    typed_value = store.retrieve("artifact-2", expected_type=dict)
    assert typed_value == payload

    with pytest.raises(TypeError):
        store.retrieve("artifact-2", expected_type=list)


def test_store_file_backs_large_payload_when_base_dir_present(tmp_path: Path) -> None:
    store = ArtifactStore(base_dir=tmp_path)
    payload = {"blob": "x" * (100 * 1024 + 1)}

    info = store.store("artifact-large", "large-output", payload)

    assert info.is_file_backed is True
    artifact_path = tmp_path / "artifacts" / "artifact-large.json"
    assert artifact_path.exists()
    assert json.loads(artifact_path.read_text(encoding="utf-8")) == payload
    assert store.retrieve("artifact-large", expected_type=dict) == payload


def test_store_keeps_large_payload_in_memory_without_base_dir() -> None:
    store = ArtifactStore()
    payload = "x" * (100 * 1024 + 1)

    info = store.store("artifact-memory", "large-output", payload)

    assert info.is_file_backed is False
    assert store.retrieve("artifact-memory", expected_type=str) == payload


def test_has_reports_presence_by_artifact_id() -> None:
    store = ArtifactStore()

    assert store.has("artifact-1") is False
    store.store("artifact-1", "sample", {"ok": True})
    assert store.has("artifact-1") is True


def test_list_returns_registered_artifact_metadata() -> None:
    store = ArtifactStore()
    first = store.store("artifact-1", "one", 1)
    second = store.store("artifact-2", "two", 2)

    infos = store.list()

    assert infos == [first, second]


def test_remove_deletes_registered_artifact() -> None:
    store = ArtifactStore()
    store.store("artifact-1", "sample", {"ok": True})

    store.remove("artifact-1")

    assert store.has("artifact-1") is False
    with pytest.raises(KeyError):
        store.retrieve("artifact-1")


def test_clear_removes_all_artifacts() -> None:
    store = ArtifactStore()
    store.store("artifact-1", "one", {"a": 1})
    store.store("artifact-2", "two", {"b": 2})

    store.clear()

    assert store.list() == []
    assert store.has("artifact-1") is False
    assert store.has("artifact-2") is False
