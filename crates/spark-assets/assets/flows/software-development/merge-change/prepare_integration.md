---
type: tool
title: Prepare Integration
config:
  env_map:
    SPARK_RUN_ID: internal.run_id
    SPARK_SOURCE_REF: context.task.source_ref
  exit_codes:
    '0': next
  output_map:
    context.integration.conflict_count: conflict_count
    context.integration.path: path
    context.integration.source_commit: source_commit
    context.integration.target_branch: target_branch
    context.integration.target_commit: target_commit
    context.integration.target_moved: target_moved
contracts:
  writes_context:
  - context.integration.path
  - context.integration.target_branch
  - context.integration.target_commit
  - context.integration.source_commit
  - context.integration.conflict_count
  - context.integration.target_moved
routes:
  next:
    to: check_conflicts
    description: Integration preparation completed; inspect its conflict count.
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
# The merged tree is the one already tested only when the target has not moved
# past the source branch's merge base.
target_moved=true
if [ "$(git merge-base "$target_commit" "$source_commit" || true)" = "$target_commit" ]; then
  target_moved=false
fi
# Worktrees live in the project's private work area, outside the checkout
# and never inside .git: tooling such as Vite refuses to serve or resolve
# paths matching **/.git/**.
integration_root="${SPARK_PROJECT_WORK_DIR}/integrations"
worktree="${integration_root}/${SPARK_RUN_ID}"
if [ ! -d "$worktree" ]; then
  mkdir -p "$integration_root"
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
unresolved=$(git diff --name-only --diff-filter=U)
conflicts=0
if [ -n "$unresolved" ]; then
  conflicts=$(printf '%s\n' "$unresolved" | wc -l | tr -d ' ')
fi
case "$conflicts" in
  ''|*[!0-9]*)
    echo "could not count unresolved paths: '$conflicts'" >&2
    exit 1
    ;;
esac
printf '{"path":"%s","target_branch":"%s","target_commit":"%s","source_commit":"%s","conflict_count":%s,"target_moved":"%s"}' \
  "$worktree" "$target_branch" "$target_commit" "$source_commit" "$conflicts" "$target_moved"
```
