---
type: Flow
id: implement_review_loop
title: Implement And Review Loop
description: Plan a change, implement it, review the result, and loop on failures until the work is ready.
schema_version: '1'
inputs:
- key: context.request.summary
  label: Request Summary
  type: string
  description: Short description of the requested change.
  required: true
- key: context.request.target_paths
  label: Target Paths
  type: string[]
  description: Optional files or directories the flow should focus on.
  required: false
- key: context.request.acceptance_criteria
  label: Acceptance Criteria
  type: string[]
  description: Optional criteria the finished work should satisfy.
  required: false
defaults:
  llm_provider: codex
  llm_model: gpt-6.1-sol
  reasoning_effort: xhigh
---

Plan, implement, review, and iterate on a codebase change until it is ready.
