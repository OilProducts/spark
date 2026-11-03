---
type: tool
title: Refuter Gate
config:
  decision_field: decision
extensions:
  tool.artifacts.stderr: refuter-gate.stderr.txt
  tool.artifacts.stdout: refuter-gate.stdout.txt
routes:
  passed:
    to: join
    description: The refuter verifier passed and any REFUTED marker has its required witness report.
  rejected:
    to: refute
    description: Required verifier or witness evidence is missing, or the verifier did not pass.
---

```sh
# Prints one JSON object and exits 0 for both decisions: rejected when
# refuter/verify.sh is missing or exits nonzero, or refuter/REFUTED lacks
# refuter/counterexample.md; passed otherwise. Missing bash and log or cleanup
# failures exit nonzero as execution errors. The gate log goes to stderr and
# stays in .mathlab/refuter-gate.log after a rejection.
set -eu
command -v bash >/dev/null 2>&1 || { echo 'refuter_gate: bash is required' >&2; exit 1; }
mkdir -p .mathlab
log="$(pwd)/.mathlab/refuter-gate.log"
reject() {
  [ -z "$1" ] || printf '%s\n' "$1" >> "$log"
  cat "$log" >&2
  printf '{"decision":"rejected"}\n'
  exit 0
}
if [ ! -f refuter/verify.sh ]; then
  : > "$log"
  reject 'GATE FAILURE: refuter/verify.sh is missing.'
fi
: > "$log"
status=0
bash refuter/verify.sh >> "$log" 2>&1 || status=$?
[ "$status" -eq 0 ] || reject ''
if [ -f refuter/REFUTED ] && [ ! -f refuter/counterexample.md ]; then
  reject 'GATE FAILURE: REFUTED claimed without refuter/counterexample.md.'
fi
cat "$log" >&2
rm -f "$log"
printf '{"decision":"passed"}\n'
```
