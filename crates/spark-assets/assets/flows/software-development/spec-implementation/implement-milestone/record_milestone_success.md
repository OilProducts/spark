---
type: agent_task
title: Record Milestone Success
runtime: {}
contracts:
  reads_context:
  - context.milestone.id
  - context.milestone.requirement_ids
  response: status_envelope
execution:
  llm_model: gpt-6.1-sol
  reasoning_effort: xhigh
routes:
- to: done
  extensions:
    loop_restart: false
---

Read context.milestone.id, context.milestone.requirement_ids, the milestone state in .spark/spec-implementation/current/milestones/<context milestone id>/state.json, and .spark/spec-implementation/current/validation-result.json if it exists. Treat context.milestone.id plus milestone-local state as authoritative even if shared runtime milestone metadata drifted during implementation. Write .spark/spec-implementation/current/milestones/<context milestone id>/result.json with fields status, outcome, summary, completed_requirement_ids, and evidence using status=completed and outcome=success. The completed_requirement_ids list should come from context.milestone.requirement_ids. Respond with ONLY a JSON status envelope and return outcome success only after the result file is written.
