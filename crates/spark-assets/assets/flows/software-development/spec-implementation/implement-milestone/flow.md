---
type: Flow
id: implement_milestone
title: Implement Milestone
description: A child milestone worker for the software-development spec implementation program. The committed spec package lives under specs/<slug>/ and is surfaced to this flow through the stable the project work area's spec-implementation/current/spec/ symlink. It keeps the authoritative live active-item binding in protected context.item.* keys and the durable child queue ledger under the project work area's spec-implementation/current/milestones/<id>/state.json while shared runtime binding files and validation artifacts under the project work area's spec-implementation/current/ remain runtime control data rather than the active item-control plane.
schema_version: '2'
inputs:
- key: context.milestone.id
  label: Milestone ID
  type: string
  description: Bound by the parent spec implementation run.
  required: false
- key: context.milestone.title
  label: Milestone Title
  type: string
  description: Bound by the parent spec implementation run.
  required: false
- key: context.milestone.objective
  label: Milestone Objective
  type: string
  description: Bound by the parent spec implementation run.
  required: false
- key: context.milestone.requirement_ids
  label: Milestone Requirement IDs
  type: string[]
  description: Bound by the parent spec implementation run.
  required: false
- key: context.milestone.decision_ids
  label: Milestone Decision IDs
  type: string[]
  description: Bound by the parent spec implementation run.
  required: false
- key: context.milestone.acceptance_criteria
  label: Milestone Acceptance Criteria
  type: string[]
  description: Bound by the parent spec implementation run.
  required: false
- key: context.milestone.target_paths
  label: Milestone Target Paths
  type: string[]
  description: Bound by the parent spec implementation run.
  required: false
- key: context.milestone.attempts
  label: Milestone Attempts
  type: number
  description: Bound by the parent spec implementation run.
  required: false
defaults:
  llm_provider: codex
  llm_model: gpt-6.2-sol
  reasoning_effort: xhigh
metadata:
  software_development:
    launcher: false
    worker: true
---

Implement the bound current milestone from the active software-development runtime alias exposed at the project work area's spec-implementation/current/current-milestone.json by decomposing it into bounded items, implementing them, validating the repository, and recording a milestone result.
