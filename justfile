set shell := ["bash", "-lc"]

[private]
frontend-deps:
  if [[ ! -x frontend/node_modules/.bin/tsc || ! -x frontend/node_modules/.bin/vite || ! -x frontend/node_modules/.bin/vitest || ! -x frontend/node_modules/.bin/playwright ]]; then echo "Installing frontend dependencies with npm ci..." >&2; npm --prefix frontend ci; fi

setup:
  uv sync --dev
  npm --prefix frontend ci

clean:
  rm -rf dist frontend/dist frontend/node_modules/.tmp

dev-docker:
  bash scripts/dev-docker.sh

run-docker:
  bash scripts/run-docker.sh

dev-run: frontend-deps
  bash scripts/dev-run.sh

dev-init:
  spark_home="${SPARK_HOME:-$HOME/.spark-dev}"; SPARK_HOME="${spark_home}" uv run spark-server init

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
  bash scripts/ui-smoke.sh

frontend-build: frontend-deps
  npm --prefix frontend run build

deliverable: frontend-deps
  uv run python scripts/build_deliverable.py

build:
  just deliverable

[private]
install-wheel:
  bash scripts/install-wheel.sh

install: install-wheel
  bash scripts/install.sh

install-systemd: install-wheel
  bash scripts/install-systemd.sh
