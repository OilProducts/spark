from __future__ import annotations

from pathlib import Path

from spark_common.settings import Settings


def resolve_default_ui_dir(project_root: Path) -> Path | None:
    source_dist = project_root / "frontend" / "dist"
    if (source_dist / "index.html").exists():
        return source_dist.resolve(strict=False)

    packaged_dist = Path(__file__).resolve().parent / "ui_dist"
    if (packaged_dist / "index.html").exists():
        return packaged_dist.resolve(strict=False)

    return None


def resolve_ui_dir(settings: Settings) -> Path | None:
    if settings.ui_dir:
        index_path = settings.ui_dir / "index.html"
        if index_path.exists():
            return settings.ui_dir
    return resolve_default_ui_dir(settings.project_root)


def resolve_ui_index_path(settings: Settings) -> Path | None:
    ui_dir = resolve_ui_dir(settings)
    if ui_dir is None:
        return None
    index_path = ui_dir / "index.html"
    if index_path.exists():
        return index_path
    return None


def resolve_ui_asset_path(settings: Settings, relative_path: str) -> Path | None:
    ui_dir = resolve_ui_dir(settings)
    if ui_dir is None:
        return None
    candidate = ui_dir / relative_path
    if candidate.exists():
        return candidate
    return None
