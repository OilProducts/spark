---
type: tool
title: Capture Evidence
config:
  decision_field: decision
  env_map:
    SPARK_EVIDENCE_COMMAND: context.climb.evidence_command
    SPARK_EVIDENCE_ROOT: context.climb.evidence_root
    SPARK_ROUND: context.climb.round
    SPARK_WORKSPACE_PATH: context.workspace.path
  output_map:
    context.climb.capture_failed: capture_failed
    context.climb.capture_log: capture_log
    context.climb.commit: commit
    context.climb.evidence_dir: evidence_dir
    context.climb.round: round
runtime:
  max_visits: 64
contracts:
  writes_context:
  - context.climb.round
  - context.climb.evidence_dir
  - context.climb.commit
  - context.climb.capture_log
  - context.climb.capture_failed
routes:
  capture_failed:
    to: climb
    description: The evidence command failed on the climber's latest commit; its output is in the capture log.
  captured:
    to: council
    description: This round's evidence was captured for the commit at HEAD.
---

```sh
# Exit 0 reports captured or capture_failed. Exit 1 means the evidence
# command failed before any change was made, so the request itself is wrong.
set -eu
round=$((SPARK_ROUND + 1))
dir="${SPARK_EVIDENCE_ROOT}/round-${round}"
log="${dir}.log"
rm -rf "$dir"
mkdir -p "$dir"
cd "$SPARK_WORKSPACE_PATH"
commit=$(git rev-parse HEAD)
status=0
: > "$log"
if [ -n "$SPARK_EVIDENCE_COMMAND" ]; then
  SPARK_EVIDENCE_DIR="$dir" sh -ec "$SPARK_EVIDENCE_COMMAND" >"$log" 2>&1 </dev/null || status=$?
fi
if [ "$status" -ne 0 ] && [ "$round" -eq 1 ]; then
  echo "evidence command failed on the base commit (exit $status):" >&2
  tail -n 50 "$log" >&2
  exit 1
fi
if [ "$status" -eq 0 ]; then
  printf '{"decision":"captured","round":%d,"evidence_dir":"%s","commit":"%s","capture_log":"%s","capture_failed":false}\n' \
    "$round" "$dir" "$commit" "$log"
else
  printf '{"decision":"capture_failed","round":%d,"evidence_dir":"%s","commit":"%s","capture_log":"%s","capture_failed":true}\n' \
    "$SPARK_ROUND" "$dir" "$commit" "$log"
fi
```
