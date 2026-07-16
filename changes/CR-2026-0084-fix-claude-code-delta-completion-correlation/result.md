# Result

- Correlated Claude Code text and reasoning partials with completions through per-channel FIFO queues.
- Reset partial stream indices on `message_start` and reused the final text block identity for final-answer promotion.
- Updated the fake Claude Code CLI to model per-block assistant events, plus multi-block and no-partials scenarios.
- Added adapter and workspace contracts for identity reuse, FIFO ordering, no-partials behavior, and duplicate-free live segments.

Tool-use handling and durable-data migration were unchanged, as scoped.
