---
type: Flow
id: review_organization_change
title: Review Organization Change
description: Review a change to the organization's spec, roles, flows, playbooks or triggers on an org repo branch, biased toward fewer and more general rules, and return approve, revise or reject.
schema_version: '1'
inputs:
- key: context.request.source_ref
  label: Source Ref
  type: string
  description: Branch in the org repo holding the change; the diff is taken against its merge base with the org repo's checked-out branch.
  required: true
- key: context.request.objective
  label: Reason
  type: string
  description: Why the change is needed, with its evidence, such as the operator's words or the runs where the problem occurred.
  required: true
defaults:
  llm_provider: codex
  llm_model: gpt-6.1-sol
  reasoning_effort: xhigh
---

Review a change to the organization's spec, roles, flows, playbooks or triggers on an org repo branch, biased toward fewer and more general rules, and return approve, revise or reject.
