---
type: agent_task
title: Blocked Exit
runtime: {}
contracts:
  reads_context:
  - context.milestone.id
  - context.milestone.title
  writes_context:
  - context.workflow_outcome
  - context.workflow_outcome_reason_code
  - context.workflow_outcome_reason_message
  response: status_envelope
execution:
  llm_model: gpt-6.1-sol
  reasoning_effort: xhigh
routes:
- to: done
  label: Finish Blocked
  condition: outcome=success
  extensions:
    loop_restart: false
- to: done
  label: Finish Partial Blocked
  condition: outcome=partial_success
  extensions:
    loop_restart: false
---

Read context.milestone.id, context.milestone.title, .spark/spec-implementation/current/spec/source.md, .spark/spec-implementation/current/spec/requirements.json, and the current milestone state under .spark/spec-implementation/current/milestones/<context milestone id>/. Treat context.milestone.id plus the milestone-local state as the authoritative identity for this worker even if shared runtime bindings under .spark/spec-implementation/current/current-milestone.json or state.json.current_milestone_id drifted during implementation. Write .spark/spec-implementation/current/milestones/<context milestone id>/result.json with fields status, outcome, outcome_reason_code, outcome_reason_message, and summary using status=completed and outcome=failure. Also write context.workflow_outcome="failure", context.workflow_outcome_reason_code="blocked", and context.workflow_outcome_reason_message to a short summary of what remains blocked and why the milestone could not complete automatically. Respond with ONLY a JSON status envelope and return outcome success only after the result file and context updates are written.
