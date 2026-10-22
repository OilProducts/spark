---
type: tool
title: Refuter Gate
extensions:
  tool.artifacts.stderr: refuter-gate.stderr.txt
  tool.artifacts.stdout: refuter-gate.stdout.txt
  tool.command: |-
    set -eu
    mkdir -p .mathlab
    if [ ! -f refuter/verify.sh ]; then
      echo 'GATE FAILURE: refuter/verify.sh is missing.' | tee .mathlab/refuter-gate.log
      exit 1
    fi
    if bash refuter/verify.sh > .mathlab/refuter-gate.log 2>&1; then
      if [ -f refuter/REFUTED ] && [ ! -f refuter/counterexample.md ]; then
        echo 'GATE FAILURE: REFUTED claimed without refuter/counterexample.md.' | tee -a .mathlab/refuter-gate.log
        exit 1
      fi
      cat .mathlab/refuter-gate.log
      rm -f .mathlab/refuter-gate.log
    else
      cat .mathlab/refuter-gate.log
      exit 1
    fi
routes:
- to: join
- to: refute
  label: Fix Evidence
  condition: outcome=fail
  extensions:
    loop_restart: false
---

```sh
set -eu
mkdir -p .mathlab
if [ ! -f refuter/verify.sh ]; then
  echo 'GATE FAILURE: refuter/verify.sh is missing.' | tee .mathlab/refuter-gate.log
  exit 1
fi
if bash refuter/verify.sh > .mathlab/refuter-gate.log 2>&1; then
  if [ -f refuter/REFUTED ] && [ ! -f refuter/counterexample.md ]; then
    echo 'GATE FAILURE: REFUTED claimed without refuter/counterexample.md.' | tee -a .mathlab/refuter-gate.log
    exit 1
  fi
  cat .mathlab/refuter-gate.log
  rm -f .mathlab/refuter-gate.log
else
  cat .mathlab/refuter-gate.log
  exit 1
fi
```
