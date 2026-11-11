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
# ponytail: tool artifacts capture only checkout files, so the validation
# evidence stays in the work area; capture it when tools can name other roots.
bash "${SPARK_PROJECT_WORK_DIR}/spec-implementation/current/run-milestone-validation.sh"
```
