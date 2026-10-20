---
type: Flow
id: software_development_resolve_merge_conflicts
title: Resolve Merge Conflicts
description: Hidden reusable software-development worker.
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

Resolve only unambiguous merge conflicts inside the current integration worktree.
