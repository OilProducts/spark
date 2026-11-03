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
  response: decision_envelope
routes:
  blocked:
    to: failed
    description: This node cannot complete its bounded task without unavailable input or authority and no honest local workaround or deferral remains; record the blocker and its evidence in notes and any required artifacts.
  done:
    to: commit
    description: The bounded work required by this node’s prompt is complete, including its required outputs; this does not assert that the whole workflow objective is achieved.
  unfinished:
    to: record
    description: Required work for this bounded node invocation remains and can be continued in the same session; no completed judgment is being asserted.
---

Record the independently assessed research action. Read problem.md, state.md, the current evidence directory, and its assessment. Update state.md so it remains a concise current claim state rather than a diary: verdict, established claims with stable IDs and evidence pointers, dependencies, ranked open frontier, and approaches not to repeat unchanged. Accept no claim beyond its assessment verdict. Record the assessment of substantive progress separately from claim correctness. If substantive_progress is false, preserve the valid evidence without letting repeated certification or bookkeeping dominate state.md, and prohibit another action of the same kind unless it states a new resolution-relevant consequence. Write exactly one immutable history/<ACTION>.md describing why the action was selected, what was attempted, what was accepted/downgraded/rejected, whether and how the frontier changed, what changed in state.md, and what a future attempt must do differently. Preserve unsuccessful evidence. Do not rewrite older history records. Set context_updates for context.state.summary, then choose `done` once this step's required outputs are written. Respond with ONLY a JSON decision envelope.
