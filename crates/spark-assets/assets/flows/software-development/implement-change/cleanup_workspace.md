---
type: tool
title: Cleanup Workspace
config:
  env_map:
    SPARK_WORKSPACE_PATH: context.workspace.path
  exit_codes:
    '0': done
routes:
  done:
    to: done
    description: The bounded work required by this node’s prompt is complete, including its required outputs; this does not assert that the whole workflow objective is achieved.
---

```sh
set -eu
git worktree remove "$SPARK_WORKSPACE_PATH"
```
