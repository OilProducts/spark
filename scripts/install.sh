set -euo pipefail

spark_home="${SPARK_HOME:-$HOME/.spark}"
venv_dir="${spark_home}/venv"
SPARK_HOME="${spark_home}" "${venv_dir}/bin/spark-server" init
