---
type: tool
title: Tally
config:
  decision_field: decision
  env_map:
    SPARK_HISTORY: context.climb.history
    SPARK_MAX_ROUNDS: context.request.max_rounds
    SPARK_RESULTS: parallel.results
    SPARK_ROUND: context.climb.round
    SPARK_TARGET: context.request.target_score
    SPARK_WORKSPACE_PATH: context.workspace.path
  output_map:
    context.climb.findings: findings
    context.climb.history: history
    context.climb.outcome: decision
    context.climb.round: round
runtime:
  max_visits: 64
contracts:
  writes_context:
  - context.climb.round
  - context.climb.history
  - context.climb.findings
  - context.climb.outcome
routes:
  climb:
    to: climb
    description: Some judge scored below the target or did not score, and rounds remain.
  exhausted:
    to: cleanup
    description: The last allowed round ended without every judge reaching the target.
  passed:
    to: cleanup
    description: Every judge scored the work at or above the target this round.
---

```sh
# A judge passes only when it completed and scored a number at or above the
# target; any other judge keeps the climb going.
set -eu
command -v jq >/dev/null 2>&1 || { echo "tally: jq is required" >&2; exit 1; }
commit=$(git -C "$SPARK_WORKSPACE_PATH" rev-parse HEAD)
printf '%s' "$SPARK_RESULTS" | jq -c \
  --argjson target "$SPARK_TARGET" --argjson round "$SPARK_ROUND" \
  --argjson max "$SPARK_MAX_ROUNDS" --argjson history "$SPARK_HISTORY" \
  --arg commit "$commit" '
  ($round + 1) as $round
  | [.[] | {judge: .id, status, score: .context["context.judge.score"],
            findings: .context["context.judge.findings"], error}] as $judges
  | {decision: (if ($judges | length > 0) and all($judges[]; .status == "completed"
                    and (.score | type) == "number" and .score >= $target) then "passed"
                elif $round >= $max then "exhausted" else "climb" end),
     round: $round,
     history: ($history + [{round: $round, commit: $commit,
               scores: [$judges[] | {judge, status, score}]}]),
     findings: $judges}'
```
