---
type: tool
title: Round Gate
config:
  decision_field: decision
extensions:
  tool.artifacts.stderr: round-gate.stderr.txt
  tool.artifacts.stdout: round-gate.stdout.txt
routes:
  exhausted:
    to: synthesize
    description: The completed round was recorded and the domain round budget is exhausted; synthesize the honest current findings.
  more:
    to: fan_out
    description: The completed round was recorded and the explicit domain round budget permits another round.
---

```sh
# Prints one JSON object and exits 0 for both decisions after recording the
# completed round: exhausted when the new round count reaches .mathlab/max-rounds,
# more otherwise. Missing files default to round 0 and max 3; an unreadable or
# non-integer file, max <= 0, or a failed write exits nonzero as an execution
# error.
set -eu
mkdir -p .mathlab
ROUND=0
if [ -e .mathlab/round ]; then ROUND="$(cat .mathlab/round)"; fi
MAX=3
if [ -e .mathlab/max-rounds ]; then MAX="$(cat .mathlab/max-rounds)"; fi
for value in "$ROUND" "$MAX"; do
  case "$value" in
    ''|*[!0-9]*|0?*)
      echo "round_gate: expected a nonnegative integer without leading zeros, got '$value'" >&2
      exit 1
      ;;
  esac
done
if [ "$MAX" -le 0 ]; then
  echo "round_gate: .mathlab/max-rounds must be positive, got $MAX" >&2
  exit 1
fi
ROUND=$((ROUND + 1))
printf '%s\n' "$ROUND" > .mathlab/round
if [ "$ROUND" -ge "$MAX" ]; then
  echo "BUDGET EXHAUSTED: round $ROUND of $MAX complete; proceeding to synthesis." >&2
  printf '{"decision":"exhausted"}\n'
else
  echo "Round $ROUND of $MAX complete; starting next round." >&2
  printf '{"decision":"more"}\n'
fi
```
