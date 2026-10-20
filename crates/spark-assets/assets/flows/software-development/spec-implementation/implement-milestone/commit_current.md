---
type: tool
title: Commit Item
runtime: {}
execution:
  reasoning_effort: xhigh
extensions:
  tool.artifacts.stderr: commit-item-node.stderr.txt
  tool.artifacts.stdout: commit-item-node.stdout.txt
  tool.command: |-
    set -euo pipefail
    if ! git rev-parse --is-inside-work-tree >/dev/null 2>&1; then
      echo "Skipping item commit: current repository is not a git work tree."
      exit 0
    fi
    if git diff --quiet && git diff --cached --quiet && [ -z "$(git ls-files --others --exclude-standard)" ]; then
      echo "Skipping item commit: no repository changes detected."
      exit 0
    fi
    message="Complete milestone item"
    state_dir=".spark/spec-implementation/current"
    if command -v jq >/dev/null 2>&1; then
      milestone=$(jq -r '.id // empty' "$state_dir/current-milestone.json" 2>/dev/null || true)
      if [ -n "$milestone" ]; then
        derived=$(jq -r '[.items[]? | select(.status == "completed")][-1] | select(. != null) | (.id // "item") + ": " + (.title // "completed")' "$state_dir/milestones/$milestone/state.json" 2>/dev/null || true)
        if [ -n "$derived" ]; then
          message="$derived"
        fi
      fi
    fi
    git add -A
    git commit -m "$message"
routes:
- to: next_item
  extensions:
    loop_restart: false
---

```sh
set -euo pipefail
if ! git rev-parse --is-inside-work-tree >/dev/null 2>&1; then
  echo "Skipping item commit: current repository is not a git work tree."
  exit 0
fi
if git diff --quiet && git diff --cached --quiet && [ -z "$(git ls-files --others --exclude-standard)" ]; then
  echo "Skipping item commit: no repository changes detected."
  exit 0
fi
message="Complete milestone item"
state_dir=".spark/spec-implementation/current"
if command -v jq >/dev/null 2>&1; then
  milestone=$(jq -r '.id // empty' "$state_dir/current-milestone.json" 2>/dev/null || true)
  if [ -n "$milestone" ]; then
    derived=$(jq -r '[.items[]? | select(.status == "completed")][-1] | select(. != null) | (.id // "item") + ": " + (.title // "completed")' "$state_dir/milestones/$milestone/state.json" 2>/dev/null || true)
    if [ -n "$derived" ]; then
      message="$derived"
    fi
  fi
fi
git add -A
git commit -m "$message"
```
