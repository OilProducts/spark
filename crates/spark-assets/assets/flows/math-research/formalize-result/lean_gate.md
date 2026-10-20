---
type: tool
title: Lean Gate
extensions:
  tool.artifacts.stderr: lean-gate.stderr.txt
  tool.artifacts.stdout: lean-gate.stdout.txt
  tool.command: |-
    set -eu
    mkdir -p .mathlab
    {
      cd lean
      lake build
      if grep -rn --include='*.lean' -E '\bsorry\b|\badmit\b' MathLab MathLab.lean; then
        echo 'GATE FAILURE: sorry/admit present in deliverable sources.'
        exit 1
      fi
      if grep -rn --include='*.lean' -E '^[[:space:]]*axiom[[:space:]]' MathLab MathLab.lean; then
        echo 'GATE FAILURE: new axiom declared in deliverable sources.'
        exit 1
      fi
    } > ../.mathlab/lean-gate.log 2>&1 || { cat ../.mathlab/lean-gate.log; exit 1; }
    cat .mathlab/lean-gate.log
    rm -f .mathlab/lean-gate.log
routes:
- to: formalize
  label: Fix Proof
  condition: outcome=fail
  extensions:
    loop_restart: false
- to: review_fidelity
---

```sh
set -eu
mkdir -p .mathlab
{
  cd lean
  lake build
  if grep -rn --include='*.lean' -E '\bsorry\b|\badmit\b' MathLab MathLab.lean; then
    echo 'GATE FAILURE: sorry/admit present in deliverable sources.'
    exit 1
  fi
  if grep -rn --include='*.lean' -E '^[[:space:]]*axiom[[:space:]]' MathLab MathLab.lean; then
    echo 'GATE FAILURE: new axiom declared in deliverable sources.'
    exit 1
  fi
} > ../.mathlab/lean-gate.log 2>&1 || { cat ../.mathlab/lean-gate.log; exit 1; }
cat .mathlab/lean-gate.log
rm -f .mathlab/lean-gate.log
```
