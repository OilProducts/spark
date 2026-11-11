---
type: tool
title: Sync Judges
config:
  env_map:
    SPARK_BRANCH: context.workspace.branch
    SPARK_OPUS_PATH: context.workspace.opus_path
    SPARK_ROUND: context.climb.round
    SPARK_RUN_ID: internal.run_id
    SPARK_SOL_PATH: context.workspace.sol_path
  exit_codes:
    '0': done
  output_map:
    context.climb.evidence_dir: evidence_dir
runtime:
  max_visits: 64
contracts:
  writes_context:
  - context.climb.evidence_dir
routes:
  done:
    to: judges
    description: Every judge's checkout is at the branch's latest commit and this round's evidence folder exists.
---

```sh
# Tracked files match the branch; ignored build output stays, so each
# judge's rebuilds are incremental. Evidence is kept per round for people
# to follow the climb; the run never deletes it.
set -eu
for dir in "$SPARK_OPUS_PATH" "$SPARK_SOL_PATH"; do
  git -C "$dir" checkout --quiet --force --detach "$SPARK_BRANCH"
done
evidence="${SPARK_PROJECT_WORK_DIR}/hill-climb/${SPARK_RUN_ID}/round-$((SPARK_ROUND + 1))"
mkdir -p "$evidence/opus" "$evidence/sol"
printf '{"evidence_dir":"%s"}' "$evidence"
```
