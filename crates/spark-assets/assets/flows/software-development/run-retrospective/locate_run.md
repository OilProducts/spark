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
run_dir=$(find "$SPARK_RUNS_ROOT" -mindepth 2 -maxdepth 2 -type d -name "$SPARK_TARGET_RUN_ID" | head -1)
if [ -z "$run_dir" ]; then
  echo "run $SPARK_TARGET_RUN_ID not found under $SPARK_RUNS_ROOT" >&2
  exit 1
fi
printf '{"run_dir":"%s"}' "$run_dir"
```
