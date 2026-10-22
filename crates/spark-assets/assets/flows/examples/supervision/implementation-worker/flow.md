---
type: Flow
id: implementation_worker
title: Implementation Worker
description: A reusable child workflow that makes a codebase change, reviews the result, and loops until it is ready.
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
  fidelity: summary:high
  llm_provider: codex
  llm_model: gpt-6.1-sol
  reasoning_effort: xhigh
---

Attempt a repository change, review it, and keep iterating until the result is ready or the attempt fails.
