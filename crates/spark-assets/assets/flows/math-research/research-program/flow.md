---
type: Flow
id: research_program
title: Research Program
description: Repeatedly perform the most useful next mathematical action, assess its evidence, update a concise claim state, and commit the result until the exact problem is mechanically resolved or a human cancels the run. Launch with execution_profile_id math-lab.
schema_version: '1'
inputs:
- key: context.request.problem
  label: Problem
  type: string
  description: The precise mathematical problem to investigate, with known constraints or results.
  required: true
- key: context.request.focus
  label: Focus
  type: string
  description: Optional standing scope, resource, or methodology constraints.
  required: false
defaults:
  llm_provider: codex
  llm_model: gpt-6.1-sol
  reasoning_effort: xhigh
metadata:
  math_research:
    launcher: true
    required_execution_profile: math-lab
---

Improve justified knowledge about one mathematical problem until a faithful kernel-checked proof, a directly verified counterexample, or an explicitly human-accepted reviewed proof resolves it.
