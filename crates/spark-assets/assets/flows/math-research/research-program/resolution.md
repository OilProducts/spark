---
type: tool
title: Check Resolution
extensions:
  tool.artifacts.stderr: resolution.stderr.txt
  tool.artifacts.stdout: resolution.stdout.txt
  tool.command: |-
    set -eu
    ACTION="$(cat .math-research/current-action)"
    DIR="evidence/$ACTION"
    if [ -f .math-research/human-accepted-resolution.json ]; then
      jq -e '.claim_id | type == "string" and length > 0' .math-research/human-accepted-resolution.json >/dev/null
      echo "Human-accepted reviewed proof recorded."
      exit 0
    fi
    test -f "$DIR/resolution.json" || { echo "Problem remains open."; exit 1; }
    test -f "$DIR/.verified"
    jq -e '.kind == "certified_proof" or .kind == "verified_counterexample"' "$DIR/resolution.json" >/dev/null
    jq -e '.claim_id | type == "string" and length > 0' "$DIR/resolution.json" >/dev/null
    bash "$DIR/verify.sh"
    echo "Mechanically verified resolution accepted."
routes:
- to: done
  condition: outcome=success
- to: research
  condition: outcome=fail
---

```sh
set -eu
ACTION="$(cat .math-research/current-action)"
DIR="evidence/$ACTION"
if [ -f .math-research/human-accepted-resolution.json ]; then
  jq -e '.claim_id | type == "string" and length > 0' .math-research/human-accepted-resolution.json >/dev/null
  echo "Human-accepted reviewed proof recorded."
  exit 0
fi
test -f "$DIR/resolution.json" || { echo "Problem remains open."; exit 1; }
test -f "$DIR/.verified"
jq -e '.kind == "certified_proof" or .kind == "verified_counterexample"' "$DIR/resolution.json" >/dev/null
jq -e '.claim_id | type == "string" and length > 0' "$DIR/resolution.json" >/dev/null
bash "$DIR/verify.sh"
echo "Mechanically verified resolution accepted."
```
