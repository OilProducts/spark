---
type: agent_task
title: 'Proposal: Minimal Diff'
contracts:
  reads_context:
  - context.request.objective
  - context.request.target_paths
  - context.request.constraints
  response: status_envelope
routes:
- to: fan_in
---

Propose an approach to context.request.objective from the minimal-diff stance: the smallest coherent change that truthfully satisfies the goal, reusing existing code, stdlib, and installed dependencies before writing anything new. Inspect the repository read-only; do not change any files. Respect context.request.constraints and weight context.request.target_paths. Your proposal must be decision-complete: the concrete changes by file or surface, what is deliberately not done, acceptance criteria, compatibility risks with evidence from the actual code, and the truthful validation command. State the real costs of staying minimal, not just the benefits. Respond with ONLY a JSON status envelope; put the full proposal in your response.
