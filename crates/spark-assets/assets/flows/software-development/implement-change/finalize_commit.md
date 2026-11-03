---
type: tool
title: Finalize Commit
config:
  env_map:
    SPARK_RUN_ID: internal.run_id
    SPARK_WORKSPACE_PATH: context.workspace.path
  exit_codes:
    '0': done
  output_map:
    context.workspace.commit: commit
contracts:
  writes_context:
  - context.workspace.commit
routes:
  done:
    to: cleanup_workspace
    description: The bounded work required by this node’s prompt is complete, including its required outputs; this does not assert that the whole workflow objective is achieved.
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
