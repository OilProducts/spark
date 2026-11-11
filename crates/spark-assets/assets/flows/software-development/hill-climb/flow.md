---
type: Flow
id: hill_climb
title: Hill Climb
description: Improve work in an isolated worktree, round after round, until every judge scores it at or above a target.
schema_version: '2'
inputs:
- key: context.request.objective
  label: Objective
  type: string
  description: What to improve and for whom, with any constraints the changes must keep and any other work to compare against.
  required: true
- key: context.request.target_score
  label: Target Score
  type: number
  description: Every judge must score the work at or above this, out of 10.
  required: true
- key: context.request.max_rounds
  label: Max Rounds
  type: number
  description: Rounds of judging before the run stops without reaching the target.
  required: true
defaults:
  llm_provider: claude-code
  llm_model: claude-opus-5-5[1m]
  reasoning_effort: xhigh
---

Improve the work in an isolated worktree until every judge scores it at or above the target, or stop honestly when the rounds run out.
