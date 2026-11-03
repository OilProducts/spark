---
type: tool
title: Check Resolution
config:
  decision_field: decision
extensions:
  tool.artifacts.stderr: resolution.stderr.txt
  tool.artifacts.stdout: resolution.stdout.txt
routes:
  open:
    to: research
    description: No mechanically accepted resolution is currently established; continue research without claiming resolution.
  resolved:
    to: done
    description: A valid explicit human acceptance or mechanically verified faithful resolution satisfies this node’s resolution predicate.
---

```sh
# Prints one JSON object and exits 0 for both decisions: resolved for a valid
# human acceptance, or for a candidate resolution.json with an allowed kind and
# claim ID, a .verified marker and a passing verify.sh; open otherwise. Missing
# bash/jq, an unreadable or invalid action ID or input, and a malformed human
# acceptance file exit nonzero as execution errors.
set -eu
for tool in bash jq; do
  command -v "$tool" >/dev/null 2>&1 || { echo "resolution: $tool is required" >&2; exit 1; }
done
ACTION="$(cat .math-research/current-action)"
case "$ACTION" in A[0-9][0-9][0-9][0-9]) ;; *) echo "Invalid action id: $ACTION" >&2; exit 1;; esac
DIR="evidence/$ACTION"
HUMAN=.math-research/human-accepted-resolution.json
still_open() {
  echo "$1" >&2
  printf '{"decision":"open"}\n'
  exit 0
}
if [ -e "$HUMAN" ]; then
  jq -e '.claim_id | type == "string" and length > 0' "$HUMAN" >/dev/null || {
    echo "resolution: $HUMAN must name a nonempty string claim_id" >&2
    exit 1
  }
  echo "Human-accepted reviewed proof recorded." >&2
  printf '{"decision":"resolved"}\n'
  exit 0
fi
[ -e "$DIR/resolution.json" ] || still_open "Problem remains open."
[ -f "$DIR/resolution.json" ] && [ -r "$DIR/resolution.json" ] || {
  echo "resolution: cannot read $DIR/resolution.json" >&2
  exit 1
}
[ -f "$DIR/.verified" ] || still_open "Candidate resolution has no .verified marker; problem remains open."
jq -e '.kind == "certified_proof" or .kind == "verified_counterexample"' "$DIR/resolution.json" >/dev/null 2>&1 ||
  still_open "Candidate resolution has an unsupported kind; problem remains open."
jq -e '.claim_id | type == "string" and length > 0' "$DIR/resolution.json" >/dev/null 2>&1 ||
  still_open "Candidate resolution has no valid claim_id; problem remains open."
[ -f "$DIR/verify.sh" ] || still_open "Candidate resolution has no verify.sh; problem remains open."
status=0
bash "$DIR/verify.sh" >&2 || status=$?
[ "$status" -eq 0 ] || still_open "Candidate verifier exited with status $status; problem remains open."
echo "Mechanically verified resolution accepted." >&2
printf '{"decision":"resolved"}\n'
```
