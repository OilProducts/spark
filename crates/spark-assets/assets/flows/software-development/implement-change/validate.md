---
type: tool
title: Validate
config:
  decision_field: decision
  env_map:
    SPARK_VALIDATION_COMMAND: context.task.validation_command
    SPARK_WORKSPACE_PATH: context.workspace.path
  output_map:
    context.validation.command_exit_code: command_exit_code
    context.validation.command_output: command_output
contracts:
  writes_context:
  - context.validation.command_exit_code
  - context.validation.command_output
routes:
  passed:
    to: finalize_commit
    description: The configured repository validation command returned zero and its evidence was captured.
  rejected:
    to: diagnose_validation
    description: That command returned nonzero; diagnose the captured output before further implementation.
---

```sh
# Prints one JSON object and exits 0 for both decisions: passed when the
# validation command exits 0, rejected for any nonzero command status. Any
# nonzero exit of this wrapper (workspace, jq, log allocation, read or
# removal failure) is an execution error, not a validation result.
set -eu
command -v jq >/dev/null 2>&1 || { echo "validate: jq is required" >&2; exit 1; }
cd "$SPARK_WORKSPACE_PATH"
log=$(mktemp)
tail_log=$(mktemp)
trap 'rm -f "$log" "$tail_log"' EXIT
status=0
sh -ec "$SPARK_VALIDATION_COMMAND" >"$log" 2>&1 || status=$?
if [ "$status" -eq 0 ]; then
  decision=passed
  limit=2000
else
  decision=rejected
  limit=20000
fi
tail -c "$limit" "$log" >"$tail_log"
result=$(jq -Rsc --arg decision "$decision" --argjson code "$status" \
  '{decision: $decision, command_exit_code: $code, command_output: .}' <"$tail_log")
rm -f "$log" "$tail_log"
printf '%s\n' "$result"
```
