---
id: CR-2026-0070-remove-high-volume-adapter-events-from-run-journal
title: Remove High-Volume Adapter Events From Run Journal
status: completed
type: internal
changelog: internal
---

## Summary
Implemented durable run-local transcript persistence under each run root so renderable run output is stored separately from `events.jsonl`. New Codergen/unified LLM execution paths now keep `events.jsonl` focused on low-volume operational history and write transcript content through coalesced transcript entries instead of relying on replayed adapter/session deltas.

## Validation
- `cargo fmt --all -- --check`: passed
- `cargo test --workspace --all-features`: passed
- `npm --prefix frontend run test:unit`: passed
- `npm --prefix frontend run build`: passed, with the existing Vite large chunk-size warning

## Shipped Changes
- Replaced generic `CodergenAdapter` journal persistence with low-volume `LLMRequestStarted`, `LLMRequestCompleted`, and `LLMTokenUsage` runtime events.
- Added run-local transcript storage and APIs for loading completed run transcript entries from run state rather than raw journal adapter payloads.
- Persisted streaming/final assistant output, tool calls, request-user-input entries, notices, and node/run boundaries as renderable transcript entries.
- Routed raw Codex/app-session diagnostics to debug trace sidecars when debug tracing is enabled instead of normal journal rendering.
- Updated the Runs tab to render a transcript panel from transcript state, with legacy journal compatibility limited to workflow/operational rows rather than raw adapter transcript reconstruction.
- Added and updated Rust and frontend contract tests covering journal filtering, transcript persistence/hydration, debug trace routing, and Runs tab rendering.
