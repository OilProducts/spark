---
id: CR-2026-0001-move-codergen-loop-to-top-level-agent-package
title: Move Codergen Loop To Top-Level `agent` Package
status: completed
type: refactor
changelog: public
---

## Summary

The codergen loop is now exposed through the top-level `agent` package under `src/agent/`. The old `unified_llm.agent` package files were removed instead of replaced by a compatibility shim, while the moved agent implementation continues to import SDK types and client APIs from `unified_llm`.

## Validation

- `uv run pytest -q` passed with 453 passed and 10 skipped.
- The test suite now imports the agent API from `agent`, checks submodule imports such as `agent.history`, expects logging under `agent.session`, and verifies that importing `unified_llm.agent` raises `ModuleNotFoundError`.

## Shipped Changes

- Moved the agent implementation and provider profiles from `src/unified_llm/agent/` to `src/agent/`.
- Updated agent implementation imports so SDK dependencies resolve through absolute `unified_llm.*` imports and intra-agent dependencies remain package-relative.
- Updated README bootstrap documentation and coding-agent specification artifacts to name `agent` and `src/agent/` as the canonical public surface and topology.
- Rewrote the agent tests to use the top-level `agent` import surface while keeping tests under `tests/agent/`.
