---
type: tool
title: Finalize Commit
config:
  env_map:
    SPARK_RUN_ID: internal.run_id
    SPARK_WORKSPACE_PATH: context.workspace.path
  output_map:
    context.workspace.commit: commit
contracts:
  writes_context:
  - context.workspace.commit
routes:
- to: cleanup_workspace
  condition: outcome=success
---

```sh
set -eu
cd "$SPARK_WORKSPACE_PATH"
git add --all
if ! git diff --cached --quiet; then
  git commit -m "Complete implement-change run ${SPARK_RUN_ID}" >/dev/null
fi
printf '{"commit":"%s"}' "$(git rev-parse HEAD)"
```
