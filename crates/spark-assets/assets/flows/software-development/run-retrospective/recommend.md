---
type: agent_task
title: Recommend Fixes
contracts:
  reads_context:
  - context.retro.diagnosis
  - context.retro.evidence
  writes_context:
  - context.retro.recommendations
  - context.retro.task
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

Turn the verified diagnosis (context.retro.diagnosis, context.retro.evidence) into recommendations, without changing any files. State the smallest change that prevents this failure class from recurring — a flow prompt or graph change, a contract tightening, a validation fix, or a runtime defect repair — and where that change lives. When the fix belongs in the current repository checkout, also produce one bounded, self-contained implementation task (the defect, the fix, acceptance criteria including a check that would have caught this run's failure); when the fix lives outside this repository (an installed flow, configuration, or operator action), say so and leave the task empty. Choose `done` when the recommendations are complete, with context_updates setting context.retro.recommendations, context.retro.task (the implementable task statement, or an empty string), and context.retro.actionable (the string "true" when context.retro.task contains an implementable repository task, otherwise "false"). Respond with ONLY a JSON decision envelope.
