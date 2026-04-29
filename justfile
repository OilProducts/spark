set shell := ["bash", "-lc"]

[private]
frontend-deps:
  if [[ ! -x frontend/node_modules/.bin/tsc || ! -x frontend/node_modules/.bin/vite || ! -x frontend/node_modules/.bin/vitest || ! -x frontend/node_modules/.bin/playwright ]]; then echo "Installing frontend dependencies with npm ci..." >&2; npm --prefix frontend ci; fi

setup:
  uv sync --dev
  npm --prefix frontend ci

dev-docker:
  bash scripts/dev-docker.sh

run-docker:
  bash scripts/run-docker.sh

dev-run: frontend-deps
  bash scripts/dev-run.sh

test: frontend-deps
  uv run pytest -q
  npm --prefix frontend run test:unit

deliverable:
  docker build -f Dockerfile.wheel -t spark-wheel-builder .
  docker run --rm --user "$(id -u):$(id -g)" -e HOME=/tmp/spark-builder-home -e UV_CACHE_DIR=/tmp/uv-cache -e UV_PROJECT_ENVIRONMENT=/tmp/spark-builder-venv -e npm_config_cache=/tmp/npm-cache -v "$PWD:/workspace" -w /workspace spark-wheel-builder uv run python scripts/build_deliverable.py

[private]
install-wheel: deliverable
  spark_home="${SPARK_HOME:-$HOME/.spark}"; venv_dir="${spark_home}/venv"; mkdir -p "${spark_home}"; python3 -m venv "${venv_dir}"; wheel_path="$(ls -t dist/spark-[0-9]*.whl | head -n 1)"; "${venv_dir}/bin/pip" install --upgrade --force-reinstall "${wheel_path}"

install: install-wheel
  spark_home="${SPARK_HOME:-$HOME/.spark}"; venv_dir="${spark_home}/venv"; SPARK_HOME="${spark_home}" "${venv_dir}/bin/spark-server" init

install-systemd: install-wheel
  spark_home="${SPARK_HOME:-$HOME/.spark}"; venv_dir="${spark_home}/venv"; spark_host="${SPARK_HOST:-0.0.0.0}"; spark_port="${SPARK_PORT:-8000}"; "${venv_dir}/bin/spark-server" service install --host "${spark_host}" --port "${spark_port}" --data-dir "${spark_home}"
