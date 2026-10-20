---
type: tool
title: Prepare Integration
config:
  env_map:
    SPARK_RUN_ID: internal.run_id
    SPARK_SOURCE_REF: context.task.source_ref
  output_map:
    context.integration.conflict_count: conflict_count
    context.integration.path: path
    context.integration.source_commit: source_commit
    context.integration.target_branch: target_branch
    context.integration.target_commit: target_commit
contracts:
  writes_context:
  - context.integration.path
  - context.integration.target_branch
  - context.integration.target_commit
  - context.integration.source_commit
  - context.integration.conflict_count
routes:
- to: finish_merge
  condition: outcome=success && context.integration.conflict_count=0
- to: resolve_conflicts
  condition: outcome=success && context.integration.conflict_count!=0
---

```sh
set -eu
if [ -n "$(git status --porcelain --untracked-files=no)" ]; then
  echo "target checkout has uncommitted changes; commit or stash before integrating" >&2
  exit 1
fi
target_branch=$(git rev-parse --abbrev-ref HEAD)
if [ "$target_branch" = "HEAD" ]; then
  echo "target checkout is on a detached HEAD; check out the target branch first" >&2
  exit 1
fi
target_commit=$(git rev-parse HEAD)
source_commit=$(git rev-parse --verify "${SPARK_SOURCE_REF}^{commit}")
repo_root=$(git rev-parse --show-toplevel)
# Worktrees live under <repo>/.spark, never inside .git: tooling such
# as Vite refuses to serve or resolve paths matching **/.git/**. The
# directory ignores itself so the target checkout stays clean.
integration_root="${repo_root}/.spark/integrations"
worktree="${integration_root}/${SPARK_RUN_ID}"
if [ ! -d "$worktree" ]; then
  mkdir -p "$integration_root"
  printf '*\n' > "${repo_root}/.spark/.gitignore"
  git worktree add --detach "$worktree" "$target_commit" >/dev/null
fi
cd "$worktree"
merge_head=$(git rev-parse --git-path MERGE_HEAD)
if [ ! -f "$merge_head" ]; then
  git merge --no-ff --no-commit "$source_commit" >/dev/null ||
    test -n "$(git diff --name-only --diff-filter=U)"
fi
if [ ! -f "$merge_head" ]; then
  echo "no merge in progress; source may already be integrated" >&2
  exit 1
fi
conflicts=$(git diff --name-only --diff-filter=U | wc -l | tr -d ' ')
printf '{"path":"%s","target_branch":"%s","target_commit":"%s","source_commit":"%s","conflict_count":%s}' \
  "$worktree" "$target_branch" "$target_commit" "$source_commit" "$conflicts"
```
