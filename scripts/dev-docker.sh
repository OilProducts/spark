set -euo pipefail

spark_home="${SPARK_HOME:-$HOME/.spark-dev}"
env_file="${spark_home}/config/provider.env"
if [[ -f "${env_file}" ]]; then
  set -a
  source "${env_file}"
  set +a
fi

docker compose up --build
