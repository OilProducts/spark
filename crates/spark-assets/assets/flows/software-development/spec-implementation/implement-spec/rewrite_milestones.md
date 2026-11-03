---
type: agent_task
title: Rewrite Milestones
runtime: {}
contracts:
  reads_context:
  - context.milestone.id
  response: decision_envelope
execution:
  llm_provider: claude-code
  llm_model: claude-opus-5-5[1m]
  reasoning_effort: xhigh
routes:
  blocked:
    to: failed
    description: This node cannot complete its bounded task without unavailable input or authority and no honest local workaround or deferral remains; record the blocker and its evidence in notes and any required artifacts.
  done:
    to: validate_milestone_plan
    description: The bounded work required by this node’s prompt is complete, including its required outputs; this does not assert that the whole workflow objective is achieved.
  unfinished:
    to: rewrite_milestones
    description: Required work for this bounded node invocation remains and can be continued in the same session; no completed judgment is being asserted.
---

Read context.milestone.id, .spark/state/spec-implementation/current/milestones.json, and .spark/state/spec-implementation/current/rewrite-milestones.json. Treat context.milestone.id as the authoritative milestone to replace even if shared runtime bindings under .spark/state/spec-implementation/current/current-milestone.json or state.json.current_milestone_id drifted during the child run. Replace that milestone with the milestone entries from rewrite-milestones.json, set each replacement milestone status to pending and attempts to 0 unless already provided, clear current_milestone_id in .spark/state/spec-implementation/current/state.json, remove or clear .spark/state/spec-implementation/current/current-milestone.json, and keep both JSON files valid. Choose `done` only after the milestone plan is updated. Respond with ONLY a JSON decision envelope.
