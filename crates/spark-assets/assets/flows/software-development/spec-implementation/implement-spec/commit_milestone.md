---
type: tool
title: Commit Milestone
config:
  exit_codes:
    '0': done
runtime: {}
execution:
  reasoning_effort: xhigh
extensions:
  tool.artifacts.stderr: commit-node.stderr.txt
  tool.artifacts.stdout: commit-node.stdout.txt
routes:
  done:
    to: next_milestone
    description: The bounded work required by this node’s prompt is complete, including its required outputs; this does not assert that the whole workflow objective is achieved.
---

```sh
set -euo pipefail
if ! git rev-parse --is-inside-work-tree >/dev/null 2>&1; then
  echo "Skipping milestone commit: current repository is not a git work tree."
  exit 0
fi
milestone=$(jq -r '.id // empty' .spark/state/spec-implementation/current/current-milestone.json 2>/dev/null || true)
git add -A
git commit --allow-empty -m "Accepted ${milestone:-milestone}"
```
