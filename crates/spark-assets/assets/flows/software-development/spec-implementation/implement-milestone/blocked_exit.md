---
type: agent_task
title: Blocked Exit
runtime: {}
contracts:
  reads_context:
  - context.milestone.id
  - context.milestone.title
  response: decision_envelope
execution:
  llm_model: gpt-6.1-sol
  reasoning_effort: xhigh
routes:
  blocked:
    to: failed
    description: This node cannot complete its bounded task without unavailable input or authority and no honest local workaround or deferral remains; record the blocker and its evidence in notes and any required artifacts.
  done:
    to: failed
    description: The bounded work required by this node’s prompt is complete, including its required outputs; this does not assert that the whole workflow objective is achieved.
  unfinished:
    to: blocked_exit
    description: Required work for this bounded node invocation remains and can be continued in the same session; no completed judgment is being asserted.
---

Read context.milestone.id, context.milestone.title, .spark/state/spec-implementation/current/spec/source.md, .spark/state/spec-implementation/current/spec/requirements.json, and the current milestone state under .spark/state/spec-implementation/current/milestones/<context milestone id>/. Treat context.milestone.id plus the milestone-local state as the authoritative identity for this worker even if shared runtime bindings under .spark/state/spec-implementation/current/current-milestone.json or state.json.current_milestone_id drifted during implementation. Write .spark/state/spec-implementation/current/milestones/<context milestone id>/result.json with fields status, outcome, outcome_reason_code, outcome_reason_message, and summary setting the JSON field values "status": "completed", "outcome": "failure", and "outcome_reason_code": "blocked", with outcome_reason_message set to a short summary of what remains blocked and why the milestone could not complete automatically, and the summary citing the originating blocker evidence. Choose `done` only after the result file is written. Respond with ONLY a JSON decision envelope.
