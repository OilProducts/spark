---
type: agent_task
title: Recommend Improvements
contracts:
  reads_context:
  - context.retro.assessment
  - context.retro.evidence
  writes_context:
  - context.retro.recommendations
  - context.retro.actionable
  response: decision_envelope
routes:
  blocked:
    to: failed
    description: This node cannot complete its bounded task without unavailable input or authority and no honest local workaround or deferral remains; record the blocker and its evidence in notes and any required artifacts.
  done:
    to: done
    description: The bounded work required by this node’s prompt is complete, including its required outputs; this does not assert that the whole workflow objective is achieved.
  unfinished:
    to: recommend
    description: Required work for this bounded node invocation remains and can be continued in the same session; no completed judgment is being asserted.
---

Turn the verified mission retrospective (`context.retro.assessment`, `context.retro.evidence`) into prioritized, concrete recommendations without changing any files. For each recommendation, name the smallest effective change — a playbook refinement, a flow prompt or graph adjustment, a validation or test improvement, a runtime/CLI defect fix, or a follow-up task for missed scope — where that change lives, and the exact failure mode or wasted work it prevents. Keep speculative or low-value suggestions out. Choose `done` when the recommendations are complete, with `context_updates` setting `context.retro.recommendations` (ordered by impact) and `context.retro.actionable` (the string `"true"` when concrete changes are recommended, otherwise `"false"`). Respond with ONLY a JSON decision envelope.
