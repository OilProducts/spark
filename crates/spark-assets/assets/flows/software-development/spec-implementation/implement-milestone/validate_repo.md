---
type: tool
title: Validate Repo
runtime: {}
execution:
  reasoning_effort: xhigh
extensions:
  tool.artifacts.paths: .spark/spec-implementation/current/run-milestone-validation.sh,.spark/spec-implementation/current/validation-plan.json,.spark/spec-implementation/current/validation-plan.json,.spark/spec-implementation/current/validation.stdout.txt,.spark/spec-implementation/current/validation.stderr.txt
  tool.artifacts.stderr: validate-node.stderr.txt
  tool.artifacts.stdout: validate-node.stdout.txt
  tool.command: bash .spark/spec-implementation/current/run-milestone-validation.sh
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
bash .spark/spec-implementation/current/run-milestone-validation.sh
```
