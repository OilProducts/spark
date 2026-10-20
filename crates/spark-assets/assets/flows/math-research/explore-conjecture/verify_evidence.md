---
type: tool
title: Verify Evidence
extensions:
  tool.artifacts.stderr: verify-evidence.stderr.txt
  tool.artifacts.stdout: verify-evidence.stdout.txt
  tool.command: |-
    set -eu
    mkdir -p .mathlab
    if [ ! -x experiments/verify.sh ] && [ ! -f experiments/verify.sh ]; then
      echo 'GATE FAILURE: experiments/verify.sh is missing.' | tee .mathlab/verify.log
      exit 1
    fi
    if bash experiments/verify.sh > .mathlab/verify.log 2>&1; then
      cat .mathlab/verify.log
      rm -f .mathlab/verify.log
    else
      cat .mathlab/verify.log
      exit 1
    fi
routes:
- to: experiment
  label: Fix Evidence
  condition: outcome=fail
  extensions:
    loop_restart: false
- to: synthesize
---

```sh
set -eu
mkdir -p .mathlab
if [ ! -x experiments/verify.sh ] && [ ! -f experiments/verify.sh ]; then
  echo 'GATE FAILURE: experiments/verify.sh is missing.' | tee .mathlab/verify.log
  exit 1
fi
if bash experiments/verify.sh > .mathlab/verify.log 2>&1; then
  cat .mathlab/verify.log
  rm -f .mathlab/verify.log
else
  cat .mathlab/verify.log
  exit 1
fi
```
