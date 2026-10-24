---
type: Flow
id: explore_conjecture
title: Explore Conjecture
description: Survey an open problem, run computational experiments with the math-lab container's solver stack, verify every certificate, and synthesize honestly-labeled findings that are adversarially reviewed before commit. Launch with execution_profile_id math-lab.
execution_lock:
  scope: project
  key: math-research
  conflict_policy: queue
schema_version: '1'
inputs:
- key: context.request.problem
  label: Problem
  type: string
  description: The open problem or conjecture to explore, with any known constraints or prior results worth starting from.
  required: true
- key: context.request.focus
  label: Focus
  type: string
  description: Optional focus for this session (a case, a bound, a construction family, a computational budget).
  required: false
defaults:
  fidelity: summary:high
  llm_provider: codex
  llm_model: gpt-6.1-sol
  reasoning_effort: xhigh
metadata:
  math_research:
    launcher: true
    required_execution_profile: math-lab
---

Produce verified computational evidence and honestly-labeled findings about the stated problem.
