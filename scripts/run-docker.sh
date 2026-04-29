set -euo pipefail

spark_home="${SPARK_HOME:-$HOME/.spark}"
env_file="${spark_home}/config/provider.env"
if [[ -f "${env_file}" ]]; then
  set -a
  source "${env_file}"
  set +a
fi

docker compose -f compose.package.yaml up --build
