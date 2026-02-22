# Generic MySQL CRUD (FastAPI + Tailwind)

This project exposes a FastAPI-powered CRUD API backed by any MySQL database and pairs it with a lightweight Tailwind + vanilla JS UI for quick table exploration.

## Features

- Generic CRUD over any MySQL table (single-column primary keys)
- Automatic schema reflection per table
- Environment-based configuration for database URL, CORS, and table allow-list
- Tailwind-powered UI to inspect schema, query rows, and create/update/delete entries
- Packaged as a uv project with console entry (`test-app`) that launches both API and UI

## Getting Started

1. **Set configuration** via environment variables (examples shown with defaults):
   ```bash
   export DATABASE_URL="mysql+aiomysql://user:password@localhost:3306/app"
   export ALLOWED_TABLES="users,orders"   # optional, allow all when unset
   export CORS_ALLOW_ORIGINS="http://localhost:5173"  # or "*"
   ```

2. **Install dependencies** (already added to `pyproject.toml`):
   ```bash
   uv sync
   ```

3. **Run the server**:
   ```bash
   uv run test-app
   ```
   The API will listen on `http://0.0.0.0:8000`. Visit `http://localhost:8000/` for the UI and `http://localhost:8000/docs` for the OpenAPI schema.

## API Overview

| Endpoint | Description |
| --- | --- |
| `GET /api/health` | Connection health check |
| `GET /api/tables` | List available tables (filtered by `ALLOWED_TABLES`) |
| `GET /api/tables/{table}/schema` | Inspect table metadata |
| `GET /api/tables/{table}/rows` | Paginated row fetch with `limit`, `offset`, `order_by` |
| `GET /api/tables/{table}/rows/{pk}` | Fetch a single row by primary key |
| `POST /api/tables/{table}/rows` | Insert a record (JSON body = column/value map) |
| `PUT /api/tables/{table}/rows/{pk}` | Update a record |
| `DELETE /api/tables/{table}/rows/{pk}` | Delete a record |

The backend enforces single-column primary keys for update/delete simplicity. Adjust `test_app/crud.py` if you need composite key support.

## UI Notes

- UI uses Tailwind via CDN plus a small design system in `.interface-design/system.md` (Utility & Function direction, 4px spacing base).
- Filters include `limit`, `offset`, and `order_by` (prefix with `-` for descending) and call the same API endpoints.
- All requests go through `/api`, so you can host the UI separately by adjusting `apiBase` in `static/app.js`.

## Development

- Run formatting/tests as needed:
  ```bash
  uv run ruff format .
  uv run pytest
  ```
- To modify design tokens, update `.interface-design/system.md` and keep CSS/JS aligned.
