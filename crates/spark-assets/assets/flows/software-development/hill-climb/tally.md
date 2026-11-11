---
type: tool
title: Tally
config:
  decision_field: decision
  env_map:
    SPARK_COMMIT: context.climb.commit
    SPARK_HISTORY: context.climb.history
    SPARK_MAX_ROUNDS: context.climb.max_rounds
    SPARK_RESULTS: parallel.results
    SPARK_ROUND: context.climb.round
    SPARK_RUBRIC: context.climb.rubric
    SPARK_TARGET: context.climb.target_score
  output_map:
    context.climb.findings: findings
    context.climb.history: history
runtime:
  max_visits: 64
contracts:
  writes_context:
  - context.climb.history
  - context.climb.findings
routes:
  climb:
    to: climb
    description: Some score is below the target, or some member did not score, and rounds remain.
  exhausted:
    to: unmet
    description: The last allowed round ended without every score reaching the target.
  passed:
    to: done
    description: Every council member scored every rubric category at or above the target this round.
---

```sh
# A member passes only when it completed and scored every rubric category
# as a number at or above the target; any other member keeps the climb going.
set -eu
command -v jq >/dev/null 2>&1 || { echo "tally: jq is required" >&2; exit 1; }
printf '%s' "$SPARK_RESULTS" | jq -c \
  --argjson rubric "$SPARK_RUBRIC" --argjson target "$SPARK_TARGET" \
  --argjson round "$SPARK_ROUND" --argjson max "$SPARK_MAX_ROUNDS" \
  --argjson history "$SPARK_HISTORY" --arg commit "$SPARK_COMMIT" '
  def passes: .status == "completed" and (.scores | type) == "object"
    and ([.scores[$rubric[]]] | all(type == "number" and . >= $target));
  [.[] | {member: .id, status,
          scores: .context["context.council.scores"],
          reference_scores: .context["context.council.reference_scores"],
          findings: .context["context.council.findings"],
          error}] as $members
  | {decision: (if ($members | length > 0 and all(passes)) then "passed"
                elif $round >= $max then "exhausted" else "climb" end),
     history: ($history + [{round: $round, commit: $commit,
               members: [$members[] | {member, status, scores, reference_scores}]}]),
     findings: [$members[] | {member, status, findings, error}]}'
```
