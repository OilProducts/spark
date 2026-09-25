---
title: Bug report
description: Reproduce a reported defect, find and verify its root cause and fix, implement and verify the fix, and merge it.
---

Take a reported defect from diagnosis to a merged, verified fix. The objective is the bug report.

1. Reproduce the defect and find its root cause: launch `software-development/investigate-bug.yaml` with the bug report as `context.request.objective`.
2. Propose a fix and have it reviewed adversarially before any code changes. `investigate-bug` does this too: it ends with a verified diagnosis and a verified repair plan (`context.plan.objective`, `context.plan.validation_command`).
3. Implement the fix with an adversarially reviewed, validated change: launch `software-development/implement-change.yaml` with the verified plan as `context.request.objective` and its validation command as `context.request.validation_command`.
4. Verify independently: launch `software-development/review-change.yaml` with the resulting branch as `context.request.source_ref`, and check the original reproduction against that branch yourself.
5. Merge: launch `software-development/merge-change.yaml` with the branch as `context.request.source_ref`.
6. Close the mission with a summary of the cause, the fix, and the evidence that it works.

These are guidelines, not a script. Skip a step when the evidence already covers it; for example, when the objective states an agreed fix, go straight to implementing it. Repeat or reorder steps when results warrant: rerun an investigation that did not reproduce the defect, or return to planning when review finds the fix wrong. Before deciding each next step, read the run's result, and its logs when the result is not enough. Ask the user only when a decision is theirs to make.
