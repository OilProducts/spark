from __future__ import annotations

from dataclasses import dataclass
from datetime import datetime, timezone
import json
from pathlib import Path
from typing import Any, Dict, Tuple, Type, TypeVar

from .context import ReadWriteLock


T = TypeVar("T")
FILE_BACKING_THRESHOLD_BYTES = 100 * 1024


@dataclass(frozen=True)
class ArtifactInfo:
    id: str
    name: str
    size_bytes: int
    stored_at: datetime
    is_file_backed: bool


class ArtifactStore:
    def __init__(
        self,
        *,
        base_dir: str | Path | None = None,
        file_backing_threshold_bytes: int = FILE_BACKING_THRESHOLD_BYTES,
    ) -> None:
        self._artifacts: Dict[str, Tuple[ArtifactInfo, Any]] = {}
        self._lock = ReadWriteLock()
        self._base_dir = Path(base_dir) if base_dir is not None else None
        self._file_backing_threshold_bytes = file_backing_threshold_bytes

    def store(self, artifact_id: str, name: str, data: Any) -> ArtifactInfo:
        size_bytes = _byte_size(data)
        is_file_backed = size_bytes > self._file_backing_threshold_bytes and self._base_dir is not None
        stored_data: Any = data
        if is_file_backed:
            artifacts_dir = self._base_dir / "artifacts"
            artifacts_dir.mkdir(parents=True, exist_ok=True)
            artifact_path = artifacts_dir / f"{artifact_id}.json"
            artifact_path.write_text(
                json.dumps(data, ensure_ascii=False, sort_keys=True),
                encoding="utf-8",
            )
            stored_data = artifact_path

        info = ArtifactInfo(
            id=artifact_id,
            name=name,
            size_bytes=size_bytes,
            stored_at=datetime.now(timezone.utc),
            is_file_backed=is_file_backed,
        )
        with self._lock.write_lock():
            self._artifacts[artifact_id] = (info, stored_data)
        return info

    def retrieve(self, artifact_id: str, expected_type: Type[T] | None = None) -> T | Any:
        with self._lock.read_lock():
            artifact = self._artifacts.get(artifact_id)
        if artifact is None:
            raise KeyError(f"Artifact not found: {artifact_id}")

        info, data = artifact
        if info.is_file_backed:
            data = json.loads(Path(data).read_text(encoding="utf-8"))
        if expected_type is not None and not isinstance(data, expected_type):
            raise TypeError(
                f"Artifact '{artifact_id}' expected type {expected_type.__name__}, "
                f"received {type(data).__name__}"
            )
        return data

    def has(self, artifact_id: str) -> bool:
        with self._lock.read_lock():
            return artifact_id in self._artifacts

    def list(self) -> list[ArtifactInfo]:
        with self._lock.read_lock():
            return [info for info, _ in self._artifacts.values()]

    def remove(self, artifact_id: str) -> None:
        with self._lock.write_lock():
            artifact = self._artifacts.pop(artifact_id, None)
        if artifact is None:
            return
        info, stored_data = artifact
        if info.is_file_backed:
            Path(stored_data).unlink(missing_ok=True)

    def clear(self) -> None:
        with self._lock.write_lock():
            artifacts = list(self._artifacts.values())
            self._artifacts.clear()
        for info, stored_data in artifacts:
            if info.is_file_backed:
                Path(stored_data).unlink(missing_ok=True)


def _byte_size(data: Any) -> int:
    try:
        payload = json.dumps(data, ensure_ascii=False, sort_keys=True).encode("utf-8")
    except (TypeError, ValueError):
        payload = repr(data).encode("utf-8")
    return len(payload)
