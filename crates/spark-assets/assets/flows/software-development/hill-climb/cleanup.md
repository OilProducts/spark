---
type: tool
title: Clean Up
config:
  decision_field: decision
  env_map:
    SPARK_OPUS_PATH: context.workspace.opus_path
    SPARK_OUTCOME: context.climb.outcome
    SPARK_SOL_PATH: context.workspace.sol_path
routes:
  exhausted:
    to: unmet
    description: The judges' checkouts are removed; the target was not reached.
  passed:
    to: done
    description: The judges' checkouts are removed; every judge reached the target.
---

```sh
# The climber's worktree stays for review; the judges' checkouts are removed.
set -eu
for dir in "$SPARK_OPUS_PATH" "$SPARK_SOL_PATH"; do
  if git -C "$dir" rev-parse --git-dir >/dev/null 2>&1; then
    git worktree remove --force "$dir" >&2
  fi
done
printf '{"decision":"%s"}\n' "$SPARK_OUTCOME"
```
