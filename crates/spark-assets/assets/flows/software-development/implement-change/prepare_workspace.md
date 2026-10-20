---
type: tool
title: Prepare Workspace
config:
  env_map:
    SPARK_BASE_REF: context.task.base_ref
    SPARK_RUN_ID: internal.run_id
    SPARK_TASK_ARTIFACT: context.task.artifact_path
  output_map:
    context.validation.attempts: validation_attempts
    context.workspace.base_commit: base_commit
    context.workspace.branch: branch
    context.workspace.path: path
    context.workspace.source_dirty: source_dirty
contracts:
  writes_context:
  - context.workspace.path
  - context.workspace.branch
  - context.workspace.base_commit
  - context.workspace.source_dirty
  - context.validation.attempts
routes:
- to: implement
  condition: outcome=success
---

```sh
set -eu
repo_root=$(git rev-parse --show-toplevel)
artifact=$SPARK_TASK_ARTIFACT
fail_artifact() { echo "task artifact: $*" >&2; exit 1; }
# Reject symlinks in every component, including dangling destination links.
check_artifact_path() {
  path=$1
  remaining=$artifact
  while [ -n "$remaining" ]; do
    component=${remaining%%/*}
    case "$component" in
      ''|.|..|.git) fail_artifact "unsafe path: $artifact" ;;
    esac
    path="$path/$component"
    [ ! -L "$path" ] || fail_artifact "symlink in path: $path"
    case "$remaining" in
      */*) remaining=${remaining#*/} ;;
      *) remaining='' ;;
    esac
  done
}
if [ -n "$artifact" ]; then
  case "$artifact" in
    /*|*/) fail_artifact "expected a repository-relative file: $artifact" ;;
  esac
  check_artifact_path "$repo_root"
  [ -f "$repo_root/$artifact" ] && [ -r "$repo_root/$artifact" ] ||
    fail_artifact "missing or unreadable source: $artifact"
fi
base_commit=$(git rev-parse --verify "${SPARK_BASE_REF}^{commit}")
branch="spark/implement-change/${SPARK_RUN_ID}"
# Worktrees live under <repo>/.spark, never inside .git: tooling such
# as Vite refuses to serve or resolve paths matching **/.git/**. The
# directory ignores itself so the source checkout stays clean.
checkout_root="${repo_root}/.spark/checkouts"
worktree="${checkout_root}/${SPARK_RUN_ID}"
mkdir -p "$checkout_root"
printf '*\n' > "${repo_root}/.spark/.gitignore"
if [ -d "$worktree" ] && git -C "$worktree" rev-parse --git-dir >/dev/null 2>&1; then
  existing=$(git -C "$worktree" rev-parse --abbrev-ref HEAD)
  if [ "$existing" != "$branch" ]; then
    echo "existing worktree $worktree is on unexpected branch $existing" >&2
    exit 1
  fi
else
  if git show-ref --verify --quiet "refs/heads/$branch"; then
    git worktree add "$worktree" "$branch" >/dev/null
  else
    git worktree add -b "$branch" "$worktree" "$base_commit" >/dev/null
  fi
fi
if [ -n "$artifact" ]; then
  check_artifact_path "$worktree"
  destination="$worktree/$artifact"
  if [ -e "$destination" ]; then
    [ -f "$destination" ] && cmp -s "$repo_root/$artifact" "$destination" ||
      fail_artifact "checkout content conflicts with source: $artifact"
  else
    mkdir -p "$(dirname "$destination")"
    cp "$repo_root/$artifact" "$destination"
  fi
fi
dirty=false
if [ -n "$(git status --porcelain)" ]; then dirty=true; fi
printf '{"path":"%s","branch":"%s","base_commit":"%s","source_dirty":%s,"validation_attempts":0}' \
  "$worktree" "$branch" "$base_commit" "$dirty"
```
