---
type: tool
title: Locate Mission
config:
  env_map:
    SPARK_RUNS_ROOT: internal.runs_dir
    SPARK_TARGET_MISSION_ID: context.request.mission_id
  exit_codes:
    '0': done
  output_map:
    context.retro.project_id: project_id
    context.retro.runs_dir: runs_dir
    context.retro.state_db: state_db
contracts:
  writes_context:
  - context.retro.state_db
  - context.retro.project_id
  - context.retro.runs_dir
routes:
  done:
    to: review_mission
    description: The bounded work required by this node’s prompt is complete, including its required outputs; this does not assert that the whole workflow objective is achieved.
---

```sh
set -eu
command -v python3 >/dev/null 2>&1 || { echo "locate_mission: python3 is required" >&2; exit 1; }
python3 - <<'PY'
import json, os, sqlite3, sys

runs_root = os.environ["SPARK_RUNS_ROOT"]
mission_id = os.environ["SPARK_TARGET_MISSION_ID"]
spark_home = os.environ.get("SPARK_HOME") or os.path.dirname(os.path.abspath(runs_root))
state_db = os.path.join(spark_home, "state.sqlite3")

if not os.path.isfile(state_db):
    print(f"state database not found at {state_db}", file=sys.stderr)
    sys.exit(1)

conn = sqlite3.connect(f"file:{state_db}?mode=ro", uri=True)
try:
    row = conn.execute(
        "SELECT project_id FROM missions WHERE mission_id = ?",
        (mission_id,),
    ).fetchone()
finally:
    conn.close()

if not row:
    print(f"mission {mission_id} not found in {state_db}", file=sys.stderr)
    sys.exit(1)

project_id = row[0]
default_runs = os.path.join(runs_root, project_id, "runs")
external_runs = os.path.join(runs_root, project_id)
runs_dir = default_runs if os.path.isdir(default_runs) else external_runs

print(json.dumps({
    "state_db": state_db,
    "project_id": project_id,
    "runs_dir": runs_dir,
}))
PY
```
