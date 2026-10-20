---
type: tool
title: Check Evidence
extensions:
  tool.artifacts.stderr: verify.stderr.txt
  tool.artifacts.stdout: verify.stdout.txt
  tool.command: |-
    set -eu
    ACTION="$(cat .math-research/current-action)"
    case "$ACTION" in A[0-9][0-9][0-9][0-9]) ;; *) echo "Invalid action id: $ACTION"; exit 1;; esac
    DIR="evidence/$ACTION"
    test -f "$DIR/claim.json"
    rm -f "$DIR/.verified" "$DIR/.not-machine-checkable"
    if [ ! -f "$DIR/verify.sh" ]; then
      touch "$DIR/.not-machine-checkable"
      echo "No machine-checkable claim proposed."
      exit 0
    fi
    if bash "$DIR/verify.sh" > "$DIR/verification.log" 2>&1; then
      touch "$DIR/.verified"
      cat "$DIR/verification.log"
    else
      cat "$DIR/verification.log"
      exit 1
    fi
routes:
- to: assess
  condition: outcome=success
- to: repair_budget
  condition: outcome=fail
---

```sh
set -eu
ACTION="$(cat .math-research/current-action)"
case "$ACTION" in A[0-9][0-9][0-9][0-9]) ;; *) echo "Invalid action id: $ACTION"; exit 1;; esac
DIR="evidence/$ACTION"
test -f "$DIR/claim.json"
rm -f "$DIR/.verified" "$DIR/.not-machine-checkable"
if [ ! -f "$DIR/verify.sh" ]; then
  touch "$DIR/.not-machine-checkable"
  echo "No machine-checkable claim proposed."
  exit 0
fi
if bash "$DIR/verify.sh" > "$DIR/verification.log" 2>&1; then
  touch "$DIR/.verified"
  cat "$DIR/verification.log"
else
  cat "$DIR/verification.log"
  exit 1
fi
```
