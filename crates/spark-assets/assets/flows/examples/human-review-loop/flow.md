---
type: Flow
id: human_review_loop
title: Human Review Loop
description: Draft a change, route it through explicit human approval, and loop back on requested fixes.
schema_version: '1'
defaults:
  llm_provider: codex
  llm_model: gpt-6.1-sol
  reasoning_effort: xhigh
---

Prepare a proposed change, pause for human review, and either ship it or iterate on fixes.
