set -euo pipefail

spark_home="${SPARK_HOME:-$HOME/.spark}"
venv_dir="${spark_home}/venv"
spark_host="${SPARK_HOST:-0.0.0.0}"
spark_port="${SPARK_PORT:-8000}"
"${venv_dir}/bin/spark-server" service install --host "${spark_host}" --port "${spark_port}" --data-dir "${spark_home}"
