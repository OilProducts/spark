---
type: tool
title: Finish Merge
config:
  env_map:
    SPARK_INTEGRATION_PATH: context.integration.path
    SPARK_RUN_ID: internal.run_id
    SPARK_SOURCE_REF: context.task.source_ref
    SPARK_TARGET_COMMIT: context.integration.target_commit
  output_map:
    context.integration.merge_commit: merge_commit
contracts:
  writes_context:
  - context.integration.merge_commit
routes:
- to: done
  condition: outcome=success
---

```sh
set -eu
if [ -n "$(git status --porcelain --untracked-files=no)" ]; then
  echo "target checkout became dirty during integration" >&2
  exit 1
fi
current_target=$(git rev-parse HEAD)
if [ "$current_target" != "$SPARK_TARGET_COMMIT" ]; then
  echo "target moved during integration: $SPARK_TARGET_COMMIT -> $current_target" >&2
  exit 1
fi
git -C "$SPARK_INTEGRATION_PATH" add --all
git -C "$SPARK_INTEGRATION_PATH" commit -m "Merge ${SPARK_SOURCE_REF} (spark run ${SPARK_RUN_ID})" >/dev/null
merge_commit=$(git -C "$SPARK_INTEGRATION_PATH" rev-parse HEAD)
git merge --ff-only "$merge_commit" >/dev/null
git worktree remove "$SPARK_INTEGRATION_PATH"
printf '{"merge_commit":"%s"}' "$merge_commit"
```
