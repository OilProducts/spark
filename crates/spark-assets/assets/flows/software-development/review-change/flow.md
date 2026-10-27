---
type: Flow
id: review_change
title: Review Change
description: Review a branch or diff with evidence-graded findings that survive an adversarial verification pass.
schema_version: '1'
inputs:
- key: context.request.source_ref
  label: Source Ref
  type: string
  description: Branch or commit to review; the diff is taken against its merge base with the current checkout.
  required: true
- key: context.request.objective
  label: Review Focus
  type: string
  description: Optional focus areas or concerns to weight the review toward.
  required: false
defaults:
  llm_provider: codex
  llm_model: gpt-6.1-sol
  reasoning_effort: xhigh
metadata:
  software_development:
    launcher: true
---

Review a branch or diff with evidence-graded findings that survive an adversarial verification pass.
