---
type: tool
title: Round Gate
extensions:
  tool.artifacts.stderr: round-gate.stderr.txt
  tool.artifacts.stdout: round-gate.stdout.txt
  tool.command: |-
    set -eu
    mkdir -p .mathlab
    ROUND="$(cat .mathlab/round 2>/dev/null || echo 0)"
    ROUND=$((ROUND + 1))
    printf '%s\n' "$ROUND" > .mathlab/round
    MAX="$(cat .mathlab/max-rounds 2>/dev/null || echo 3)"
    if [ "$ROUND" -ge "$MAX" ]; then
      echo "BUDGET EXHAUSTED: round $ROUND of $MAX complete; proceeding to synthesis."
      exit 1
    fi
    echo "Round $ROUND of $MAX complete; starting next round."
routes:
- to: fan_out
  label: Next Round
- to: synthesize
  label: Budget Exhausted
  condition: outcome=fail
  extensions:
    loop_restart: false
---

```sh
set -eu
mkdir -p .mathlab
ROUND="$(cat .mathlab/round 2>/dev/null || echo 0)"
ROUND=$((ROUND + 1))
printf '%s\n' "$ROUND" > .mathlab/round
MAX="$(cat .mathlab/max-rounds 2>/dev/null || echo 3)"
if [ "$ROUND" -ge "$MAX" ]; then
  echo "BUDGET EXHAUSTED: round $ROUND of $MAX complete; proceeding to synthesis."
  exit 1
fi
echo "Round $ROUND of $MAX complete; starting next round."
```
