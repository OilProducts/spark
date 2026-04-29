set -euo pipefail

spark-server init --data-dir /spark
exec spark-server serve --host 0.0.0.0 --port 8000 --data-dir /spark
