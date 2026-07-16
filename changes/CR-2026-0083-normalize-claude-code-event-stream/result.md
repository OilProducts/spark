# Result

Normalized Claude Code turns to the canonical stream contract: each invocation now has a stable UUID turn identity, partial text and thinking messages emit correlated deltas, and non-empty result text promotes the last assistant block to `final_answer` (or creates a result-only block).

Removed the obsolete workspace final-answer synthesis and anonymous assistant item-id compensation. Phase-less assistant content is no longer treated as final, while missing final answers still fail the turn.

Extended adapter, fake CLI, workspace, segment, and live SSE contracts for streaming identity, authoritative completions, in-place promotion (including plan-mode live delivery), result-only and empty results, chat/plan behavior, missing-final failure, and existing Codex behavior.

Validation: `just test` passes.
