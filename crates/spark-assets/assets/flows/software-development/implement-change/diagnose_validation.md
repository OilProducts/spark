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
  - context.validation.command_exit_code
  - context.validation.command_output
  writes_context:
  - context.review.summary
  - context.review.required_changes
  response: decision_envelope
routes:
  blocked:
    to: failed
    description: This node cannot complete its bounded task without unavailable input or authority and no honest local workaround or deferral remains; record the blocker and its evidence in notes and any required artifacts.
  done:
    to: implement
    description: The bounded work required by this node’s prompt is complete, including its required outputs; this does not assert that the whole workflow objective is achieved.
  unfinished:
    to: diagnose_validation
    description: Required work for this bounded node invocation remains and can be continued in the same session; no completed judgment is being asserted.
---

The deterministic validation gate just failed for the implementation in the isolated worktree at context.workspace.path. context.task.validation_command is the gate command, context.validation.command_output holds the tail of its log, and context.validation.command_exit_code the validation command's own exit status. Your job is to translate that failure into the minimal fix guidance the implementer needs; you have no authority to judge the gate green — it reruns deterministically after the next implementation attempt. Diagnose with deterministic evidence: rerun the narrowest failing check inside the worktree to confirm the failure and localize it. Distinguish a failure caused by the implementation from an environmental or flaky failure: for implementation failures, name the concrete defect and the smallest change that fixes it; for failures you can show are environmental or nondeterministic (rerun the failing check in isolation to demonstrate it), say so explicitly and direct the implementer to verify and, when the cause is in the repository, stabilize it rather than change unrelated code. This node may read files and run commands in the worktree but must not edit files, stage, or commit. Set context_updates for context.review.summary (concise diagnosis with evidence basis) and context.review.required_changes (a minimal, actionable fix prompt). Choose `done` once the diagnosis is recorded. Respond with ONLY a JSON decision envelope.
