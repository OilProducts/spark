---
type: tool
title: Verify Evidence
config:
  decision_field: decision
extensions:
  tool.artifacts.stderr: verify-evidence.stderr.txt
  tool.artifacts.stdout: verify-evidence.stdout.txt
routes:
  passed:
    to: synthesize
    description: The evidence verifier completed successfully.
  rejected:
    to: experiment
    description: Required verifier evidence is missing or the verifier did not pass.
---

```sh
# Prints one JSON object and exits 0 for both decisions: rejected when
# experiments/verify.sh is missing or exits nonzero, passed when it exits 0.
# Any nonzero exit of this gate (missing bash, log or cleanup failure) is an
# execution error. The verifier log goes to stderr and .mathlab/verify.log.
set -eu
command -v bash >/dev/null 2>&1 || { echo 'verify_evidence: bash is required' >&2; exit 1; }
mkdir -p .mathlab
if [ ! -x experiments/verify.sh ] && [ ! -f experiments/verify.sh ]; then
  echo 'GATE FAILURE: experiments/verify.sh is missing.' > .mathlab/verify.log
  cat .mathlab/verify.log >&2
  printf '{"decision":"rejected"}\n'
  exit 0
fi
: > .mathlab/verify.log
status=0
bash experiments/verify.sh >> .mathlab/verify.log 2>&1 || status=$?
cat .mathlab/verify.log >&2
if [ "$status" -eq 0 ]; then
  rm -f .mathlab/verify.log
  printf '{"decision":"passed"}\n'
else
  printf '{"decision":"rejected"}\n'
fi
```
