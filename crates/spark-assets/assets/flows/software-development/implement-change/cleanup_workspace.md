---
type: tool
title: Cleanup Workspace
config:
  env_map:
    SPARK_WORKSPACE_PATH: context.workspace.path
routes:
- to: done
  condition: outcome=success
---

```sh
set -eu
git worktree remove "$SPARK_WORKSPACE_PATH"
```
