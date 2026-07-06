---
id: CR-2026-0071-align-run-transcript-persistence-with-canonical-render-state
title: Align Run Transcript Persistence With Canonical Render State
status: completed
type: bugfix
changelog: internal
---

## Summary

Run transcript rendering now uses a run-local durable transcript state instead of reconstructing transcript rows from journal payload internals. The run journal remains scoped to operational history, while selected-run transcript inspection has a dedicated `/pipelines/{id}/transcript` API and frontend hydration path.

## Validation

- `cargo fmt --all -- --check` passed.
- `cargo test --workspace --all-features` passed.
- `npm --prefix frontend run test:unit` passed, with existing stderr warnings from frontend tests.
- `npm --prefix frontend run build` passed, with the existing Vite chunk-size warning.

## Shipped Changes

- Documented the transcript contract in `specs/attractor-spec.md`, including the new endpoint, coalesced render-state semantics, transcript boundaries, and the operational-only role of `/journal`.
- Added runtime transcript persistence in `crates/attractor-runtime`, including `transcript.json`, segment upserts for renderable assistant/reasoning/plan/tool/input/notice content, human-gate answer updates, and run/node/stage boundary records keyed by source scope, child flow, node, stage, and attempt.
- Added API support in `crates/attractor-api` for `GET /pipelines/{id}/transcript`.
- Adjusted Codex and unified LLM runtime/adapter paths so high-volume raw adapter/session data is kept out of normal `events.jsonl` transcript rendering, with LLM request summary events retained for operational journal history.
- Updated Runs frontend data loading and rendering to hydrate canonical transcript entries from `/transcript`, keep older-history loading scoped to journal events, and render transcript rows through the new `RunTranscriptCard`.
- Expanded Rust and frontend contract tests around transcript persistence, API responses, live transcript refresh, input-answer updates, boundary rendering, and removal of legacy raw/journal-derived transcript rows.
