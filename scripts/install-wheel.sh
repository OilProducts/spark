set -euo pipefail

just deliverable

spark_home="${SPARK_HOME:-$HOME/.spark}"
venv_dir="${spark_home}/venv"
mkdir -p "${spark_home}"
python3 -m venv "${venv_dir}"
wheel_path="$(ls -t dist/spark-[0-9]*.whl | head -n 1)"
"${venv_dir}/bin/pip" install --upgrade --force-reinstall "${wheel_path}"
