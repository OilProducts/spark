---
type: tool
title: Check Evidence
config:
  decision_field: decision
extensions:
  tool.artifacts.stderr: verify.stderr.txt
  tool.artifacts.stdout: verify.stdout.txt
routes:
  passed:
    to: assess
    description: The action has no proposed machine-checkable verifier or its verifier passed, and the corresponding marker was written.
  rejected:
    to: repair_budget
    description: Required claim evidence is missing or malformed, or the action verifier did not pass; record the evidence for repair.
---

```sh
# Prints one JSON object and exits 0 for both decisions: rejected when the
# action's claim.json is missing or not a JSON object, or its verify.sh exits
# nonzero; passed when no verify.sh is proposed (.not-machine-checkable) or it
# exits 0 (.verified). A missing/invalid action ID, missing bash/jq, and
# marker or log failures exit nonzero as execution errors.
set -eu
for tool in bash jq; do
  command -v "$tool" >/dev/null 2>&1 || { echo "verify: $tool is required" >&2; exit 1; }
done
ACTION="$(cat .math-research/current-action)"
case "$ACTION" in A[0-9][0-9][0-9][0-9]) ;; *) echo "Invalid action id: $ACTION" >&2; exit 1;; esac
DIR="evidence/$ACTION"
rm -f "$DIR/.verified" "$DIR/.not-machine-checkable"
reject() {
  echo "$1" >&2
  if [ -d "$DIR" ]; then printf '%s\n' "$1" > "$DIR/verification.log"; fi
  printf '{"decision":"rejected"}\n'
  exit 0
}
[ -f "$DIR/claim.json" ] || reject "GATE FAILURE: $DIR/claim.json is missing."
[ -r "$DIR/claim.json" ] || { echo "verify: cannot read $DIR/claim.json" >&2; exit 1; }
jq -e 'type == "object"' "$DIR/claim.json" >/dev/null 2>&1 ||
  reject "GATE FAILURE: $DIR/claim.json is not a JSON object."
if [ ! -f "$DIR/verify.sh" ]; then
  touch "$DIR/.not-machine-checkable"
  echo "No machine-checkable claim proposed." >&2
  printf '{"decision":"passed"}\n'
  exit 0
fi
: > "$DIR/verification.log"
status=0
bash "$DIR/verify.sh" >> "$DIR/verification.log" 2>&1 || status=$?
cat "$DIR/verification.log" >&2
if [ "$status" -eq 0 ]; then
  touch "$DIR/.verified"
  printf '{"decision":"passed"}\n'
else
  printf '{"decision":"rejected"}\n'
fi
```
