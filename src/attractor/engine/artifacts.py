from __future__ import annotations

from dataclasses import dataclass
import mimetypes
from pathlib import Path
import shutil
from typing import Iterable

from .context import ReadWriteLock


@dataclass(frozen=True)
class ArtifactInfo:
    path: str
    size_bytes: int
    media_type: str


class ArtifactStore:
    def __init__(self, *, base_dir: str | Path) -> None:
        self._base_dir = Path(base_dir)
        self._base_dir.mkdir(parents=True, exist_ok=True)
        self._lock = ReadWriteLock()
        self._artifacts: list[ArtifactInfo] = []

    def write_text(self, node_id: str, relative_path: str, content: str) -> ArtifactInfo:
        target_path = self._resolve_target_path(node_id, relative_path)
        target_path.parent.mkdir(parents=True, exist_ok=True)
        target_path.write_text(content, encoding="utf-8")
        return self._register(target_path)

    def copy_path(self, node_id: str, source_path: str | Path, relative_path: str) -> ArtifactInfo:
        source = Path(source_path).resolve(strict=True)
        if not source.is_file():
            raise IsADirectoryError(f"Artifact source is not a file: {source}")
        target_path = self._resolve_target_path(node_id, relative_path)
        target_path.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(source, target_path)
        return self._register(target_path)

    def copy_matches(self, node_id: str, cwd: str | Path, patterns: Iterable[str]) -> list[ArtifactInfo]:
        resolved_cwd = Path(cwd).resolve(strict=True)
        copied: list[ArtifactInfo] = []
        seen_paths: set[str] = set()
        for raw_pattern in patterns:
            pattern = raw_pattern.strip()
            if not pattern:
                continue
            self._validate_source_pattern(pattern)
            for match in sorted(resolved_cwd.glob(pattern)):
                resolved_match = match.resolve(strict=True)
                try:
                    source_relative = resolved_match.relative_to(resolved_cwd)
                except ValueError as exc:
                    raise ValueError(
                        f"Artifact source '{resolved_match}' resolves outside tool working directory '{resolved_cwd}'"
                    ) from exc

                if resolved_match.is_dir():
                    for nested in sorted(path for path in resolved_match.rglob("*") if path.is_file()):
                        nested_resolved = nested.resolve(strict=True)
                        nested_relative = nested_resolved.relative_to(resolved_cwd)
                        destination = f"captured/{nested_relative.as_posix()}"
                        if destination in seen_paths:
                            continue
                        copied.append(self.copy_path(node_id, nested_resolved, destination))
                        seen_paths.add(destination)
                    continue

                destination = f"captured/{source_relative.as_posix()}"
                if destination in seen_paths:
                    continue
                copied.append(self.copy_path(node_id, resolved_match, destination))
                seen_paths.add(destination)
        return copied

    def list(self) -> list[ArtifactInfo]:
        with self._lock.read_lock():
            return list(self._artifacts)

    def _register(self, absolute_path: Path) -> ArtifactInfo:
        relative_path = absolute_path.relative_to(self._base_dir).as_posix()
        media_type, _ = mimetypes.guess_type(absolute_path.name)
        info = ArtifactInfo(
            path=f"artifacts/{relative_path}",
            size_bytes=absolute_path.stat().st_size,
            media_type=media_type or "application/octet-stream",
        )
        with self._lock.write_lock():
            self._artifacts.append(info)
        return info

    def _resolve_target_path(self, node_id: str, relative_path: str) -> Path:
        node_key = node_id.strip()
        if not node_key:
            raise ValueError("Artifact writes require a node id")
        normalized = _normalize_relative_path(relative_path)
        return (self._base_dir / node_key / normalized).resolve()

    def _validate_source_pattern(self, pattern: str) -> None:
        candidate = Path(pattern)
        if candidate.is_absolute():
            raise ValueError(f"Artifact source pattern must be relative: {pattern}")
        if any(part == ".." for part in candidate.parts):
            raise ValueError(f"Artifact source pattern must stay within the tool working directory: {pattern}")


def _normalize_relative_path(relative_path: str) -> Path:
    candidate = Path(relative_path.strip())
    if not relative_path.strip():
        raise ValueError("Artifact path must not be empty")
    if candidate.is_absolute():
        raise ValueError(f"Artifact path must be relative: {relative_path}")
    if any(part == ".." for part in candidate.parts):
        raise ValueError(f"Artifact path must not escape the artifact root: {relative_path}")
    return candidate
