---
id: CR-2026-0069-runs-view-as-workflow-transcript
title: Runs View as Workflow Transcript
status: completed
type: feature
changelog: public
---

## Summary

The selected run detail view now presents run activity as a workflow transcript instead of the prior separate pending-questions, progress, result, and raw event timeline stack. The transcript groups visible output by workflow node boundaries, reuses the project conversation markdown, tool-call, and request-user-input rendering paths, and keeps graph, checkpoint, context, artifacts, and result information under advanced details.

The run transcript projection is derived from the existing run journal. It coalesces visible `LLMContent` and nested `CodergenAdapter` content streams into transcript messages, prefers completed content when available, maps human gates into inline answer cards, renders tool-call events with the shared conversation tool-call UI, and filters raw adapter/journal payload details out of the normal transcript surface.

## Validation

Recorded validation for the shipped change:

- `npm --prefix frontend run test:unit -- timelineModel`
- `cargo fmt --all -- --check`
- `cargo test --workspace --all-features`
- `npm --prefix frontend run test:unit`
- `npm --prefix frontend run build`

The recorded unit-test run reported all 47 frontend test files and 357 tests passing. The build completed with the existing Vite large chunk warning, and the frontend tests emitted existing stderr warnings for React `act(...)`, mocked result requests, and schema-error logging.

## Shipped Changes

- Added run transcript entry types and projection logic in the runs timeline model for node boundaries, assistant/plan/reasoning messages, notices, tool calls, and request-user-input gates.
- Replaced the selected run detail surface with `RunTranscriptCard`, while keeping the compact summary/header area and moving run result visibility into advanced details with graph, checkpoint, context, and artifacts.
- Reused project conversation markdown, tool-call rows, and request-user-input cards for run transcript rendering, including support for option values needed by run human gates.
- Extended run timeline hydration to expose the transcript projection while preserving live updates, latest-page hydration, and load-older pagination.
- Normalized Codex app server tool-call payloads so frontend transcript rendering can show command execution and file-change tool calls without exposing raw adapter payloads.
- Updated frontend and Rust contract tests around run transcript rendering, journal projection, Codergen adapter content/tool-call normalization, live replay behavior, and raw event filtering.
