# Fix Unified Backend Review Findings

## Summary
Address the review findings from the unified backend integration pass so pipeline execution, project chat, flow launch, and UI callers all use the same backend contracts without regressing legacy Codex app server behavior.

## Key Changes
- Add an explicit backend contract layer for Codex-generated outcomes, including structured status envelope parsing, contract violation reporting, write-contract repair prompts, and plain-text fallback behavior when no response contract is active.
- Route pipeline execution through a provider-aware backend that preserves model, provider, and reasoning-effort choices from launch context and node-level flow attributes.
- Preserve user-visible run semantics for retry and continue operations: retries reuse the failed run id and restart from the failed checkpoint, while continue creates a derived run with lineage metadata and the selected flow source mode.
- Carry runtime context into codergen and fan-in prompts through declared context reads, runtime carryover summaries, write-contract guidance, and stage-level raw RPC logging where supported.
- Update project chat so Codex app server turns can resume threads, stream command and token state, handle request-user-input prompts, start approved flow runs, persist artifacts, and expose conversation model metadata.
- Keep workspace flow launch APIs backend-neutral by omitting legacy backend defaults unless explicitly needed, while preserving provider and reasoning-effort fields when the caller sets them.
- Update frontend API and execution surfaces to pass the new model/provider/reasoning and artifact/chat metadata through observable UI-facing contracts.

## Acceptance Criteria
- Pipeline start, retry, continue, and run-list endpoints expose stable JSON behavior for model, provider, reasoning effort, lineage, status, and run id semantics.
- Codergen and fan-in handlers pass effective model/provider/reasoning options to their backend calls and include only the intended runtime context in prompts.
- Structured backend responses produce modeled outcomes, invalid contracted responses become contract failures or repair prompts, and uncontracted plain text remains accepted.
- Project chat conversations persist assistant responses, artifacts, request-user-input records, flow-run requests, token usage, and conversation model metadata through the public API.
- Workspace flow listing and pipeline start payloads remain backend-neutral for callers that do not explicitly select a provider, and preserve explicit provider/reasoning choices.
- Existing and new regression tests pass with `uv run pytest -q`.

## Validation Command
`uv run pytest -q`
