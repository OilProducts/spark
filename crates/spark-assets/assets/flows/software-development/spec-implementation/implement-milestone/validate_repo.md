---
type: tool
title: Validate Repo
config:
  exit_codes:
    '0': passed
    '1': rejected
    '2': blocked
runtime: {}
execution:
  reasoning_effort: xhigh
extensions:
  tool.artifacts.paths: .spark/state/spec-implementation/current/run-milestone-validation.sh,.spark/state/spec-implementation/current/validation-plan.json,.spark/state/spec-implementation/current/validation-plan.json,.spark/state/spec-implementation/current/validation.stdout.txt,.spark/state/spec-implementation/current/validation.stderr.txt
  tool.artifacts.stderr: validate-node.stderr.txt
  tool.artifacts.stdout: validate-node.stdout.txt
routes:
  blocked:
    to: assess_validation
    description: The generated entrypoint identified a genuinely external missing prerequisite; assess its evidence before deciding whether the item is blocked.
  passed:
    to: assess_validation
    description: The generated repository validation entrypoint returned pass; assess its captured evidence.
  rejected:
    to: assess_validation
    description: The generated validation entrypoint reported non-pass or missing repository support; assess the evidence before repair.
---

```sh
bash .spark/state/spec-implementation/current/run-milestone-validation.sh
```
