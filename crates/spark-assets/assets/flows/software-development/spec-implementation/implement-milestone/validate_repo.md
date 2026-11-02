---
type: tool
title: Validate Repo
runtime: {}
execution:
  reasoning_effort: xhigh
extensions:
  tool.artifacts.paths: .spark/state/spec-implementation/current/run-milestone-validation.sh,.spark/state/spec-implementation/current/validation-plan.json,.spark/state/spec-implementation/current/validation-plan.json,.spark/state/spec-implementation/current/validation.stdout.txt,.spark/state/spec-implementation/current/validation.stderr.txt
  tool.artifacts.stderr: validate-node.stderr.txt
  tool.artifacts.stdout: validate-node.stdout.txt
  tool.command: bash .spark/state/spec-implementation/current/run-milestone-validation.sh
routes:
- to: assess_validation
  label: Assess Fail
  condition: outcome=fail
  extensions:
    loop_restart: false
- to: assess_validation
  label: Assess Pass
  condition: outcome=success
  extensions:
    loop_restart: false
---

```sh
bash .spark/state/spec-implementation/current/run-milestone-validation.sh
```
