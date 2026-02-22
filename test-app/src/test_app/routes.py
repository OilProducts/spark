from __future__ import annotations

from typing import Any, Dict

from fastapi import APIRouter, Body, Depends, Query
from sqlalchemy import Table
from sqlalchemy.ext.asyncio import AsyncSession

from .config import get_settings
from .crud import create_row, delete_row, fetch_row_by_pk, fetch_rows, update_row
from .db import ensure_connection, get_session, get_table, list_tables, table_schema

router = APIRouter(prefix="/api", tags=["crud"])
settings = get_settings()


async def table_dependency(table_name: str) -> Table:
    return await get_table(table_name)


@router.get("/health")
async def healthcheck() -> Dict[str, Any]:
    await ensure_connection()
    return {"status": "ok"}


@router.get("/tables")
async def tables() -> Dict[str, Any]:
    names = await list_tables()
    return {"tables": names}


@router.get("/tables/{table_name}/schema")
async def schema(table: Table = Depends(table_dependency)) -> Dict[str, Any]:
    return table_schema(table)


@router.get("/tables/{table_name}/rows")
async def rows(
    *,
    limit: int = Query(default=settings.default_limit, ge=1, le=settings.max_limit),
    offset: int = Query(default=0, ge=0),
    order_by: str | None = Query(default=None, description="Column to order by; prefix with '-' for DESC"),
    session: AsyncSession = Depends(get_session),
    table: Table = Depends(table_dependency),
) -> Dict[str, Any]:
    records = await fetch_rows(session, table, limit=limit, offset=offset, order_by=order_by)
    return {
        "items": records,
        "count": len(records),
        "limit": limit,
        "offset": offset,
    }


@router.get("/tables/{table_name}/rows/{pk}")
async def row_detail(
    pk: str,
    session: AsyncSession = Depends(get_session),
    table: Table = Depends(table_dependency),
) -> Dict[str, Any]:
    record = await fetch_row_by_pk(session, table, pk)
    return record


@router.post("/tables/{table_name}/rows", status_code=201)
async def create(
    payload: Dict[str, Any] = Body(..., description="JSON payload with column/value pairs"),
    session: AsyncSession = Depends(get_session),
    table: Table = Depends(table_dependency),
) -> Dict[str, Any]:
    return await create_row(session, table, payload)


@router.put("/tables/{table_name}/rows/{pk}")
async def update(
    pk: str,
    payload: Dict[str, Any] = Body(..., description="JSON payload with column/value pairs"),
    session: AsyncSession = Depends(get_session),
    table: Table = Depends(table_dependency),
) -> Dict[str, Any]:
    return await update_row(session, table, pk, payload)


@router.delete("/tables/{table_name}/rows/{pk}")
async def delete(
    pk: str,
    session: AsyncSession = Depends(get_session),
    table: Table = Depends(table_dependency),
) -> Dict[str, Any]:
    return await delete_row(session, table, pk)


__all__ = ["router"]
