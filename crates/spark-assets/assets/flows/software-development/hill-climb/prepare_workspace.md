---
type: tool
title: Prepare Workspace
config:
  env_map:
    SPARK_BASE_REF: context.climb.base_ref
    SPARK_RUN_ID: internal.run_id
  exit_codes:
    '0': done
  output_map:
    context.climb.baseline_evidence_dir: baseline_evidence_dir
    context.climb.evidence_root: evidence_root
    context.climb.history: history
    context.climb.round: round
    context.workspace.base_commit: base_commit
    context.workspace.branch: branch
    context.workspace.path: path
contracts:
  writes_context:
  - context.workspace.path
  - context.workspace.branch
  - context.workspace.base_commit
  - context.climb.evidence_root
  - context.climb.baseline_evidence_dir
  - context.climb.round
  - context.climb.history
routes:
  done:
    to: capture
    description: The worktree and evidence directory exist and the round count starts at zero.
---

```sh
set -eu
base_commit=$(git rev-parse --verify "${SPARK_BASE_REF}^{commit}")
branch="spark/hill-climb/${SPARK_RUN_ID}"
work="${SPARK_PROJECT_WORK_DIR}"
worktree="${work}/checkouts/${SPARK_RUN_ID}"
evidence_root="${work}/hill-climb/${SPARK_RUN_ID}"
mkdir -p "${work}/checkouts" "$evidence_root"
if ! git -C "$worktree" rev-parse --git-dir >/dev/null 2>&1; then
  if git show-ref --verify --quiet "refs/heads/$branch"; then
    git worktree add "$worktree" "$branch" >/dev/null
  else
    git worktree add -b "$branch" "$worktree" "$base_commit" >/dev/null
  fi
fi
printf '{"path":"%s","branch":"%s","base_commit":"%s","evidence_root":"%s","baseline_evidence_dir":"%s/round-1","round":0,"history":[]}' \
  "$worktree" "$branch" "$base_commit" "$evidence_root" "$evidence_root"
```
