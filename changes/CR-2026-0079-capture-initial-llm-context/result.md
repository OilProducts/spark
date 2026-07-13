---
id: CR-2026-0079-capture-initial-llm-context
title: Capture Initial LLM Context
status: completed
type: feature
changelog: public
---

## Summary

Spark now records the first LLM context submitted for each agent node as `logs/<node>/initial-context.txt`. Normal run execution no longer writes `prompt.md`, while existing `prompt.md` artifacts remain listable when present. The Runs artifact browser can surface the new artifact and shows a Codex-specific note when the capture represents the observable `turn/start.input` text.

## Validation

Repository changes include contract and process-contract coverage for initial-context artifact creation, write-if-absent behavior across retries and resumed Codex turns, capture failure preventing provider execution, provider failure after capture leaving the artifact available, omission of new `prompt.md` files, artifact-list metadata, and the Runs UI Codex note. Frontend unit coverage was updated for the conditional note.

The full validation gate passed:

- `cargo fmt --all -- --check`
- `cargo test --workspace --all-features`
- `npm --prefix frontend run test:unit`
- `npm --prefix frontend run build`

## Shipped Changes

- Added `crates/spark-agent-adapter/src/initial_context.rs` and wired adapter/session/Codex app-server paths to persist first-request context before transport.
- Added `context_capture_kind` event and artifact metadata for `assembled_messages` and `codex_turn_input` captures.
- Updated runtime artifact listing to include `initial-context.txt` and derive capture metadata from recorded backend events.
- Stopped the outer runtime executor from writing `prompt.md` during normal node artifact creation while retaining the lower-level explicit prompt storage interface.
- Updated Runs API types/parsing and artifact preview UI to recognize capture metadata and show the Codex internal-instructions note only for `codex_turn_input`.
- Expanded Rust and frontend tests around capture content, failure behavior, retry/resume preservation, artifact listing, and UI rendering.
