---
type: Flow
id: review_organization_change
title: Review Organization Change
description: Review a proposed change to the organization's spec, roles, flows, playbooks or triggers, given as a unified diff of the project's .spark/definitions, biased toward fewer and more general rules, and return approve, revise or reject.
schema_version: '1'
inputs:
- key: context.request.proposal_path
  label: Proposal
  type: string
  description: Path of a unified diff under the project's .spark/state/reviews/ holding the proposed change to .spark/definitions, with paths relative to the project root.
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

Review a proposed change to the organization's spec, roles, flows, playbooks or triggers, given as a unified diff of the project's .spark/definitions, biased toward fewer and more general rules, and return approve, revise or reject.
