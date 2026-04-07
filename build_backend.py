from __future__ import annotations

from contextlib import contextmanager
from pathlib import Path
import shutil
import tempfile
from typing import Iterator

from setuptools import build_meta as _setuptools_build_meta


_ROOT = Path(__file__).resolve().parent
_FRONTEND_DIST = _ROOT / "frontend" / "dist"
_PACKAGED_UI_DIST = _ROOT / "src" / "spark_app" / "ui_dist"


def _ui_bundle_exists(path: Path) -> bool:
    return (path / "index.html").is_file()


@contextmanager
def _prepared_ui_bundle() -> Iterator[None]:
    staged_from_source = False
    backup_root: Path | None = None
    backup_ui_dist: Path | None = None

    if _ui_bundle_exists(_FRONTEND_DIST):
        if _PACKAGED_UI_DIST.exists():
            backup_root = Path(tempfile.mkdtemp(prefix="spark-ui-dist-backup-"))
            backup_ui_dist = backup_root / "ui_dist"
            shutil.copytree(_PACKAGED_UI_DIST, backup_ui_dist)
            shutil.rmtree(_PACKAGED_UI_DIST)

        _PACKAGED_UI_DIST.parent.mkdir(parents=True, exist_ok=True)
        shutil.copytree(_FRONTEND_DIST, _PACKAGED_UI_DIST)
        staged_from_source = True
    elif not _ui_bundle_exists(_PACKAGED_UI_DIST):
        raise RuntimeError(
            "Spark build requires bundled UI assets. "
            "Build the frontend first with 'npm --prefix frontend run build' when building from a source checkout."
        )

    try:
        yield
    finally:
        if staged_from_source:
            shutil.rmtree(_PACKAGED_UI_DIST, ignore_errors=True)
            if backup_ui_dist is not None and backup_ui_dist.exists():
                shutil.copytree(backup_ui_dist, _PACKAGED_UI_DIST)
        if backup_root is not None:
            shutil.rmtree(backup_root, ignore_errors=True)


def get_requires_for_build_sdist(config_settings=None):
    return _setuptools_build_meta.get_requires_for_build_sdist(config_settings)


def get_requires_for_build_wheel(config_settings=None):
    return _setuptools_build_meta.get_requires_for_build_wheel(config_settings)


def prepare_metadata_for_build_wheel(metadata_directory, config_settings=None):
    return _setuptools_build_meta.prepare_metadata_for_build_wheel(metadata_directory, config_settings)


def build_sdist(sdist_directory, config_settings=None):
    with _prepared_ui_bundle():
        return _setuptools_build_meta.build_sdist(sdist_directory, config_settings)


def build_wheel(wheel_directory, config_settings=None, metadata_directory=None):
    with _prepared_ui_bundle():
        return _setuptools_build_meta.build_wheel(wheel_directory, config_settings, metadata_directory)


if hasattr(_setuptools_build_meta, "get_requires_for_build_editable"):
    def get_requires_for_build_editable(config_settings=None):
        return _setuptools_build_meta.get_requires_for_build_editable(config_settings)


if hasattr(_setuptools_build_meta, "prepare_metadata_for_build_editable"):
    def prepare_metadata_for_build_editable(metadata_directory, config_settings=None):
        return _setuptools_build_meta.prepare_metadata_for_build_editable(metadata_directory, config_settings)


if hasattr(_setuptools_build_meta, "build_editable"):
    def build_editable(wheel_directory, config_settings=None, metadata_directory=None):
        return _setuptools_build_meta.build_editable(wheel_directory, config_settings, metadata_directory)
