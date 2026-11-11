---
type: Flow
id: hill_climb
title: Hill Climb
description: Improve work in an isolated worktree, round after round, until an independent council of models scores every rubric category at or above a target.
schema_version: '2'
inputs:
- key: context.request.objective
  label: Objective
  type: string
  description: What to improve and for whom, with any constraints the changes must keep.
  required: true
- key: context.request.rubric
  label: Rubric
  type: string[]
  description: Categories the council scores, such as typography or error handling. Derived from the objective when omitted.
  required: false
- key: context.request.target_score
  label: Target Score
  type: number
  description: Every council member must score every category at or above this, out of 10. Defaults to 9.
  required: false
- key: context.request.max_rounds
  label: Max Rounds
  type: number
  description: Council rounds before the run stops without reaching the target, at most 20. Defaults to 8.
  required: false
- key: context.request.evidence_command
  label: Evidence Command
  type: string
  description: Shell command run in the worktree each round that writes evidence, such as screenshots or eval output, into $SPARK_EVIDENCE_DIR. When omitted, the council inspects the worktree directly.
  required: false
- key: context.request.references
  label: References
  type: string[]
  description: Paths or descriptions of comparable work the council also scores each round, to calibrate its scores.
  required: false
- key: context.request.base_ref
  label: Base Ref
  type: string
  description: Commit to climb from. Defaults to HEAD.
  required: false
defaults:
  llm_provider: claude-code
  llm_model: claude-opus-5-5[1m]
  reasoning_effort: xhigh
---

Improve the work in an isolated worktree until every council member scores every rubric category at or above the target, or stop honestly when the rounds run out.
