---
type: tool
title: Locate Run
config:
  env_map:
    SPARK_RUNS_ROOT: internal.runs_dir
    SPARK_TARGET_RUN_ID: context.request.run_id
  exit_codes:
    '0': done
  output_map:
    context.retro.run_dir: run_dir
contracts:
  writes_context:
  - context.retro.run_dir
routes:
  done:
    to: diagnose
    description: The bounded work required by this node’s prompt is complete, including its required outputs; this does not assert that the whole workflow objective is achieved.
---

```sh
set -eu
# Default runs live in <project-id>/runs/<run-id>, an external runs root
# holds <project-id>/<run-id>.
run_dir=
for dir in "$SPARK_RUNS_ROOT"/*/runs/"$SPARK_TARGET_RUN_ID" "$SPARK_RUNS_ROOT"/*/"$SPARK_TARGET_RUN_ID"; do
  if [ -f "$dir/run.json" ]; then
    run_dir=$dir
    break
  fi
done
if [ -z "$run_dir" ]; then
  echo "run $SPARK_TARGET_RUN_ID not found under $SPARK_RUNS_ROOT" >&2
  exit 1
fi
printf '{"run_dir":"%s"}' "$run_dir"
```
