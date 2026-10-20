---
type: agent_task
title: Blocked Exit
runtime: {}
contracts:
  writes_context:
  - context.workflow_outcome
  - context.workflow_outcome_reason_code
  - context.workflow_outcome_reason_message
  response: status_envelope
retry:
  max_retries: 1
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

Inspect .spark/spec-implementation/current/state.json, .spark/spec-implementation/current/spec/source.md, .spark/spec-implementation/current/spec/requirements.json, .spark/spec-implementation/current/milestones.json, and any current milestone artifacts. Write context.workflow_outcome="failure", context.workflow_outcome_reason_code="blocked", and context.workflow_outcome_reason_message to a short summary of what remains blocked and why the program could not complete automatically. Respond with ONLY a JSON status envelope and return outcome success only after those context updates are included.
