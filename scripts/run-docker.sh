set -euo pipefail

real_home=""
if command -v getent >/dev/null 2>&1; then
  real_home="$(getent passwd "$(id -u)" | cut -d: -f6 || true)"
elif command -v dscl >/dev/null 2>&1; then
  real_home="$(dscl . -read "/Users/$(id -un)" NFSHomeDirectory 2>/dev/null | awk '{print $2}' || true)"
fi
real_home="${real_home:-$HOME}"

spark_home="${SPARK_DOCKER_HOME:-$real_home/.spark-docker}"
host_codex_home="${CODEX_HOME:-$real_home/.codex}"
docker_codex_home="${spark_home}/runtime/codex/.codex"
env_file="${spark_home}/config/provider.env"
export SPARK_DOCKER_HOME="${spark_home}"
export SPARK_DOCKER_HOST_UID="${SPARK_DOCKER_HOST_UID:-$(id -u)}"
export SPARK_DOCKER_HOST_GID="${SPARK_DOCKER_HOST_GID:-$(id -g)}"
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
