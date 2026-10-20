---
type: tool
title: One Repair
extensions:
  tool.artifacts.stderr: repair-budget.stderr.txt
  tool.artifacts.stdout: repair-budget.stdout.txt
  tool.command: |-
    set -eu
    ACTION="$(cat .math-research/current-action)"
    if [ -f .math-research/repair-used ]; then
      echo "Evidence repair already attempted for $ACTION; assess it as unverified."
      exit 1
    fi
    printf '%s\n' "$ACTION" > .math-research/repair-used
    echo "Allowing one focused evidence repair for $ACTION."
routes:
- to: assess
  condition: outcome=fail
- to: repair
  condition: outcome=success
---

```sh
set -eu
ACTION="$(cat .math-research/current-action)"
if [ -f .math-research/repair-used ]; then
  echo "Evidence repair already attempted for $ACTION; assess it as unverified."
  exit 1
fi
printf '%s\n' "$ACTION" > .math-research/repair-used
echo "Allowing one focused evidence repair for $ACTION."
```
