---
type: agent_task
title: Diagnose Validation
runtime:
  max_visits: 3
contracts:
  reads_context:
  - context.workspace.path
  - context.task.objective
  - context.task.validation_command
  - context.tool.output
  - context.tool.exit_code
  writes_context:
  - context.review.summary
  - context.review.required_changes
  response: status_envelope
routes:
- to: implement
  label: Fix
  condition: outcome=success
---

The deterministic validation gate just failed for the implementation in the isolated worktree at context.workspace.path. context.task.validation_command is the gate command, context.tool.output holds the tail of its log, and context.tool.exit_code its exit code. Your job is to translate that failure into the minimal fix guidance the implementer needs; you have no authority to judge the gate green — it reruns deterministically after the next implementation attempt. Diagnose with deterministic evidence: rerun the narrowest failing check inside the worktree to confirm the failure and localize it. Distinguish a failure caused by the implementation from an environmental or flaky failure: for implementation failures, name the concrete defect and the smallest change that fixes it; for failures you can show are environmental or nondeterministic (rerun the failing check in isolation to demonstrate it), say so explicitly and direct the implementer to verify and, when the cause is in the repository, stabilize it rather than change unrelated code. This node may read files and run commands in the worktree but must not edit files, stage, or commit. Set context_updates for context.review.summary (concise diagnosis with evidence basis) and context.review.required_changes (a minimal, actionable fix prompt). Respond with ONLY a JSON status envelope; return outcome success once the diagnosis is recorded.
