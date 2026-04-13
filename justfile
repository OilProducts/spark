set shell := ["bash", "-lc"]

[private]
frontend-deps:
  #!/usr/bin/env bash
  set -euo pipefail
  if [[ ! -x frontend/node_modules/.bin/tsc || ! -x frontend/node_modules/.bin/vite || ! -x frontend/node_modules/.bin/vitest || ! -x frontend/node_modules/.bin/playwright ]]; then
    echo "Installing frontend dependencies with npm ci..." >&2
    npm --prefix frontend ci
  fi

setup:
  uv sync --dev
  npm --prefix frontend ci

clean:
  rm -rf dist frontend/dist frontend/node_modules/.tmp

dev:
  docker compose up --build

dev-docker:
  just dev

run: frontend-deps
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

dev-run:
  just run

init-dev:
  #!/usr/bin/env bash
  set -euo pipefail
  spark_home="${SPARK_HOME:-$HOME/.spark-dev}"
  SPARK_HOME="${spark_home}" uv run spark-server init

dev-init:
  just init-dev

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

test: frontend-deps
  uv run pytest -q
  npm --prefix frontend run test:unit

frontend-unit: frontend-deps
  npm --prefix frontend run test:unit

ui-smoke: frontend-deps
  #!/usr/bin/env bash
  set -euo pipefail
  spark_home="${SPARK_HOME:-$HOME/.spark-dev}"
  spark_port="${SPARK_PORT:-8000}"
  backend_log="${SPARK_UI_SMOKE_BACKEND_LOG:-/tmp/spark-ui-smoke-backend.log}"
  SPARK_HOME="${spark_home}" uv run uvicorn spark.app:app --host 127.0.0.1 --port "${spark_port}" --log-level warning > "${backend_log}" 2>&1 &
  backend_pid=$!
  cleanup() {
    if kill -0 "${backend_pid}" 2>/dev/null; then
      kill "${backend_pid}" || true
    fi
    wait "${backend_pid}" 2>/dev/null || true
  }
  trap cleanup EXIT INT TERM
  for _ in $(seq 1 60); do
    if curl -sf "http://127.0.0.1:${spark_port}/attractor/status" >/dev/null; then
      break
    fi
    sleep 1
  done
  curl -sf "http://127.0.0.1:${spark_port}/attractor/status" >/dev/null
  npm --prefix frontend run ui:smoke

frontend-build: frontend-deps
  npm --prefix frontend run build

deliverable: frontend-deps
  uv run python scripts/build_deliverable.py

build:
  just deliverable

install:
  #!/usr/bin/env bash
  set -euo pipefail
  just deliverable
  spark_home="${SPARK_HOME:-$HOME/.spark}"
  venv_dir="${spark_home}/venv"
  mkdir -p "${spark_home}"
  python3 -m venv "${venv_dir}"
  wheel_path="$(ls -t dist/spark-[0-9]*.whl | head -n 1)"
  "${venv_dir}/bin/pip" install --upgrade --force-reinstall "${wheel_path}"
  SPARK_HOME="${spark_home}" "${venv_dir}/bin/spark-server" init
