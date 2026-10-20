---
type: tool
title: Validate
config:
  env_map:
    SPARK_VALIDATION_COMMAND: context.task.validation_command
    SPARK_WORKSPACE_PATH: context.workspace.path
routes:
- to: count_validate_attempt
  label: Fix
  condition: outcome=fail
- to: finalize_commit
  condition: outcome=success
---

```sh
set -eu
cd "$SPARK_WORKSPACE_PATH"
log=$(mktemp)
trap 'rm -f "$log"' EXIT
if sh -ec "$SPARK_VALIDATION_COMMAND" >"$log" 2>&1; then
  tail -c 2000 "$log"
else
  tail -c 20000 "$log"
  exit 1
fi
```
