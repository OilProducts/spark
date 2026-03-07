set shell := ["bash", "-lc"]

dev:
  docker compose up --build

run:
  #!/usr/bin/env bash
  set -euo pipefail
  trap 'kill "${backend_pid:-}" "${frontend_pid:-}" 2>/dev/null || true; wait || true' EXIT INT TERM
  uv run sparkspawn serve --host 127.0.0.1 --port 8000 --reload &
  backend_pid=$!
  npm --prefix frontend run dev -- --host 127.0.0.1 &
  frontend_pid=$!
  while kill -0 "${backend_pid}" 2>/dev/null && kill -0 "${frontend_pid}" 2>/dev/null; do sleep 1; done
  if ! kill -0 "${backend_pid}" 2>/dev/null; then wait "${backend_pid}"; else wait "${frontend_pid}"; fi

stop:
  docker compose down

logs:
  docker compose logs -f

restart:
  docker compose down
  docker compose up --build

dot-lint:
  uv run pytest -q tests/integration/test_dot_format_lint.py

parser-unsupported-grammar:
  uv run pytest -q tests/dsl/test_parser.py -k unsupported_grammar_regression

test:
  uv run pytest -q

frontend-unit:
  npm --prefix frontend run test:unit

ui-smoke:
  npm --prefix frontend run ui:smoke

frontend-build:
  npm --prefix frontend run build

sync-ui-dist:
  ./scripts/sync_ui_dist.sh

wheel:
  uv build

build:
  npm --prefix frontend run build
  ./scripts/sync_ui_dist.sh
  uv build
