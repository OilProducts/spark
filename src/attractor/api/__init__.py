"""FastAPI integration for Attractor."""

from typing import Any


__all__ = ["attractor_app"]


def __getattr__(name: str) -> Any:
    if name == "attractor_app":
        from .server import attractor_app

        return attractor_app
    raise AttributeError(f"module {__name__!r} has no attribute {name!r}")
