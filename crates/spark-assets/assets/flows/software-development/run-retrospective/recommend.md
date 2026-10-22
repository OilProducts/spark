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
  response: status_envelope
routes:
- to: done
  condition: outcome=success
---

Turn the verified diagnosis (context.retro.diagnosis, context.retro.evidence) into recommendations, without changing any files. State the smallest change that prevents this failure class from recurring — a flow prompt or graph change, a contract tightening, a validation fix, or a runtime defect repair — and where that change lives. When the fix belongs in the current repository checkout, also produce one bounded, self-contained implementation task (the defect, the fix, acceptance criteria including a check that would have caught this run's failure); when the fix lives outside this repository (an installed flow, configuration, or operator action), say so and leave the task empty. Respond with ONLY a JSON status envelope whose context_updates set context.retro.recommendations, context.retro.task (the implementable task statement, or an empty string), and context.retro.actionable (the string "true" when context.retro.task contains an implementable repository task, otherwise "false").
