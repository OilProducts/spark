---
type: tool
title: Validate
config:
  env_map:
    SPARK_INTEGRATION_PATH: context.integration.path
    SPARK_VALIDATION_COMMAND: context.task.validation_command
  exit_codes:
    '0': done
routes:
  done:
    to: finish_merge
    description: The bounded work required by this node’s prompt is complete, including its required outputs; this does not assert that the whole workflow objective is achieved.
---

```sh
set -eu
cd "$SPARK_INTEGRATION_PATH"
if [ -n "$(git diff --name-only --diff-filter=U)" ]; then
  echo "unresolved merge conflicts remain" >&2
  exit 1
fi
sh -ec "$SPARK_VALIDATION_COMMAND"
```
