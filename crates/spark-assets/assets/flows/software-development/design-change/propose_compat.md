---
type: agent_task
title: 'Proposal: Compatibility First'
contracts:
  reads_context:
  - context.request.objective
  - context.request.target_paths
  - context.request.constraints
  response: status_envelope
routes:
- to: fan_in
---

Propose an approach to context.request.objective from the compatibility-and-operability stance: prioritize preserving existing behavior, interfaces, data, and operational properties (resume, persistence, concurrency, failure recovery), and make risks observable. Inspect the repository read-only; do not change any files. Respect context.request.constraints and weight context.request.target_paths. Your proposal must be decision-complete: the concrete changes by file or surface, acceptance criteria including regression checks for the surfaces you protect, compatibility boundaries with evidence from the actual code, and the truthful validation command. State what this stance costs in added care versus the minimal approach. Respond with ONLY a JSON status envelope; put the full proposal in your response.
