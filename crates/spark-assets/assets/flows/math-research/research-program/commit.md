---
type: tool
title: Commit Knowledge
config:
  exit_codes:
    '0': done
extensions:
  tool.artifacts.stderr: commit.stderr.txt
  tool.artifacts.stdout: commit.stdout.txt
routes:
  done:
    to: resolution
    description: The bounded work required by this node’s prompt is complete, including its required outputs; this does not assert that the whole workflow objective is achieved.
---

```sh
# Exits 0 after committing this action's knowledge changes. Any nonzero exit,
# including the missing history record or no staged knowledge change, is an
# execution error.
set -eu
ACTION="$(cat .math-research/current-action)"
test -f "history/$ACTION.md"
git add -- problem.md state.md history evidence
if git diff --cached --quiet; then
  echo "No knowledge changes to commit for $ACTION."
  exit 1
fi
git commit -m "Research $ACTION: $(head -c 72 "evidence/$ACTION/plan.md" | head -1)"
rm -f .math-research/repair-used
```
