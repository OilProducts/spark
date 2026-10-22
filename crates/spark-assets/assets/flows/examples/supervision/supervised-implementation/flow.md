---
type: Flow
id: supervised_implementation
title: Supervised Implementation
description: Demonstrate a parent-child flow by running a bundled implementation worker and summarizing success or failure at the parent level.
schema_version: '1'
defaults:
  llm_provider: codex
  llm_model: gpt-6.1-sol
  reasoning_effort: xhigh
---

Run a bundled child implementation worker and summarize whether the change attempt succeeded.
