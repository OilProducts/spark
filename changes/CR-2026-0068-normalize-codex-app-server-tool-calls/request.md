# Normalize Codex App-Server Tool Calls

## Summary
Fix future Codex app-server command/file tool calls so they persist in Spark’s existing renderable `tool_call` shape. Keep the scope backend-only: already-persisted raw segments, including existing `rapid-pond` history, will not be rewritten or rendered by this change unless repaired separately later.

## Key Changes
- Add a Rust equivalent of the old Python `_tool_call_from_item` normalization in the Codex app-server adapter.
- For Codex `commandExecution` items, emit `tool_call` payloads with:
  - `id` from item `id`, falling back to `itemId` where applicable;
  - `kind: "command_execution"`;
  - normalized status: `running`, `failed`, or `completed`;
  - `title: "Run command"`;
  - `command` from the existing command extraction helper;
  - `output` from `aggregatedOutput` or `aggregated_output`.
- For Codex `fileChange` items, emit:
  - `kind: "file_change"`;
  - `title: "Apply file changes"`;
  - normalized status;
  - `file_paths` from the existing file path extraction logic or an empty array.
- Apply the normalizer where `process_codex_app_server_message` currently forwards raw tool items for `item/started`, `item/completed`, command approval, and file-change approval.
- Preserve raw Codex details only as extra metadata if already useful, but do not let raw shape replace the normalized frontend contract.
- Leave `spark-workspace` segment persistence unchanged; it should persist whatever normalized `event.tool_call` it receives.

## Public Interfaces
- No API route changes.
- No frontend type or renderer changes.
- The persisted `segments[].tool_call` shape for future Codex app-server tool calls changes from raw Codex item shape to the existing Spark UI contract.
- Existing raw historical segments are intentionally not migrated.

## Test Plan
- Extend `codex_app_server_contracts.rs` to assert `item/started` and `item/completed` for `commandExecution` produce normalized `tool_call` payloads with `title`, `kind`, `command`, and `output`.
- Add coverage for `fileChange` normalization with `title: "Apply file changes"` and `file_paths`.
- Add or update a workspace conversation ingestion test to verify persisted Codex app-server tool segments contain frontend-renderable fields after normalization.
- Run the full validation gate:
  - `cargo fmt --all -- --check`
  - `cargo test --workspace --all-features`
  - `npm --prefix frontend run test:unit`
  - `npm --prefix frontend run build`

## Assumptions
- Backend-only means no compatibility fallback for old raw persisted conversations.
- `rapid-pond` is useful evidence for the bug, but this fix will only affect new Codex app-server turns.
- The frontend’s existing requirement that `tool_call.id` and `tool_call.title` be present remains correct.
