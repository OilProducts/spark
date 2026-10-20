---
type: agent_task
title: 'Proposal: Refactor First'
contracts:
  reads_context:
  - context.request.objective
  - context.request.target_paths
  - context.request.constraints
  response: status_envelope
routes:
- to: fan_in
---

Propose an approach to context.request.objective from the refactor-first stance: if the goal is fighting the current structure, name the smallest structural change that makes the goal natural, then the goal itself on top of it. Inspect the repository read-only; do not change any files. Respect context.request.constraints and weight context.request.target_paths. Your proposal must be decision-complete: the concrete changes by file or surface, acceptance criteria, migration and compatibility risks with evidence from the actual code, and the truthful validation command. Justify every piece of restructuring against a cost the current shape actually imposes — no speculative architecture. Respond with ONLY a JSON status envelope; put the full proposal in your response.
