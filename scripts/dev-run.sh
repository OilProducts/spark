set -euo pipefail

trap 'kill "${backend_pid:-}" "${frontend_pid:-}" 2>/dev/null || true; wait || true' EXIT INT TERM

spark_home="${SPARK_HOME:-$HOME/.spark-dev}"
spark_port="${SPARK_PORT:-8010}"
backend_url="${VITE_BACKEND_URL:-http://127.0.0.1:${spark_port}}"
env_file="${spark_home}/config/provider.env"

backend() {
  if [[ -f "${env_file}" ]]; then
    set -a
    source "${env_file}"
    set +a
  fi
  SPARK_HOME="${spark_home}" uv run spark-server serve --host 127.0.0.1 --port "${spark_port}" --reload
}

backend &
backend_pid=$!

VITE_BACKEND_URL="${backend_url}" npm --prefix frontend run dev -- --host 127.0.0.1 &
frontend_pid=$!

while kill -0 "${backend_pid}" 2>/dev/null && kill -0 "${frontend_pid}" 2>/dev/null; do
  sleep 1
done

if ! kill -0 "${backend_pid}" 2>/dev/null; then
  wait "${backend_pid}"
else
  wait "${frontend_pid}"
fi
