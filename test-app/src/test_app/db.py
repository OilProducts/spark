from __future__ import annotations

from typing import Any, AsyncIterator, Dict, List

from fastapi import HTTPException
from sqlalchemy import MetaData, Table, exc, inspect, text
from sqlalchemy.ext.asyncio import AsyncSession, async_sessionmaker, create_async_engine

from .config import get_settings

settings = get_settings()
engine = create_async_engine(
    settings.database_url,
    pool_pre_ping=True,
    pool_recycle=3600,
)
SessionLocal = async_sessionmaker(engine, expire_on_commit=False)
_table_cache: Dict[str, Table] = {}


async def get_session() -> AsyncIterator[AsyncSession]:
    async with SessionLocal() as session:
        yield session


async def ensure_connection() -> None:
    async with engine.begin() as connection:
        await connection.execute(text("SELECT 1"))


async def list_tables() -> List[str]:
    async with engine.begin() as connection:
        table_names = await connection.run_sync(lambda sync_conn: inspect(sync_conn).get_table_names())
    if settings.allowed_tables:
        return [name for name in table_names if name in settings.allowed_tables]
    return table_names


async def get_table(table_name: str) -> Table:
    if not settings.is_table_allowed(table_name):
        raise HTTPException(status_code=404, detail=f"Table '{table_name}' is not allowed")

    if table_name in _table_cache:
        return _table_cache[table_name]

    local_metadata = MetaData()
    async with engine.begin() as connection:
        try:
            await connection.run_sync(local_metadata.reflect, only=[table_name])
        except exc.NoSuchTableError as exc_info:
            raise HTTPException(status_code=404, detail=f"Table '{table_name}' does not exist") from exc_info

    table = local_metadata.tables.get(table_name)
    if table is None:
        raise HTTPException(status_code=404, detail=f"Table '{table_name}' is not available")

    _table_cache[table_name] = table
    return table


def table_schema(table: Table) -> Dict[str, Any]:
    return {
        "name": table.name,
        "schema": table.schema,
        "primary_key": [column.name for column in table.primary_key.columns],
        "columns": [
            {
                "name": column.name,
                "type": str(column.type),
                "nullable": column.nullable,
                "default": str(column.default.arg) if column.default is not None else None,
                "primary_key": column.primary_key,
            }
            for column in table.columns
        ],
    }


__all__ = [
    "engine",
    "get_session",
    "get_table",
    "list_tables",
    "table_schema",
    "ensure_connection",
]
