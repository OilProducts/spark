set shell := ["bash", "-lc"]

dev:
  docker compose up --build

run:
  #!/usr/bin/env bash
  set -euo pipefail
  trap 'kill "${backend_pid:-}" "${frontend_pid:-}" 2>/dev/null || true; wait || true' EXIT INT TERM
  spark_home="${SPARK_HOME:-$HOME/.spark-dev}"
  spark_port="${SPARK_PORT:-8010}"
  backend_url="${VITE_BACKEND_URL:-http://127.0.0.1:${spark_port}}"
  SPARK_HOME="${spark_home}" uv run spark-server serve --host 127.0.0.1 --port "${spark_port}" --reload &
  backend_pid=$!
  VITE_BACKEND_URL="${backend_url}" npm --prefix frontend run dev -- --host 127.0.0.1 &
  frontend_pid=$!
  while kill -0 "${backend_pid}" 2>/dev/null && kill -0 "${frontend_pid}" 2>/dev/null; do sleep 1; done
  if ! kill -0 "${backend_pid}" 2>/dev/null; then wait "${backend_pid}"; else wait "${frontend_pid}"; fi

init-dev:
  #!/usr/bin/env bash
  set -euo pipefail
  spark_home="${SPARK_HOME:-$HOME/.spark-dev}"
  SPARK_HOME="${spark_home}" uv run spark-server init

stop:
  docker compose down

logs:
  docker compose logs -f

restart:
  docker compose down
  docker compose up --build

dot-lint:
  uv run pytest -q tests/repo_hygiene/test_dot_format_lint.py

parser-unsupported-grammar:
  uv run pytest -q tests/dsl/test_parser.py -k unsupported_grammar_regression

test:
  uv run pytest -q
  npm --prefix frontend run test:unit

frontend-unit:
  npm --prefix frontend run test:unit

ui-smoke:
  npm --prefix frontend run ui:smoke

frontend-build:
  npm --prefix frontend run build

wheel:
  uv build

build:
  npm --prefix frontend run build
  uv build

install:
  #!/usr/bin/env bash
  set -euo pipefail
  just build
  spark_home="${SPARK_HOME:-$HOME/.spark}"
  venv_dir="${spark_home}/venv"
  mkdir -p "${spark_home}"
  python3 -m venv "${venv_dir}"
  wheel_path="$(ls -t dist/spark-[0-9]*.whl | head -n 1)"
  "${venv_dir}/bin/pip" install --upgrade --force-reinstall "${wheel_path}"
  SPARK_HOME="${spark_home}" "${venv_dir}/bin/spark-server" init
