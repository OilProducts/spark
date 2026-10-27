---
type: Flow
id: design_change
title: Design Change
description: Propose competing approaches to a goal in parallel and judge them on evidence and minimality. The run ends with the judged winning approach.
schema_version: '1'
inputs:
- key: context.request.objective
  label: Objective
  type: string
  description: The goal or problem statement whose implementation approach is undecided.
  required: true
- key: context.request.target_paths
  label: Target Paths
  type: string[]
  description: ''
  required: false
- key: context.request.constraints
  label: Constraints
  type: string[]
  description: ''
  required: false
defaults:
  llm_provider: codex
  llm_model: gpt-6.1-sol
  reasoning_effort: xhigh
metadata:
  software_development:
    launcher: true
---

Propose competing approaches to a goal in parallel and judge them on evidence and minimality. The run ends with the judged winning approach.
