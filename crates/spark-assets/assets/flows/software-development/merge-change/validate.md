---
type: tool
title: Validate
config:
  env_map:
    SPARK_INTEGRATION_PATH: context.integration.path
    SPARK_VALIDATION_COMMAND: context.task.validation_command
routes:
- to: finish_merge
  condition: outcome=success
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
