# Result

Codex workflow agents can request clarification in default execution mode and continue the same node after all answers arrive. A callback keeps persistence and run status in the runtime and JSON-RPC responses in the adapter. Explicit text-only nodes and other providers retain their existing behavior.

Questions carry run, node execution, request, and question identities. Existing question/answer APIs persist accepted answers, reject duplicate and stale submissions, and include descendant runs. Process-held file locks make interrupted requests unanswerable without deleting history or restoring agent sessions. Cancel, pause, and subprocess exit end pending waits.

The run panel supports suggested choices, custom text, child-run routing, and journal history. The attention menu includes child-run clarification prompts. Codex guidance asks for unresolved intent after inspecting evidence; the evaluator includes the exact requested instruction.

Reconnect and durable resync now refresh pending-question snapshots, including descendant questions. Terminal run updates also reconcile snapshots so dead requests lose answer controls while journal history remains visible. Regression tests exercise dropped question/answer events, parent and child routing, both reconnect signals, and terminal cleanup.

The unrelated validation command, log-retention, routing, and associated test changes were reverted. The evaluator instruction remains exactly as requested.

Verification:

- Targeted reconnect/resync and stream race regressions: 21 tests passed, including broadcasting internally detected journal gaps.
- `cargo test -p attractor-api codex_human_wait_exceeds_inactivity_threshold -- --ignored --nocapture` passed in 312.34 seconds: a 301-second human wait completes the same node without retries.
- `just test` exited 0 on 2026-09-09 UTC: formatting, all workspace Rust tests with all features (including clarification, graph gates, conversation questions, and the 50,000-event conversation regression), all 451 frontend tests, and the production build passed.
- `git diff --check` passed.

Validation logs: `/tmp/spark-cr0108-review-just-test.log` and `/tmp/spark-cr0108-review-inactivity.log`.
