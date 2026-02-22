from __future__ import annotations

import os
from functools import lru_cache
from typing import List

from pydantic import BaseModel, Field, ValidationError


def _comma_separated(value: str | None, *, fallback: List[str] | None = None) -> List[str]:
    if value is None or value.strip() == "":
        return fallback[:] if fallback else []
    return [item.strip() for item in value.split(",") if item.strip()]


def _int_from_env(name: str, default: int) -> int:
    raw = os.getenv(name)
    if raw is None:
        return default
    try:
        return int(raw)
    except ValueError as exc:
        raise RuntimeError(f"Environment variable {name} must be an integer") from exc


class Settings(BaseModel):
    database_url: str = Field(
        default="mysql+aiomysql://user:password@localhost:3306/app",
        description="SQLAlchemy-compatible async database URL",
    )
    allowed_tables: List[str] = Field(default_factory=list)
    cors_allow_origins: List[str] = Field(default_factory=lambda: ["*"])
    default_limit: int = Field(default=50, ge=1)
    max_limit: int = Field(default=200, ge=1)

    def is_table_allowed(self, table: str) -> bool:
        return not self.allowed_tables or table in self.allowed_tables


@lru_cache
def get_settings() -> Settings:
    database_url = os.getenv("DATABASE_URL")
    allowed_tables = _comma_separated(os.getenv("ALLOWED_TABLES"))
    cors_allow_origins = _comma_separated(os.getenv("CORS_ALLOW_ORIGINS"), fallback=["*"])

    data = {
        "database_url": database_url,
        "allowed_tables": allowed_tables,
        "cors_allow_origins": cors_allow_origins,
        "default_limit": _int_from_env("DEFAULT_PAGE_LIMIT", 50),
        "max_limit": _int_from_env("MAX_PAGE_LIMIT", 200),
    }

    filtered = {k: v for k, v in data.items() if v not in (None, [])}
    try:
        return Settings(**filtered)
    except ValidationError as exc:
        raise RuntimeError(f"Invalid configuration: {exc}") from exc


__all__ = ["Settings", "get_settings"]
