---
type: tool
title: Prepare
config:
  env_map:
    SPARK_RUN_ID: internal.run_id
  exit_codes:
    '0': done
  output_map:
    context.climb.history: history
    context.climb.round: round
    context.workspace.branch: branch
    context.workspace.opus_path: opus_path
    context.workspace.path: path
    context.workspace.sol_path: sol_path
contracts:
  writes_context:
  - context.workspace.path
  - context.workspace.branch
  - context.workspace.opus_path
  - context.workspace.sol_path
  - context.climb.round
  - context.climb.history
routes:
  done:
    to: sync
    description: The climber's worktree exists on its own branch, each judge has its own checkout, and the round count starts at zero.
---

```sh
# One worktree for the climber's branch, and one detached checkout per judge
# that sync moves to the branch before every round.
set -eu
branch="spark/hill-climb/${SPARK_RUN_ID}"
root="${SPARK_PROJECT_WORK_DIR}/checkouts/${SPARK_RUN_ID}"
mkdir -p "${SPARK_PROJECT_WORK_DIR}/checkouts"
if ! git -C "$root" rev-parse --git-dir >/dev/null 2>&1; then
  if git show-ref --verify --quiet "refs/heads/$branch"; then
    git worktree add "$root" "$branch" >/dev/null
  else
    git worktree add -b "$branch" "$root" HEAD >/dev/null
  fi
fi
for judge in opus sol; do
  if ! git -C "$root-$judge" rev-parse --git-dir >/dev/null 2>&1; then
    git worktree add --detach "$root-$judge" "$branch" >/dev/null
  fi
done
printf '{"path":"%s","branch":"%s","opus_path":"%s","sol_path":"%s","round":0,"history":[]}' \
  "$root" "$branch" "$root-opus" "$root-sol"
```
