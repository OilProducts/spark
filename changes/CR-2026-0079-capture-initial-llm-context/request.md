# Capture Initial LLM Context

## Summary

Add `logs/<node>/initial-context.txt` as the plain-text record of the first LLM context assembled for each agent node. Remove `prompt.md` from normal run execution and expose the new artifact in the existing Runs artifact browser.

## Implementation Changes

- Capture only the first model call for a node, including across retries and resumed runs; use write-if-absent behavior so later calls cannot overwrite it.
- For regular agent backends, concatenate every text-bearing message part in request order with no separators, labels, schema appendix, or trailing newline.
- For text-only and simulation backends, write the final expanded prompt unchanged.
- For Codex app-server, write the exact text placed in `turn/start.input`; Codex-managed system/developer instructions remain unavailable.
- Perform capture immediately before transport. If persistence fails, return an artifact error and do not send an unrecorded request.
- Stop both the adapter and outer executor from generating `prompt.md`. Retain the low-level `NodeArtifacts.prompt` storage interface for compatibility with explicit callers.
- Keep `response.md` and `status.json` behavior unchanged.

## Runs Interface

- Continue exposing `initial-context.txt` through the existing artifact listing and preview endpoints; preserve normal filename ordering and do not auto-open it.
- Add optional artifact-list metadata `context_capture_kind` with values `assembled_messages` or `codex_turn_input`, derived from the node’s recorded backend events.
- When previewing a `codex_turn_input` artifact, show a UI note that Codex may add internal instructions not observable by Spark. Keep the artifact contents themselves byte-clean.
- Historical runs remain unchanged; old `prompt.md` files continue to display when present.

## Test Plan

- Verify regular agent capture is the direct concatenation of system and user text, includes assembled project instructions, and has no labels, separator, or trailing newline.
- Verify text-only, simulation, and fake Codex app-server captures match their final submitted prompt text byte-for-byte.
- Verify tool schemas are not appended separately and existing rendered tool descriptions remain part of the system text.
- Verify retries, contract repair, and resume paths preserve the first capture.
- Verify failed capture prevents provider execution and provider failure after capture leaves the artifact available.
- Verify new runs do not generate `prompt.md`.
- Verify the artifact API reports capture kind and the Runs preview shows the Codex note only for `codex_turn_input`.
- Run the full repository validation gate: Rust formatting and workspace tests, frontend unit tests, and frontend build.

## Assumptions

- “Initial context” means the first model request for each agent node, not the evolving context after responses or tool calls.
- Only message text is included. Separately supplied tool-schema structures and provider chat-template tokens are intentionally excluded.
- Nodes that fail before reaching a model request do not receive an `initial-context.txt` artifact.
