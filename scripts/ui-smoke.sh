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
