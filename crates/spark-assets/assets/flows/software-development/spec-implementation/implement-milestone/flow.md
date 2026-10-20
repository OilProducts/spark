---
type: Flow
id: implement_milestone
title: Implement Milestone
description: A child milestone worker for the software-development spec implementation program. The committed spec package lives under specs/<slug>/ and is surfaced to this flow through the stable .spark/spec-implementation/current/spec/ symlink. It keeps the authoritative live active-item binding in protected context.item.* keys and the durable child queue ledger under .spark/spec-implementation/current/milestones/<id>/state.json while shared runtime binding files and validation artifacts under .spark/spec-implementation/current/ remain runtime control data rather than the active item-control plane.
schema_version: '1'
defaults:
  fidelity: summary:high
  llm_provider: codex
  llm_model: gpt-6.1-sol
  reasoning_effort: xhigh
metadata:
  software_development:
    launcher: false
    worker: true
---

Implement the bound current milestone from the active software-development runtime alias exposed at .spark/spec-implementation/current/current-milestone.json by decomposing it into bounded items, implementing them, validating the repository, and recording a milestone result.
