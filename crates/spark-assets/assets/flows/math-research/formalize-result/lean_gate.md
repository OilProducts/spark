---
type: tool
title: Lean Gate
config:
  decision_field: decision
extensions:
  tool.artifacts.stderr: lean-gate.stderr.txt
  tool.artifacts.stdout: lean-gate.stdout.txt
routes:
  passed:
    to: review_fidelity
    description: The Lean build and proof-integrity scans passed.
  rejected:
    to: formalize
    description: Deliverable proof sources are missing, the build did not pass, or a forbidden proof construct was found.
---

```sh
# Prints one JSON object and exits 0 for both decisions: rejected when the
# deliverable sources are missing, lake build fails, or sorry/admit/axiom is
# found; passed otherwise. Missing lake/grep, log failures and grep errors
# (status > 1) exit nonzero as execution errors. The gate log goes to stderr
# and stays in .mathlab/lean-gate.log after a rejection.
set -eu
for tool in lake grep; do
  command -v "$tool" >/dev/null 2>&1 || { echo "lean_gate: $tool is required" >&2; exit 1; }
done
mkdir -p .mathlab
log="$(pwd)/.mathlab/lean-gate.log"
: > "$log"
reject() {
  printf '%s\n' "$1" >> "$log"
  cat "$log" >&2
  printf '{"decision":"rejected"}\n'
  exit 0
}
scan() {
  status=0
  (cd lean && grep -rn --include='*.lean' -E "$1" MathLab MathLab.lean) >> "$log" 2>&1 || status=$?
  case "$status" in
    0) reject "$2" ;;
    1) ;;
    *) cat "$log" >&2; echo "lean_gate: grep failed with status $status" >&2; exit 1 ;;
  esac
}
if [ ! -f lean/MathLab.lean ] || [ ! -d lean/MathLab ]; then
  reject 'GATE FAILURE: deliverable sources lean/MathLab.lean and lean/MathLab/ are missing.'
fi
build=0
(cd lean && lake build) >> "$log" 2>&1 || build=$?
[ "$build" -eq 0 ] || reject "GATE FAILURE: lake build exited with status $build."
scan '\bsorry\b|\badmit\b' 'GATE FAILURE: sorry/admit present in deliverable sources.'
scan '^[[:space:]]*axiom[[:space:]]' 'GATE FAILURE: new axiom declared in deliverable sources.'
cat "$log" >&2
rm -f "$log"
printf '{"decision":"passed"}\n'
```
