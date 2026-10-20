---
type: tool
title: Count Validate Attempt
config:
  env_map:
    SPARK_VALIDATION_ATTEMPTS: context.validation.attempts
  output_map:
    context.validation.attempts: attempts
contracts:
  writes_context:
  - context.validation.attempts
routes:
- to: diagnose_validation
  condition: outcome=success
---

```sh
set -eu
attempts=$((SPARK_VALIDATION_ATTEMPTS + 1))
if [ "$attempts" -gt 3 ]; then
  echo "validation gate failed ${attempts} times; fix budget (3 attempts) exhausted" >&2
  exit 1
fi
printf '{"attempts":%s}' "$attempts"
```
