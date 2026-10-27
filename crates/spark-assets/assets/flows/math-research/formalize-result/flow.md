---
type: Flow
id: formalize_result
title: Formalize Result
description: Formalize a stated mathematical result in Lean 4 against the math-lab container's pinned Mathlib, gate on a kernel-checked build with no sorry and no new axioms, and adversarially verify that the formal statement faithfully encodes the requested claim. Launch with execution_profile_id math-lab.
execution_lock:
  scope: project
  key: math-research
  conflict_policy: queue
schema_version: '1'
inputs:
- key: context.request.statement
  label: Statement
  type: string
  description: The mathematical claim to formalize, stated precisely in informal language.
  required: true
- key: context.request.reference
  label: Reference
  type: string
  description: Optional source for the result and its proof (paper, book, URL, or a proof sketch).
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

Produce a kernel-checked Lean formalization of the requested result whose formal statement faithfully encodes the informal claim.
