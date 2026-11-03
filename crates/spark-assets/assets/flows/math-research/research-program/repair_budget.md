---
type: tool
title: One Repair
config:
  decision_field: decision
extensions:
  tool.artifacts.stderr: repair-budget.stderr.txt
  tool.artifacts.stdout: repair-budget.stdout.txt
routes:
  allowed:
    to: repair
    description: The current action was read and the one-repair marker was successfully written; one focused repair is allowed.
  exhausted:
    to: assess
    description: The marker already exists, so assess the existing evidence without another repair.
---

```sh
# Prints one JSON object and exits 0 for both decisions: exhausted when the
# .math-research/repair-used marker already exists, allowed after writing it.
# An unreadable or invalid action ID, a marker that is not a readable regular
# file, or a failed marker write exits nonzero as an execution error.
set -eu
ACTION="$(cat .math-research/current-action)"
case "$ACTION" in A[0-9][0-9][0-9][0-9]) ;; *) echo "Invalid action id: $ACTION" >&2; exit 1;; esac
MARKER=.math-research/repair-used
if [ -e "$MARKER" ] || [ -L "$MARKER" ]; then
  if [ ! -f "$MARKER" ] || [ ! -r "$MARKER" ]; then
    echo "repair_budget: $MARKER is not a readable regular file" >&2
    exit 1
  fi
  echo "Evidence repair already attempted for $ACTION; assess it as unverified." >&2
  printf '{"decision":"exhausted"}\n'
  exit 0
fi
printf '%s\n' "$ACTION" > "$MARKER"
echo "Allowing one focused evidence repair for $ACTION." >&2
printf '{"decision":"allowed"}\n'
```
