set shell := ["bash", "-lc"]

dev:
  docker compose up --build

stop:
  docker compose down

logs:
  docker compose logs -f

restart:
  docker compose down
  docker compose up --build

dot-lint:
  uv run pytest -q tests/integration/test_dot_format_lint.py

test:
  uv run pytest -q
