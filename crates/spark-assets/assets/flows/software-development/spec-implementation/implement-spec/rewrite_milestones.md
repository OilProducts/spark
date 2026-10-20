---
type: agent_task
title: Rewrite Milestones
runtime: {}
contracts:
  reads_context:
  - context.milestone.id
  response: status_envelope
execution:
  llm_provider: claude-code
  llm_model: claude-opus-5-5[1m]
  reasoning_effort: xhigh
routes:
- to: validate_milestone_plan
  extensions:
    loop_restart: false
---

Read context.milestone.id, .spark/spec-implementation/current/milestones.json, and .spark/spec-implementation/current/rewrite-milestones.json. Treat context.milestone.id as the authoritative milestone to replace even if shared runtime bindings under .spark/spec-implementation/current/current-milestone.json or state.json.current_milestone_id drifted during the child run. Replace that milestone with the milestone entries from rewrite-milestones.json, set each replacement milestone status to pending and attempts to 0 unless already provided, clear current_milestone_id in .spark/spec-implementation/current/state.json, remove or clear .spark/spec-implementation/current/current-milestone.json, keep both JSON files valid, and respond with ONLY a JSON status envelope. Return outcome success only after the milestone plan is updated.
