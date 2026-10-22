---
type: tool
title: Commit Milestone
runtime: {}
execution:
  reasoning_effort: xhigh
extensions:
  tool.artifacts.stderr: commit-node.stderr.txt
  tool.artifacts.stdout: commit-node.stdout.txt
  tool.command: |-
    set -euo pipefail
    if ! git rev-parse --is-inside-work-tree >/dev/null 2>&1; then
      echo "Skipping milestone commit: current repository is not a git work tree."
      exit 0
    fi
    milestone=$(jq -r '.id // empty' .spark/spec-implementation/current/current-milestone.json 2>/dev/null || true)
    git add -A
    git commit --allow-empty -m "Accepted ${milestone:-milestone}"
routes:
- to: blocked_exit
  label: Commit Failed
  condition: outcome=fail
  extensions:
    loop_restart: false
- to: next_milestone
  label: Continue
  condition: outcome=success
  extensions:
    loop_restart: false
---

```sh
set -euo pipefail
if ! git rev-parse --is-inside-work-tree >/dev/null 2>&1; then
  echo "Skipping milestone commit: current repository is not a git work tree."
  exit 0
fi
milestone=$(jq -r '.id // empty' .spark/spec-implementation/current/current-milestone.json 2>/dev/null || true)
git add -A
git commit --allow-empty -m "Accepted ${milestone:-milestone}"
```
