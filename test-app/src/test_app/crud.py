from __future__ import annotations

from typing import Any, Dict, Iterable, List, Mapping

from fastapi import HTTPException
from fastapi.encoders import jsonable_encoder
from sqlalchemy import Select, Table, delete, insert, select, update
from sqlalchemy.ext.asyncio import AsyncSession

from .config import get_settings

settings = get_settings()


def _primary_key_column(table: Table):
    pk_columns = list(table.primary_key.columns)
    if len(pk_columns) != 1:
        raise HTTPException(status_code=400, detail=f"Table '{table.name}' must have exactly one primary key")
    return pk_columns[0]


def _coerce_pk_value(column, raw_value: str) -> Any:
    python_type = getattr(column.type, "python_type", str)
    try:
        return python_type(raw_value)
    except (TypeError, ValueError) as exc:
        raise HTTPException(status_code=422, detail="Invalid primary key value") from exc


def _filter_payload(table: Table, payload: Mapping[str, Any]) -> Dict[str, Any]:
    allowed = {column.name for column in table.columns}
    filtered = {key: value for key, value in payload.items() if key in allowed}
    if not filtered:
        raise HTTPException(status_code=422, detail="Payload does not contain any valid columns")
    return filtered


async def fetch_rows(
    session: AsyncSession,
    table: Table,
    *,
    limit: int,
    offset: int,
    order_by: str | None = None,
) -> List[Dict[str, Any]]:
    limit = min(max(limit, 1), settings.max_limit)
    offset = max(offset, 0)

    query: Select = select(table).limit(limit).offset(offset)

    if order_by:
        descending = order_by.startswith("-")
        column_name = order_by[1:] if descending else order_by
        column = table.columns.get(column_name)
        if column is None:
            raise HTTPException(status_code=400, detail=f"Column '{column_name}' does not exist")
        query = query.order_by(column.desc() if descending else column.asc())

    result = await session.execute(query)
    rows = [jsonable_encoder(row) for row in result.mappings().all()]
    return rows


async def fetch_row_by_pk(session: AsyncSession, table: Table, pk_value: str) -> Dict[str, Any]:
    column = _primary_key_column(table)
    coerced = _coerce_pk_value(column, pk_value)
    query = select(table).where(column == coerced)
    result = await session.execute(query)
    record = result.mappings().first()
    if record is None:
        raise HTTPException(status_code=404, detail="Record not found")
    return jsonable_encoder(record)


async def create_row(session: AsyncSession, table: Table, payload: Mapping[str, Any]) -> Dict[str, Any]:
    values = _filter_payload(table, payload)
    stmt = insert(table).values(**values)
    result = await session.execute(stmt)
    await session.commit()

    inserted_pk = result.inserted_primary_key
    if inserted_pk:
        pk_value = str(inserted_pk[0])
        return await fetch_row_by_pk(session, table, pk_value)
    return jsonable_encoder(values)


async def update_row(
    session: AsyncSession,
    table: Table,
    pk_value: str,
    payload: Mapping[str, Any],
) -> Dict[str, Any]:
    column = _primary_key_column(table)
    coerced = _coerce_pk_value(column, pk_value)
    values = _filter_payload(table, payload)

    stmt = update(table).where(column == coerced).values(**values)
    result = await session.execute(stmt)
    if result.rowcount == 0:
        raise HTTPException(status_code=404, detail="Record not found")
    await session.commit()
    return await fetch_row_by_pk(session, table, pk_value)


async def delete_row(session: AsyncSession, table: Table, pk_value: str) -> Dict[str, Any]:
    column = _primary_key_column(table)
    coerced = _coerce_pk_value(column, pk_value)
    stmt = delete(table).where(column == coerced)
    result = await session.execute(stmt)
    if result.rowcount == 0:
        raise HTTPException(status_code=404, detail="Record not found")
    await session.commit()
    return {"deleted": True, "primary_key": pk_value}


__all__ = [
    "fetch_rows",
    "fetch_row_by_pk",
    "create_row",
    "update_row",
    "delete_row",
]
