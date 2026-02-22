from __future__ import annotations

import uvicorn

from .api import create_app


def main() -> None:
    app = create_app()
    uvicorn.run(
        app,
        host="0.0.0.0",
        port=8000,
        reload=False,
    )


__all__ = ["main"]
