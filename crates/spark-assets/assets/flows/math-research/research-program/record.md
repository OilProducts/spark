---
type: agent_task
title: Update Knowledge
contracts:
  reads_context:
  - context.action.id
  - context.assessment.summary
  - context.assessment.resolution_candidate
  writes_context:
  - context.state.summary
  response: status_envelope
routes:
- to: commit
  condition: outcome=success
---

Record the independently assessed research action. Read problem.md, state.md, the current evidence directory, and its assessment. Update state.md so it remains a concise current claim state rather than a diary: verdict, established claims with stable IDs and evidence pointers, dependencies, ranked open frontier, and approaches not to repeat unchanged. Accept no claim beyond its assessment verdict. Record the assessment of substantive progress separately from claim correctness. If substantive_progress is false, preserve the valid evidence without letting repeated certification or bookkeeping dominate state.md, and prohibit another action of the same kind unless it states a new resolution-relevant consequence. Write exactly one immutable history/<ACTION>.md describing why the action was selected, what was attempted, what was accepted/downgraded/rejected, whether and how the frontier changed, what changed in state.md, and what a future attempt must do differently. Preserve unsuccessful evidence. Do not rewrite older history records. Respond with ONLY a JSON status envelope whose context_updates set context.state.summary.
