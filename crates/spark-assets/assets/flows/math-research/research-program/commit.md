---
type: tool
title: Commit Knowledge
extensions:
  tool.artifacts.stderr: commit.stderr.txt
  tool.artifacts.stdout: commit.stdout.txt
  tool.command: |-
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
routes:
- to: resolution
  condition: outcome=success
---

```sh
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
