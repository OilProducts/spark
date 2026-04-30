set -euo pipefail

real_home="$(getent passwd "$(id -u)" | cut -d: -f6)"
real_home="${real_home:-$HOME}"

spark_home="${SPARK_DOCKER_HOME:-$real_home/.spark-docker}"
host_codex_home="${CODEX_HOME:-$real_home/.codex}"
docker_codex_home="${spark_home}/runtime/codex/.codex"
env_file="${spark_home}/config/provider.env"
export SPARK_DOCKER_HOME="${spark_home}"
if [[ -f "${env_file}" ]]; then
  set -a
  source "${env_file}"
  set +a
fi

mkdir -p "${docker_codex_home}"
for codex_file in auth.json config.toml; do
  host_path="${host_codex_home}/${codex_file}"
  docker_path="${docker_codex_home}/${codex_file}"
  if [[ -f "${host_path}" && ! -e "${docker_path}" ]]; then
    cp "${host_path}" "${docker_path}"
    chmod 600 "${docker_path}" 2>/dev/null || true
  fi
done

docker compose -f compose.package.yaml up --build
