---
type: tool
title: Prover Gate
config:
  decision_field: decision
extensions:
  tool.artifacts.stderr: prover-gate.stderr.txt
  tool.artifacts.stdout: prover-gate.stdout.txt
routes:
  passed:
    to: join
    description: No complete proof is claimed, or the claimed proof passed the build and integrity scans.
  rejected:
    to: prove
    description: A claimed proof lacks required sources or failed those checks.
---

```sh
# Prints one JSON object and exits 0 for both decisions: passed when no
# prover/PROVED marker exists or the claimed proof builds with no
# sorry/admit/axiom; rejected when the claimed proof's sources are missing,
# lake build fails, or a forbidden construct is found. Missing lake/grep, log
# failures and grep errors (status > 1) exit nonzero as execution errors. The
# gate log goes to stderr and stays in .mathlab/prover-gate.log after a
# rejection.
set -eu
mkdir -p .mathlab
log="$(pwd)/.mathlab/prover-gate.log"
if [ ! -f prover/PROVED ]; then
  echo 'No complete proof claimed this round; gate passes.' >&2
  rm -f "$log"
  printf '{"decision":"passed"}\n'
  exit 0
fi
for tool in lake grep; do
  command -v "$tool" >/dev/null 2>&1 || { echo "prover_gate: $tool is required" >&2; exit 1; }
done
: > "$log"
reject() {
  printf '%s\n' "$1" >> "$log"
  cat "$log" >&2
  printf '{"decision":"rejected"}\n'
  exit 0
}
scan() {
  status=0
  (cd prover/lean && grep -rn --include='*.lean' -E "$1" MathLab MathLab.lean) >> "$log" 2>&1 || status=$?
  case "$status" in
    0) reject "$2" ;;
    1) ;;
    *) cat "$log" >&2; echo "prover_gate: grep failed with status $status" >&2; exit 1 ;;
  esac
}
if [ ! -f prover/lean/MathLab.lean ] || [ ! -d prover/lean/MathLab ]; then
  reject 'GATE FAILURE: claimed proof sources prover/lean/MathLab.lean and prover/lean/MathLab/ are missing.'
fi
build=0
(cd prover/lean && lake build) >> "$log" 2>&1 || build=$?
[ "$build" -eq 0 ] || reject "GATE FAILURE: lake build exited with status $build."
scan '\bsorry\b|\badmit\b' 'GATE FAILURE: sorry/admit present in claimed proof.'
scan '^[[:space:]]*axiom[[:space:]]' 'GATE FAILURE: new axiom declared in claimed proof.'
cat "$log" >&2
rm -f "$log"
printf '{"decision":"passed"}\n'
```
