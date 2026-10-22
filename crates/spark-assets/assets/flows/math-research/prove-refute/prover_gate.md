---
type: tool
title: Prover Gate
extensions:
  tool.artifacts.stderr: prover-gate.stderr.txt
  tool.artifacts.stdout: prover-gate.stdout.txt
  tool.command: |-
    set -eu
    mkdir -p .mathlab
    if [ ! -f prover/PROVED ]; then
      echo 'No complete proof claimed this round; gate passes.'
      rm -f .mathlab/prover-gate.log
      exit 0
    fi
    if (
      cd prover/lean
      lake build
      if grep -rn --include='*.lean' -E '\bsorry\b|\badmit\b' MathLab MathLab.lean; then
        echo 'GATE FAILURE: sorry/admit present in claimed proof.'
        exit 1
      fi
      if grep -rn --include='*.lean' -E '^[[:space:]]*axiom[[:space:]]' MathLab MathLab.lean; then
        echo 'GATE FAILURE: new axiom declared in claimed proof.'
        exit 1
      fi
    ) > .mathlab/prover-gate.log 2>&1; then
      cat .mathlab/prover-gate.log
      rm -f .mathlab/prover-gate.log
    else
      cat .mathlab/prover-gate.log
      exit 1
    fi
routes:
- to: join
- to: prove
  label: Fix Proof
  condition: outcome=fail
  extensions:
    loop_restart: false
---

```sh
set -eu
mkdir -p .mathlab
if [ ! -f prover/PROVED ]; then
  echo 'No complete proof claimed this round; gate passes.'
  rm -f .mathlab/prover-gate.log
  exit 0
fi
if (
  cd prover/lean
  lake build
  if grep -rn --include='*.lean' -E '\bsorry\b|\badmit\b' MathLab MathLab.lean; then
    echo 'GATE FAILURE: sorry/admit present in claimed proof.'
    exit 1
  fi
  if grep -rn --include='*.lean' -E '^[[:space:]]*axiom[[:space:]]' MathLab MathLab.lean; then
    echo 'GATE FAILURE: new axiom declared in claimed proof.'
    exit 1
  fi
) > .mathlab/prover-gate.log 2>&1; then
  cat .mathlab/prover-gate.log
  rm -f .mathlab/prover-gate.log
else
  cat .mathlab/prover-gate.log
  exit 1
fi
```
