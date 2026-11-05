---
type: agent_task
title: 'Draft: Smallest Root-Cause Fix (Sol)'
contracts:
  reads_context:
  - context.request.objective
  - context.request.target_paths
  - context.request.constraints
  writes_context:
  - context.approach.draft
  response: decision_envelope
execution:
  llm_provider: codex
  llm_model: gpt-6.1-sol
routes:
  blocked:
    to: failed
    description: This node cannot complete its bounded task without unavailable input or authority and no honest local workaround or deferral remains; record the blocker and its evidence in notes and any required artifacts.
  done:
    to: explore_join
    description: The bounded work required by this node’s prompt is complete, including its required outputs; this does not assert that the whole workflow objective is achieved.
  unfinished:
    to: smallest_sol
    description: Required work for this bounded node invocation remains and can be continued in the same session; no completed judgment is being asserted.
---

Draft one approach to the change in context.request.objective, respecting context.request.constraints and weighting context.request.target_paths. Other drafters are working on the same request independently, and one draft will be chosen.

Aim for the smallest change that removes the root cause. Find the cause first, then change as little as it takes to remove it, preferring to delete or reuse over adding. A small patch that handles one symptom or one case and leaves the cause in place does not meet this aim. Report anything this leaves for later under Tradeoffs.

Check how comparable systems solve this problem, and say what you took or rejected and why. If nothing comparable exists, say so and move on.

Verify every file, symbol and behaviour you cite against the current code. Write the draft in clear prose, as short as clarity allows, with these sections: Root cause, Approach, Prior art, Tradeoffs. Leave out the implementation plan. If the request is unclear, state your assumption in the draft. Do not change any files. Choose `done` when the draft is complete, with context_updates setting context.approach.draft to its full text. Respond with ONLY a JSON decision envelope.
