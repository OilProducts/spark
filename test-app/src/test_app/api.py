from __future__ import annotations

from pathlib import Path

from fastapi import FastAPI
from fastapi.middleware.cors import CORSMiddleware
from fastapi.responses import FileResponse
from fastapi.staticfiles import StaticFiles

from .config import get_settings
from .routes import router as crud_router


def create_app() -> FastAPI:
    settings = get_settings()
    app = FastAPI(title="Generic MySQL CRUD API", version="1.0.0")

    app.add_middleware(
        CORSMiddleware,
        allow_origins=settings.cors_allow_origins,
        allow_credentials=True,
        allow_methods=["*"],
        allow_headers=["*"],
    )

    static_dir = Path(__file__).resolve().parent / "static"
    if static_dir.exists():
        app.mount("/static", StaticFiles(directory=static_dir), name="static")

        index_file = static_dir / "index.html"

        @app.get("/", include_in_schema=False)
        async def ui_root() -> FileResponse:
            return FileResponse(index_file)

    app.include_router(crud_router)
    return app


app = create_app()


__all__ = ["create_app", "app"]
