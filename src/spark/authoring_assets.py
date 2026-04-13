from __future__ import annotations

from pathlib import Path


GUIDES_DIR_NAME = "guides"
DOT_AUTHORING_GUIDE_NAME = "dot-authoring.md"
SPARK_OPERATIONS_GUIDE_NAME = "spark-operations.md"


def dot_authoring_guide_path() -> Path:
    path = Path(__file__).resolve().parent / GUIDES_DIR_NAME / DOT_AUTHORING_GUIDE_NAME
    if not path.exists():
        raise RuntimeError(f"DOT authoring guide is unavailable: {path}")
    return path


def spark_operations_guide_path() -> Path:
    path = Path(__file__).resolve().parent / GUIDES_DIR_NAME / SPARK_OPERATIONS_GUIDE_NAME
    if not path.exists():
        raise RuntimeError(f"Spark operations guide is unavailable: {path}")
    return path
