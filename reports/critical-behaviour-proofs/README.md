# Critical-behaviour proofs

Tested source: commit `b67f8cb570a07348a106019b50ea2af4c5d9c064`, tree `2f08140e068bc1b83cf8c822aa14559a28f4535b`. The breaks exist only in the diffs quoted here; none is in the code.

Every break ran in a throwaway `git archive` export of that commit, never in the real tree.

Raw evidence is kept in the gitignored `artifacts/critical-behaviour-proofs/` in this worktree, with a copy at `/tmp/spark-gate/critical-behaviour-proofs/`. Each `mutations/<id>/` holds:
- `mutation.diff`: the exact break
- `baseline.log`: complete stdout/stderr of the same command on the unbroken copy
- `mutated.log`: complete stdout/stderr with the break applied; line 1 is the command
- `result.json`: source identity, command, environment, exit statuses, and each test's parsed panic

To regenerate, from the repository root: `python3 artifacts/critical-behaviour-proofs/harness/prove.py <commit> [ids]`, then `python3 artifacts/critical-behaviour-proofs/harness/write_report.py <commit>`. The diff, command, exit status and assertion for each break are also quoted under "Detail per break" below.

Classification is computed by `harness/prove.py` from the log: BEHAVIOURAL_FAILURE means it compiled, stayed inside the harness timeout, every named test failed with a panic, and no other test failed. The alternatives are COMPILATION_ERROR, HARNESS_TIMEOUT, UNRELATED_FAILURE, TEST_DID_NOT_RUN, NOT_DETECTED and PARTLY_DETECTED.

"Own deadline" marks a failure raised by the test's own bounded wait: the behaviour never happened within the deadline. That is the test's assertion, not a harness timeout.

## 1. A result with no matching route fails the run

| id | break | test that failed | baseline | mutated | classification | failed at, and message |
|---|---|---|---|---|---|---|
| [B1-fresh](#b1-fresh) | unknown decision falls back to the first declared route | `routing_model_contracts::an_undeclared_decision_fails_the_run_with_the_declared_set` | passed, exit 0 | exit 101 | BEHAVIOURAL_FAILURE | `routing_model_contracts.rs:225:9` assertion `left == right` failed: Ready left: "completed" right: "failed" |
| [B1-resume](#b1-resume) | a stored completed visit with no route re-executes instead of failing | `flow_definition_typed_runtime_contracts::resume_of_an_undeclared_completed_visit_fails_without_reexecution` | passed, exit 0 | exit 101 | BEHAVIOURAL_FAILURE | `flow_definition_typed_runtime_contracts.rs:3309:9` ["evaluate"] |
| [B1-branch](#b1-branch) | a branch routing error is recorded as a tolerable runtime error | `routing_model_contracts::routing_errors_in_a_branch_are_fatal_even_when_tolerated` | passed, exit 0 | exit 101 | BEHAVIOURAL_FAILURE | `routing_model_contracts.rs:669:9` assertion `left == right` failed: approve left: "completed" right: "failed" |
| [B1-crash](#b1-crash) | a saved error record no longer stops the fork on resume | `routing_model_contracts::a_saved_routing_error_branch_is_fatal_on_resume` | passed, exit 0 | exit 101 | BEHAVIOURAL_FAILURE | `routing_model_contracts.rs:819:5` ["b"] |
| [B1-crash2](#b1-crash2) | a tolerant fork tolerates any saved error regardless of kind | `routing_model_contracts::a_saved_routing_error_branch_is_fatal_on_resume` | passed, exit 0 | exit 101 | BEHAVIOURAL_FAILURE | `routing_model_contracts.rs:813:5` assertion `left == right` failed left: "completed" right: "failed" |

## 2. A run never reports success unless it reached a success exit

| id | break | test that failed | baseline | mutated | classification | failed at, and message |
|---|---|---|---|---|---|---|
| [B2-exit](#b2-exit) | a failure exit reports outcome success | `routing_model_contracts::reaching_a_failure_exit_completes_the_run_with_its_declared_result` | passed, exit 0 | exit 101 | BEHAVIOURAL_FAILURE | `routing_model_contracts.rs:335:5` assertion `left == right` failed left: Some("success") right: Some("failure") |
| [B2-fail](#b2-fail) | a failed run is written as completed | `routing_model_contracts::an_undeclared_decision_fails_the_run_with_the_declared_set` | passed, exit 0 | exit 101 | BEHAVIOURAL_FAILURE | `routing_model_contracts.rs:227:9` assertion `left == right` failed: Ready left: "completed" right: "failed" |
| [B2-branch](#b2-branch) | a branch reaching a success exit is accepted before its join | `routing_model_contracts::an_unvalidated_success_exit_branch_cannot_end_the_owner_successfully` | passed, exit 0 | exit 101 | BEHAVIOURAL_FAILURE | `routing_model_contracts.rs:1780:5` assertion `left == right` failed left: "completed" right: "failed" |

## 3. Exhausting a visit limit fails the run and never takes a different decision

| id | break | test that failed | baseline | mutated | classification | failed at, and message |
|---|---|---|---|---|---|---|
| [B3-limit](#b3-limit) | the visit limit is never enforced | `routing_model_contracts::a_decision_selecting_an_exhausted_target_fails_without_another_route` | passed, exit 0 | exit 101 | BEHAVIOURAL_FAILURE | `routing_model_contracts.rs:258:5` assertion `left == right` failed left: "visit_limit_reached: implement entered 3 times, over its 2 visits" right: "visit_limit_reached: node \"review\ |
| [B3-other](#b3-other) | an exhausted target falls back to another available route | `routing_model_contracts::a_decision_selecting_an_exhausted_target_fails_without_another_route` | passed, exit 0 | exit 101 | BEHAVIOURAL_FAILURE | `routing_model_contracts.rs:256:5` assertion `left == right` failed left: "completed" right: "failed" |

## 4. External and sibling cancellation stop every tool and subflow child; independent errors are not cancellations

| id | break | test that failed | baseline | mutated | classification | failed at, and message |
|---|---|---|---|---|---|---|
| [B4-group](#b4-group) | stopping a tool kills only the shell, not its process group | `routing_model_contracts::canceling_a_branch_tool_stops_its_descendants_promptly` | passed, exit 0 | exit 101 | BEHAVIOURAL_FAILURE (own deadline) | `routing_model_contracts.rs:2071:5` 120.715866125s |
| [B4-group](#b4-group) | stopping a tool kills only the shell, not its process group | `routing_model_contracts::a_tool_timeout_stops_its_descendants` | passed, exit 0 | exit 101 | BEHAVIOURAL_FAILURE (own deadline) | `routing_model_contracts.rs:2094:5` 120.658197042s |
| [B4-external](#b4-external) | a branch tool ignores a persisted cancel request | `routing_model_contracts::external_cancellation_stops_branch_tools_that_never_return_on_their_own` | passed, exit 0 | exit 101 | BEHAVIOURAL_FAILURE | `routing_model_contracts.rs:1723:5` assertion failed: started.elapsed() < std::time::Duration::from_secs(20) |
| [B4-sibling](#b4-sibling) | a branch tool ignores its fork raising cancellation | `routing_model_contracts::a_tool_interrupted_by_its_fork_is_canceled_not_failed` | passed, exit 0 | exit 101 | BEHAVIOURAL_FAILURE | `routing_model_contracts.rs:1757:5` assertion `left == right` failed left: String("completed") right: String("canceled") |
| [B4-child](#b4-child) | a canceled fork never requests its child subflow cancel | `routing_model_contracts::a_blocked_sibling_cancels_a_waiting_child_and_settles_it` | passed, exit 0 | exit 101 | BEHAVIOURAL_FAILURE (own deadline) | `routing_model_contracts.rs:2363:10` the fork settled its waiting child: Timeout  |
| [B4-child](#b4-child) | a canceled fork never requests its child subflow cancel | `routing_model_contracts::a_fatal_sibling_cancels_a_waiting_child_and_fails_the_owner` | passed, exit 0 | exit 101 | BEHAVIOURAL_FAILURE (own deadline) | `routing_model_contracts.rs:2363:10` the fork settled its waiting child: Timeout |
| [B4-child](#b4-child) | a canceled fork never requests its child subflow cancel | `routing_model_contracts::external_cancellation_stops_a_subflow_child` | passed, exit 0 | exit 101 | BEHAVIOURAL_FAILURE (own deadline) | `routing_model_contracts.rs:2363:10` the fork settled its waiting child: Timeout |
| [B4-child](#b4-child) | a canceled fork never requests its child subflow cancel | `routing_model_contracts::external_cancellation_stops_a_subflow_child_in_a_fork_branch` | passed, exit 0 | exit 101 | BEHAVIOURAL_FAILURE (own deadline) | `routing_model_contracts.rs:2363:10` the fork settled its waiting child: Timeout  |
| [C-inline](#c-inline) | the launcher runs outside the cancel watcher | `routing_model_contracts::external_cancellation_stops_custom_launched_children` | passed, exit 0 | exit 101 | BEHAVIOURAL_FAILURE (own deadline) | `routing_model_contracts.rs:2363:10` the fork settled its waiting child: Timeout  |
| [C-inline](#c-inline) | the launcher runs outside the cancel watcher | `routing_model_contracts::sibling_cancellation_stops_custom_launched_children` | passed, exit 0 | exit 101 | BEHAVIOURAL_FAILURE (own deadline) | `routing_model_contracts.rs:2363:10` the fork settled its waiting child: Timeout |
| [C-forward](#c-forward) | an observed child is never asked to cancel | `routing_model_contracts::external_cancellation_stops_custom_launched_children` | passed, exit 0 | exit 101 | BEHAVIOURAL_FAILURE | `routing_model_contracts.rs:2665:9` assertion `left == right` failed: detached true left: "waiting" right: "canceled" |
| [C-forward](#c-forward) | an observed child is never asked to cancel | `routing_model_contracts::sibling_cancellation_stops_custom_launched_children` | passed, exit 0 | exit 101 | BEHAVIOURAL_FAILURE | `routing_model_contracts.rs:2688:9` assertion `left == right` failed: detached true left: "waiting" right: "canceled" |
| [C-settle](#c-settle) | the parent returns right after asking the child to cancel | `routing_model_contracts::external_cancellation_stops_custom_launched_children` | passed, exit 0 | exit 101 | BEHAVIOURAL_FAILURE | `routing_model_contracts.rs:2665:9` assertion `left == right` failed: detached true left: "cancel_requested" right: "canceled" |
| [C-settle](#c-settle) | the parent returns right after asking the child to cancel | `routing_model_contracts::sibling_cancellation_stops_custom_launched_children` | passed, exit 0 | exit 101 | BEHAVIOURAL_FAILURE | `routing_model_contracts.rs:2688:9` assertion `left == right` failed: detached true left: "cancel_requested" right: "canceled" |
| [I-watch](#i-watch) | seeing a stop in the child watcher marks the invocation interrupted again | `routing_model_contracts::an_independent_child_failure_after_sibling_cancellation_stays_an_error` | passed, exit 0 | exit 101 | BEHAVIOURAL_FAILURE | `routing_model_contracts.rs:2791:17` assertion `left == right` failed: detached false tolerate false Runtime left: "completed" right: "failed" |
| [I-loop](#i-loop) | seeing a stop in the observation loop marks the invocation interrupted again | `routing_model_contracts::an_independent_child_failure_after_sibling_cancellation_stays_an_error` | passed, exit 0 | exit 101 | BEHAVIOURAL_FAILURE | `routing_model_contracts.rs:2791:17` assertion `left == right` failed: detached true tolerate false Runtime left: "completed" right: "failed" |
| [I-canceled](#i-canceled) | a child settled canceled no longer marks the invocation interrupted | `routing_model_contracts::a_blocked_sibling_cancels_a_waiting_child_and_settles_it` | passed, exit 0 | exit 101 | BEHAVIOURAL_FAILURE | `routing_model_contracts.rs:2422:5` assertion `left == right` failed: branch sub (index 0) failed at sub: aborted_by_user left: "failed" right: "completed" |
| [B4-independent](#b4-independent) | any error after the fork raised cancel is recorded as canceled | `routing_model_contracts::an_independent_fatal_sibling_error_after_blocked_fails_the_owner` | passed, exit 0 | exit 101 | BEHAVIOURAL_FAILURE | `routing_model_contracts.rs:1455:13` assertion `left == right` failed: false routing left: String("canceled") right: String("error") |

## 5. A referenced attachment is never deleted

| id | break | test that failed | baseline | mutated | classification | failed at, and message |
|---|---|---|---|---|---|---|
| [B5-prune](#b5-prune) | prune selects attached uploads as well as drafts | `workspace_conversation_attachment_route_contracts::draft_removal_abandoned_cleanup_and_conversation_deletion_remove_uploads` | passed, exit 0 | exit 101 | BEHAVIOURAL_FAILURE | `workspace_conversation_attachment_route_contracts.rs:497:5` assertion `left == right` failed left: ["att-3f1c71d9dc377f0a865b679ea6d77bfb", "att-9eda45dd7f0cef2eb9d208c06b9b911f", "att-interrupted"] right: ["at |
| [B5-prune](#b5-prune) | prune selects attached uploads as well as drafts | `workspace_conversation_attachment_route_contracts::referenced_uploads_survive_deletion_and_cleanup_racing_a_send` | passed, exit 0 | exit 101 | BEHAVIOURAL_FAILURE | `workspace_conversation_attachment_route_contracts.rs:752:5` assertion failed: repository.prune_attachments("chat-f", PROJECT, past_window()).expect("prune").is_empty() |
| [B5-delete](#b5-delete) | explicit delete no longer refuses an attached upload | `workspace_conversation_attachment_route_contracts::attachments_upload_send_download_and_restore_into_history` | passed, exit 0 | exit 101 | BEHAVIOURAL_FAILURE | `workspace_conversation_attachment_route_contracts.rs:206:5` assertion `left == right` failed left: 200 right: 409 |
| [B5-delete](#b5-delete) | explicit delete no longer refuses an attached upload | `workspace_conversation_attachment_route_contracts::referenced_uploads_survive_deletion_and_cleanup_racing_a_send` | passed, exit 0 | exit 101 | BEHAVIOURAL_FAILURE | `workspace_conversation_attachment_route_contracts.rs:748:5` assertion `left == right` failed left: 200 right: 409 |
| [B5-recheck](#b5-recheck) | cleanup skips the live-reference recheck behind the barrier | `conversation_commit_contracts::a_directory_with_a_live_reference_survives_the_prune` | passed, exit 0 | exit 101 | BEHAVIOURAL_FAILURE | `conversation_commit_contracts.rs:1699:5` ["att-unreferenced", "att-referenced"] |

## 6. A resend with the same client_turn_id creates no second turn or delivery

| id | break | test that failed | baseline | mutated | classification | failed at, and message |
|---|---|---|---|---|---|---|
| [B6-dedupe](#b6-dedupe) | a resend no longer finds its original acceptance | `conversation_event_normalization_contracts::a_duplicate_send_returns_the_original_acceptance_and_starts_no_second_turn` | passed, exit 0 | exit 101 | BEHAVIOURAL_FAILURE | `conversation_event_normalization_contracts.rs:4982:9` assertion `left == right` failed: accepted left: None right: Some("turn-0ccb40f8f06e4ca1a30b01909b1aa3f3") |
| [B6-dedupe](#b6-dedupe) | a resend no longer finds its original acceptance | `native_live_turn_contracts::a_resent_mid_turn_message_is_delivered_once` | passed, exit 0 | exit 101 | BEHAVIOURAL_FAILURE | `native_live_turn_contracts.rs:209:18` called `Result::unwrap_err()` on an `Ok` value: (None, TurnAcceptance { revision: 8, user_turn_id: "turn-steer1", assistant_turn_id: Some("turn-ca9a81 |

## (a) A corrupt attachment row never causes deletion

| id | break | test that failed | baseline | mutated | classification | failed at, and message |
|---|---|---|---|---|---|---|
| [a-row](#a-row) | an unparsable attachment row is skipped instead of failing the load | `conversation_commit_contracts::a_corrupt_attachment_row_fails_the_prune_and_deletes_nothing` | passed, exit 0 | exit 101 | BEHAVIOURAL_FAILURE | `conversation_commit_contracts.rs:1648:10` a corrupt row fails the prune: ["att-7d830434445857dce3b6b198afd52439", "att-2a4a49a9b46830be2ed51fd047fffb29"] |

## (b) Received::Lagged through SSE reaches summary catch-up

| id | break | test that failed | baseline | mutated | classification | failed at, and message |
|---|---|---|---|---|---|---|
| [b-lag](#b-lag) | a lagged stream resyncs but does not catch up | `live_queue_tests::a_lagged_stream_catches_up_on_every_summary_change` | passed, exit 0 | exit 101 | BEHAVIOURAL_FAILURE | `lib.rs:1130:36` not an event: : keepalive |

## (c) Role summary change across a disconnect; chat opened on a summary cursor

| id | break | test that failed | baseline | mutated | classification | failed at, and message |
|---|---|---|---|---|---|---|
| [c-gap](#c-gap) | catch-up reports no placeholder changes | `workspace_live_sse_contracts::a_role_summary_change_during_a_disconnect_arrives_on_reconnect` | passed, exit 0 | exit 101 | BEHAVIOURAL_FAILURE | `workspace_live_sse_contracts.rs:2124:5` assertion `left == right` failed: {"cursor":null,"payload":{"from_revision":"1","generation":"fc711b73-6ce5-4b51-a9be-1ed4bf8ea46d","rows":[],"waterma |
| [c-open](#c-open) | opening a chat drops the summary scope from the connection | `workspace_live_sse_contracts::opening_a_chat_while_holding_a_summary_cursor_serves_both_scopes` | passed, exit 0 | exit 101 | BEHAVIOURAL_FAILURE (own deadline) | `workspace_live_sse_contracts.rs:3732:9` no live envelope |

## (d) A failed sync_trace publishes nothing

| id | break | test that failed | baseline | mutated | classification | failed at, and message |
|---|---|---|---|---|---|---|
| [d-sync](#d-sync) | a failed trace sync is ignored and the commit publishes | `conversation_commit_contracts::a_failed_trace_sync_publishes_nothing` | passed, exit 0 | exit 101 | BEHAVIOURAL_FAILURE | `conversation_commit_contracts.rs:1801:5` Ok(ConversationCommit { revision: 2, changes: ChangeBatch { from_revision: 1, watermark: 2, rows: [EntityRow { kind: Conversation, conversation_id: "c |

## (e) settings_schema_version is validated

| id | break | test that failed | baseline | mutated | classification | failed at, and message |
|---|---|---|---|---|---|---|
| [e-version](#e-version) | the version check is removed | `conversation_commit_contracts::a_conversation_with_an_unsupported_settings_schema_version_does_not_load` | passed, exit 0 | exit 101 | BEHAVIOURAL_FAILURE | `conversation_commit_contracts.rs:1741:14` an unsupported version fails: Some(Object {"chat_mode": String("chat"), "conversation_handle": String("mossy-prairie"), "conversation_id": String("con |

## (f) A saved routing-error branch record is fatal on resume (same breaks as item 1, crash rows)

| id | break | test that failed | baseline | mutated | classification | failed at, and message |
|---|---|---|---|---|---|---|
| [B1-crash](#b1-crash) | a saved error record no longer stops the fork on resume | `routing_model_contracts::a_saved_routing_error_branch_is_fatal_on_resume` | passed, exit 0 | exit 101 | BEHAVIOURAL_FAILURE | `routing_model_contracts.rs:819:5` ["b"] |
| [B1-crash2](#b1-crash2) | a tolerant fork tolerates any saved error regardless of kind | `routing_model_contracts::a_saved_routing_error_branch_is_fatal_on_resume` | passed, exit 0 | exit 101 | BEHAVIOURAL_FAILURE | `routing_model_contracts.rs:813:5` assertion `left == right` failed left: "completed" right: "failed" |

## Also proven: Part 1, Part 4, and the execution-lock product fix

| id | break | test that failed | baseline | mutated | classification | failed at, and message |
|---|---|---|---|---|---|---|
| [P1-help](#p1-help) | serve --help runs runtime initialization, which writes the bundled library, before printing usage | `server_product_shell_process::subcommand_help_writes_nothing` | passed, exit 0 | exit 101 | BEHAVIOURAL_FAILURE | `server_product_shell_process.rs:133:9` assertion `left == right` failed: ["serve", "--help"] wrote files left: ["/var/folders/wx/wh9np15d04ncb_k6wlcks2_r0000gn/T/.tmp980FPU/spark-home"] rig |
| [P4-never](#p4-never) | prepare_integration always says the target has not moved | `resource_contracts::merge_change_skips_validation_only_when_the_target_is_the_merge_base` | passed, exit 0 | exit 101 | BEHAVIOURAL_FAILURE | `resource_contracts.rs:923:9` assertion `left == right` failed left: String("false") right: "true" |
| [P4-always](#p4-always) | prepare_integration always says the target moved | `resource_contracts::merge_change_skips_validation_only_when_the_target_is_the_merge_base` | passed, exit 0 | exit 101 | BEHAVIOURAL_FAILURE | `resource_contracts.rs:923:9` assertion `left == right` failed left: String("true") right: "false" |
| [L-lock](#l-lock) | the executor keeps the launch copy of the lock | `routing_model_contracts::a_finished_run_keeps_the_lock_state_taken_after_launch` | passed, exit 0 | exit 101 | BEHAVIOURAL_FAILURE | `routing_model_contracts.rs:2151:5` assertion `left == right` failed left: "queued" right: "holding" |

## Totals

35 breaks. Not classified BEHAVIOURAL_FAILURE, or without a passing baseline: none. Compilation errors, unrelated failures and harness timeouts: none unless listed.

Notes on specific rows:
- B3-limit: with `select_transition`'s check removed, a second check in the executor still fails the run, so the test fails on the exact reason it asserts rather than on the run succeeding. B3-other is the row showing a different decision being taken.
- B2-fail: `run.result.status` stays `failed` under this break, so it is caught only by the persisted-record assertions added to `an_undeclared_decision…`.
- C-forward and C-settle: the inline launcher cases pass under these breaks because the watcher covers them; the message shows the detached case failing.
- Output-order fix: there is no deterministic break here. The stale read and the commit have no hook between them, so its evidence is the stress result in the table below.

`artifacts/critical-behaviour-proofs/copy-restored-check.txt` shows the throwaway copy equals the commit after the last break. `artifacts/critical-behaviour-proofs/run.txt` is the harness output.

## Root cause per listed test

Load results: `artifacts/critical-behaviour-proofs/root-causes/before-b7d98a8b/` and `artifacts/critical-behaviour-proofs/root-causes/after-fix/`, each with `summary.txt` and the complete output of every run in `runs/`. Same driver for both: 24 runs per test, 6 at once, 30 CPU burners, 2 disk writers.

| listed test | kind | root cause | fix | passed before | passed after | evidence |
|---|---|---|---|---|---|---|
| `server_product_shell_process` | Test limit | Readiness loop of 100 x 50 ms against a slower cold start under load, and a port released before the server bound it. | `--port 0`; the test waits for the printed address, up to 180 s. | 2/24 | 24/24 | `runs/` panics "did not become ready" |
| `a_tool_timeout_stops_its_descendants` | Test limit, no kill bug | A probe shows the process group stops within 30 ms and the run settles in 1-2 s under load; the 6 s limit timed the whole run. | Asserts the recorded descendant pid is gone and wrote no effect; 90 s settle limit against a 120 s sleep. | 2/24 | 24/24 | `root-causes/experiments/tool-stop-probe.log` |
| `canceling_a_branch_tool_stops_its_descendants_promptly` | Test limit, no kill bug | Same as above, across a fork. | Same as above; cancel fires once both pids exist. | 1/24 | 24/24 | `root-causes/experiments/tool-stop-probe.log` |
| `native_node_steer_cancel_and_timeout` | Test limit | The 10 s limit spanned two whole runs including setup and record writes. | Measures each stream from request start to client hang-up, per stop, still under the 10 s grace. | 0/24 | 24/24 | `runs/` panics at line 1187 |
| `all_math_catalog_flows_serialize_per_project_while_other_projects_run` | Test limit AND product bug | The holder's shell loop gave up after about 10 s, releasing the lock early. Separately the terminal write restored the launch copy of the lock, so a holder could show `queued` (2 of 24 before). | Longer release loop and 180 s wait in the test; `executor.rs` reloads the lock from disk. Test `a_finished_run_keeps_the_lock_state_taken_after_launch`. | 3/24 | 24/24 | `runs/` left "queued" and "released"; `mutations/L-lock/` |
| `continuations_resolve_the_effective_flows_lock_against_their_project` | Test limit | Same release loop, plus a 15 s wait, too tight under load. | Same test changes. | 17/24 | 24/24 | `runs/` panics "continuation and probe complete" |
| `chat_model_route_exposes_efforts_with_or_without_a_project` | Test assumption | The 15 s SSE keepalive landed where the test expected a specific event or silence once the test ran longer than 15 s. Rare: 1 of 24 in the first run, 0 of 24 in the rerun, so the after-rate alone proves little. | Helpers skip keepalives; a non-keepalive event still fails the silence checks. | 24/24 | 24/24 | `root-causes/earlier-reproductions/chat_model*.log` |
| `output_streamed_the_moment_a_message_is_accepted_follows_it_live_and_persisted` | Product bug AND test race | `begin_turn` rewrote the user segment from a stale read, resetting the order the turn had assigned: two segments at order 1 and duplicated text. Separately the harness signalled before recording the payload. Rare: 2 of 300 before; 0 of 24 in the rerun above. | `conversations.rs` leaves order unassigned so the commit keeps it; the harness records first and gates on the text segment. Then 480 of 480 on 24551897 (that run's summary was not kept; this commit changes nothing on that path). | 24/24 | 24/24 | `root-causes/experiments/output-streamed-order-collision.json`, `root-causes/earlier-reproductions/output_streamed*.log` |
| `codex_node_that_does_not_stop_is_terminated_on_cancel_and_timeout` | Test limit | 10 s interrupt grace, 1 s timeout and load against a 15 s wait. | 180 s wait; pid-gone check 60 s. Termination still asserted by pid. | 4/24 | 24/24 | `runs/` panics "condition timed out" |
| `start_turn_passes_codex_jsonrpc_trace_path_only_in_debug_mode` | Knock-on | Does not fail alone. Any test panicking while holding ENV_LOCK poisons it for later tests, shown by injecting a panic in `output_streamed…` at b7d98a8b. | Env locks recover from poison, so only the real failure reports. | 24/24 | 24/24 | `root-causes/experiments/poison-knock-on.diff` and `.log` |

Evidence paths in the table are under `artifacts/critical-behaviour-proofs/`.

Found outside the list:
- Seeing a stop marked a subflow invocation interrupted, so an independent child failure read as a cancellation and a blocked sibling could hide it (product bug, fixed by `stop_signaled` in `handlers.rs` and `manager_loop.rs`): `I-watch`, `I-loop`, `I-canceled` below.
- External cancel never reached a running subflow child, including custom-launched ones (product bug, fixed in `manager_loop.rs`): `B4-child`, `C-inline`, `C-forward`, `C-settle` below.
- `request_user_input_live_request…` raced turn completion (test race): `root-causes/earlier-reproductions/base-two-copies-copy2.log`.
- `thread_list_feed_flips_running…` hung forever when the running state coalesced away (test): `root-causes/earlier-reproductions/thread_list_feed_hang.sample.txt`.
- `local_environment_exec_command_captures_streams…` and `…_inherit_none…` failed once during a smoke-and-suite overlap (test limit): `with_options(.., 50, 50, ..)` clamps their `timeout_ms: Some(1_000)` to 50 ms, so a delayed shell was stopped before printing. It did not recur under the stress driver (24/24 before the fix too), so the failing log plus a deterministic demonstration are the evidence: `root-causes/earlier-reproductions/overlap-local-environment-failure.log`, `root-causes/experiments/local-env-clamp.diff` and `.log` (a 0.2 s delay gives the same `left: ""`). After: `root-causes/after-local-env/summary.txt`.
- ui:smoke profile CRUD: the save took 8.4 s against a 5 s expect, because it re-checks every flow in every project (`validate_profile_candidate`): `root-causes/experiments/smoke-profile-save-timeline.txt`.

## Reported answers

- (c) ceiling: yes, a placeholder change in the gap followed by a restart stays stale in catch-up until the row's next change or a fresh snapshot, which is current. Pinned by `a_placeholder_change_before_a_restart_is_missing_from_catch_up_but_not_from_a_snapshot`; passing run in `root-causes/experiments/placeholder-ceiling.log`.
- (e): validated rather than removed, as the smaller change; the error names the conversation.

## Detail per break

### B1-fresh

Behaviour: No route fails: fresh

Break: unknown decision falls back to the first declared route

Source: commit `b67f8cb570a07348a106019b50ea2af4c5d9c064`, tree `2f08140e068bc1b83cf8c822aa14559a28f4535b`

Command: `cargo test --all-features -p attractor-runtime --test contracts -- --exact routing_model_contracts::an_undeclared_decision_fails_the_run_with_the_declared_set`

Baseline exit 0: an_undeclared_decision_fails_the_run_with_the_declared_set passed

Mutated exit 101, classification BEHAVIOURAL_FAILURE. Raw log: `artifacts/critical-behaviour-proofs/mutations/B1-fresh/mutated.log`

```diff
diff --git a/a/crates/attractor-runtime/src/routing.rs b/b/crates/attractor-runtime/src/routing.rs
index 78c31a3..1aa4389 100644
--- a/a/crates/attractor-runtime/src/routing.rs
+++ b/b/crates/attractor-runtime/src/routing.rs
@@ -75,7 +75,7 @@ pub fn select_transition(
         Some(NodeConfig::Parallel { join, .. }) => (String::new(), join.clone()),
         _ => match node.routes.get(&outcome.decision) {
             Some(route) => (outcome.decision.clone(), route.to.clone()),
-            None => return Err(no_matching_route_reason(flow, node_id, &outcome.decision)),
+            None => match node.routes.iter().next() { Some((d, r)) => (d.clone(), r.to.clone()), None => return Err(no_matching_route_reason(flow, node_id, &outcome.decision)) },
         },
     };
     let used = visits.get(&target).copied().unwrap_or(0);
```

`routing_model_contracts::an_undeclared_decision_fails_the_run_with_the_declared_set` failed at `crates/attractor-runtime/tests/contracts/routing_model_contracts.rs:225:9`:

```text
assertion `left == right` failed: Ready
  left: "completed"
 right: "failed"
```

### B1-resume

Behaviour: No route fails: completed-visit resume

Break: a stored completed visit with no route re-executes instead of failing

Source: commit `b67f8cb570a07348a106019b50ea2af4c5d9c064`, tree `2f08140e068bc1b83cf8c822aa14559a28f4535b`

Command: `cargo test --all-features -p attractor-runtime --test contracts -- --exact flow_definition_typed_runtime_contracts::resume_of_an_undeclared_completed_visit_fails_without_reexecution`

Baseline exit 0: resume_of_an_undeclared_completed_visit_fails_without_reexecution passed

Mutated exit 101, classification BEHAVIOURAL_FAILURE. Raw log: `artifacts/critical-behaviour-proofs/mutations/B1-resume/mutated.log`

```diff
diff --git a/a/crates/attractor-runtime/src/executor.rs b/b/crates/attractor-runtime/src/executor.rs
index 402b834..2560d6f 100644
--- a/a/crates/attractor-runtime/src/executor.rs
+++ b/b/crates/attractor-runtime/src/executor.rs
@@ -340,7 +340,7 @@ where
                 // visit's completion leaves this one pending. A prepared retry
                 // names this exact invocation and still needs execution/recovery.
                 let completed = completed_visit(&context).filter(|completed| {
-                    completed.node == current_node
+                    flow.nodes[&current_node].routes.contains_key(&completed.outcome.decision) && completed.node == current_node
                         && Some(completed.visit) == visit_counts.get(&current_node).copied()
                 });
                 if let Some(completed) = completed.filter(|_| {
```

`flow_definition_typed_runtime_contracts::resume_of_an_undeclared_completed_visit_fails_without_reexecution` failed at `crates/attractor-runtime/tests/contracts/flow_definition_typed_runtime_contracts.rs:3309:9`:

```text
["evaluate"]
```

### B1-branch

Behaviour: No route fails: parallel branch

Break: a branch routing error is recorded as a tolerable runtime error

Source: commit `b67f8cb570a07348a106019b50ea2af4c5d9c064`, tree `2f08140e068bc1b83cf8c822aa14559a28f4535b`

Command: `cargo test --all-features -p attractor-runtime --test contracts -- --exact routing_model_contracts::routing_errors_in_a_branch_are_fatal_even_when_tolerated`

Baseline exit 0: routing_errors_in_a_branch_are_fatal_even_when_tolerated passed

Mutated exit 101, classification BEHAVIOURAL_FAILURE. Raw log: `artifacts/critical-behaviour-proofs/mutations/B1-branch/mutated.log`

```diff
diff --git a/a/crates/attractor-runtime/src/handlers.rs b/b/crates/attractor-runtime/src/handlers.rs
index 970ebda..a1109d5 100644
--- a/a/crates/attractor-runtime/src/handlers.rs
+++ b/b/crates/attractor-runtime/src/handlers.rs
@@ -2367,7 +2367,7 @@ impl RuntimeHandlerRunner {
                 Err(reason) => {
                     return record!(
                         "error",
-                        error_value(&current, FailureKind::Routing, &reason, 1)
+                        error_value(&current, FailureKind::Runtime, &reason, 1)
                     )
                 }
             };
```

`routing_model_contracts::routing_errors_in_a_branch_are_fatal_even_when_tolerated` failed at `crates/attractor-runtime/tests/contracts/routing_model_contracts.rs:669:9`:

```text
assertion `left == right` failed: approve
  left: "completed"
 right: "failed"
```

### B1-crash

Behaviour: No route fails: crash after branch status write; (f)

Break: a saved error record no longer stops the fork on resume

Source: commit `b67f8cb570a07348a106019b50ea2af4c5d9c064`, tree `2f08140e068bc1b83cf8c822aa14559a28f4535b`

Command: `cargo test --all-features -p attractor-runtime --test contracts -- --exact routing_model_contracts::a_saved_routing_error_branch_is_fatal_on_resume`

Baseline exit 0: a_saved_routing_error_branch_is_fatal_on_resume passed

Mutated exit 101, classification BEHAVIOURAL_FAILURE. Raw log: `artifacts/critical-behaviour-proofs/mutations/B1-crash/mutated.log`

```diff
diff --git a/a/crates/attractor-runtime/src/handlers.rs b/b/crates/attractor-runtime/src/handlers.rs
index 970ebda..739b6f7 100644
--- a/a/crates/attractor-runtime/src/handlers.rs
+++ b/b/crates/attractor-runtime/src/handlers.rs
@@ -3253,7 +3253,7 @@ fn record_tolerated(record: &Value, tolerate: bool) -> bool {
 fn record_stops_fork(record: &Value, tolerate: bool) -> bool {
     match record_status(record) {
         "terminal_result" => true,
-        "error" => !record_tolerated(record, tolerate),
+        "error" => false,
         _ => false,
     }
 }
```

`routing_model_contracts::a_saved_routing_error_branch_is_fatal_on_resume` failed at `crates/attractor-runtime/tests/contracts/routing_model_contracts.rs:819:5`:

```text
["b"]
```

### B1-crash2

Behaviour: No route fails: crash after branch status write; (f)

Break: a tolerant fork tolerates any saved error regardless of kind

Source: commit `b67f8cb570a07348a106019b50ea2af4c5d9c064`, tree `2f08140e068bc1b83cf8c822aa14559a28f4535b`

Command: `cargo test --all-features -p attractor-runtime --test contracts -- --exact routing_model_contracts::a_saved_routing_error_branch_is_fatal_on_resume`

Baseline exit 0: a_saved_routing_error_branch_is_fatal_on_resume passed

Mutated exit 101, classification BEHAVIOURAL_FAILURE. Raw log: `artifacts/critical-behaviour-proofs/mutations/B1-crash2/mutated.log`

```diff
diff --git a/a/crates/attractor-runtime/src/handlers.rs b/b/crates/attractor-runtime/src/handlers.rs
index 970ebda..dc0b4bf 100644
--- a/a/crates/attractor-runtime/src/handlers.rs
+++ b/b/crates/attractor-runtime/src/handlers.rs
@@ -3246,7 +3246,7 @@ fn record_status(record: &Value) -> &str {
 
 /// An execution error a tolerant fork keeps.
 fn record_tolerated(record: &Value, tolerate: bool) -> bool {
-    tolerate && record["error"]["tolerable"].as_bool() == Some(true)
+    tolerate
 }
 
 /// A settled branch that stops the fork launching more work.
```

`routing_model_contracts::a_saved_routing_error_branch_is_fatal_on_resume` failed at `crates/attractor-runtime/tests/contracts/routing_model_contracts.rs:813:5`:

```text
assertion `left == right` failed
  left: "completed"
 right: "failed"
```

### B2-exit

Behaviour: Success only at a success exit

Break: a failure exit reports outcome success

Source: commit `b67f8cb570a07348a106019b50ea2af4c5d9c064`, tree `2f08140e068bc1b83cf8c822aa14559a28f4535b`

Command: `cargo test --all-features -p attractor-runtime --test contracts -- --exact routing_model_contracts::reaching_a_failure_exit_completes_the_run_with_its_declared_result`

Baseline exit 0: reaching_a_failure_exit_completes_the_run_with_its_declared_result passed

Mutated exit 101, classification BEHAVIOURAL_FAILURE. Raw log: `artifacts/critical-behaviour-proofs/mutations/B2-exit/mutated.log`

```diff
diff --git a/a/crates/attractor-runtime/src/executor.rs b/b/crates/attractor-runtime/src/executor.rs
index 402b834..89813cf 100644
--- a/a/crates/attractor-runtime/src/executor.rs
+++ b/b/crates/attractor-runtime/src/executor.rs
@@ -1558,7 +1558,8 @@ fn finalize_completed<E: NodeExecutor>(
         );
     }
     record.status = "completed".to_string();
-    record.outcome = Some(outcome);
+    let _ = outcome;
+    record.outcome = Some("success".to_string());
     record.outcome_reason_code = outcome_reason_code.clone();
     record.outcome_reason_message = outcome_reason_message.clone();
     record.ended_at = Some(crate::events::utc_timestamp());
```

`routing_model_contracts::reaching_a_failure_exit_completes_the_run_with_its_declared_result` failed at `crates/attractor-runtime/tests/contracts/routing_model_contracts.rs:335:5`:

```text
assertion `left == right` failed
  left: Some("success")
 right: Some("failure")
```

### B2-fail

Behaviour: Success only at a success exit

Break: a failed run is written as completed

Source: commit `b67f8cb570a07348a106019b50ea2af4c5d9c064`, tree `2f08140e068bc1b83cf8c822aa14559a28f4535b`

Command: `cargo test --all-features -p attractor-runtime --test contracts -- --exact routing_model_contracts::an_undeclared_decision_fails_the_run_with_the_declared_set`

Baseline exit 0: an_undeclared_decision_fails_the_run_with_the_declared_set passed

Mutated exit 101, classification BEHAVIOURAL_FAILURE. Raw log: `artifacts/critical-behaviour-proofs/mutations/B2-fail/mutated.log`

```diff
diff --git a/a/crates/attractor-runtime/src/executor.rs b/b/crates/attractor-runtime/src/executor.rs
index 402b834..1a255d7 100644
--- a/a/crates/attractor-runtime/src/executor.rs
+++ b/b/crates/attractor-runtime/src/executor.rs
@@ -1707,8 +1707,8 @@ fn finalize_failed_unless_canceled<E: NodeExecutor>(
             "aborted_by_user",
         );
     }
-    record.status = "failed".to_string();
-    record.outcome = None;
+    record.status = "completed".to_string();
+    record.outcome = Some("success".to_string());
     record.outcome_reason_code = None;
     record.outcome_reason_message = None;
     record.ended_at = Some(crate::events::utc_timestamp());
```

`routing_model_contracts::an_undeclared_decision_fails_the_run_with_the_declared_set` failed at `crates/attractor-runtime/tests/contracts/routing_model_contracts.rs:227:9`:

```text
assertion `left == right` failed: Ready
  left: "completed"
 right: "failed"
```

### B2-branch

Behaviour: Success only at a success exit

Break: a branch reaching a success exit is accepted before its join

Source: commit `b67f8cb570a07348a106019b50ea2af4c5d9c064`, tree `2f08140e068bc1b83cf8c822aa14559a28f4535b`

Command: `cargo test --all-features -p attractor-runtime --test contracts -- --exact routing_model_contracts::an_unvalidated_success_exit_branch_cannot_end_the_owner_successfully`

Baseline exit 0: an_unvalidated_success_exit_branch_cannot_end_the_owner_successfully passed

Mutated exit 101, classification BEHAVIOURAL_FAILURE. Raw log: `artifacts/critical-behaviour-proofs/mutations/B2-branch/mutated.log`

```diff
diff --git a/a/crates/attractor-runtime/src/handlers.rs b/b/crates/attractor-runtime/src/handlers.rs
index 970ebda..c701ab9 100644
--- a/a/crates/attractor-runtime/src/handlers.rs
+++ b/b/crates/attractor-runtime/src/handlers.rs
@@ -2172,7 +2172,7 @@ impl RuntimeHandlerRunner {
                 // never end its owner successfully before the join.
                 if flow
                     .exit_result(&current)
-                    .is_some_and(|result| result.outcome == attractor_core::ExitOutcome::Success)
+                    .is_some_and(|_| false)
                 {
                     return record!(
                         "error",
```

`routing_model_contracts::an_unvalidated_success_exit_branch_cannot_end_the_owner_successfully` failed at `crates/attractor-runtime/tests/contracts/routing_model_contracts.rs:1780:5`:

```text
assertion `left == right` failed
  left: "completed"
 right: "failed"
```

### B3-limit

Behaviour: Visit limit fails

Break: the visit limit is never enforced

Source: commit `b67f8cb570a07348a106019b50ea2af4c5d9c064`, tree `2f08140e068bc1b83cf8c822aa14559a28f4535b`

Command: `cargo test --all-features -p attractor-runtime --test contracts -- --exact routing_model_contracts::a_decision_selecting_an_exhausted_target_fails_without_another_route`

Baseline exit 0: a_decision_selecting_an_exhausted_target_fails_without_another_route passed

Mutated exit 101, classification BEHAVIOURAL_FAILURE. Raw log: `artifacts/critical-behaviour-proofs/mutations/B3-limit/mutated.log`

```diff
diff --git a/a/crates/attractor-runtime/src/routing.rs b/b/crates/attractor-runtime/src/routing.rs
index 78c31a3..5c9d4f6 100644
--- a/a/crates/attractor-runtime/src/routing.rs
+++ b/b/crates/attractor-runtime/src/routing.rs
@@ -80,7 +80,7 @@ pub fn select_transition(
     };
     let used = visits.get(&target).copied().unwrap_or(0);
     let limit = max_visits(flow, &target);
-    if used >= limit {
+    if false && used >= limit {
         return Err(format!(
             "{VISIT_LIMIT_REACHED}: node {node_id:?} decision {decision:?} selects {target:?}, which used {used} of its {limit} visits"
         ));
```

`routing_model_contracts::a_decision_selecting_an_exhausted_target_fails_without_another_route` failed at `crates/attractor-runtime/tests/contracts/routing_model_contracts.rs:258:5`:

```text
assertion `left == right` failed
  left: "visit_limit_reached: implement entered 3 times, over its 2 visits"
 right: "visit_limit_reached: node \"review\" decision \"fix\" selects \"implement\", which used 2 of its 2 visits"
```

### B3-other

Behaviour: Visit limit never takes another decision

Break: an exhausted target falls back to another available route

Source: commit `b67f8cb570a07348a106019b50ea2af4c5d9c064`, tree `2f08140e068bc1b83cf8c822aa14559a28f4535b`

Command: `cargo test --all-features -p attractor-runtime --test contracts -- --exact routing_model_contracts::a_decision_selecting_an_exhausted_target_fails_without_another_route`

Baseline exit 0: a_decision_selecting_an_exhausted_target_fails_without_another_route passed

Mutated exit 101, classification BEHAVIOURAL_FAILURE. Raw log: `artifacts/critical-behaviour-proofs/mutations/B3-other/mutated.log`

```diff
diff --git a/a/crates/attractor-runtime/src/routing.rs b/b/crates/attractor-runtime/src/routing.rs
index 78c31a3..d8e2441 100644
--- a/a/crates/attractor-runtime/src/routing.rs
+++ b/b/crates/attractor-runtime/src/routing.rs
@@ -81,6 +81,9 @@ pub fn select_transition(
     let used = visits.get(&target).copied().unwrap_or(0);
     let limit = max_visits(flow, &target);
     if used >= limit {
+        if let Some((d, r)) = node.routes.iter().find(|(_, r)| visits.get(&r.to).copied().unwrap_or(0) < max_visits(flow, &r.to)) {
+            return Ok(Transition { from: node_id.to_string(), decision: d.clone(), target: r.to.clone() });
+        }
         return Err(format!(
             "{VISIT_LIMIT_REACHED}: node {node_id:?} decision {decision:?} selects {target:?}, which used {used} of its {limit} visits"
         ));
```

`routing_model_contracts::a_decision_selecting_an_exhausted_target_fails_without_another_route` failed at `crates/attractor-runtime/tests/contracts/routing_model_contracts.rs:256:5`:

```text
assertion `left == right` failed
  left: "completed"
 right: "failed"
```

### B4-group

Behaviour: Cancellation stops every tool

Break: stopping a tool kills only the shell, not its process group

Source: commit `b67f8cb570a07348a106019b50ea2af4c5d9c064`, tree `2f08140e068bc1b83cf8c822aa14559a28f4535b`

Command: `cargo test --all-features -p attractor-runtime --test contracts -- --exact routing_model_contracts::canceling_a_branch_tool_stops_its_descendants_promptly routing_model_contracts::a_tool_timeout_stops_its_descendants`

Baseline exit 0: canceling_a_branch_tool_stops_its_descendants_promptly passed, a_tool_timeout_stops_its_descendants passed

Mutated exit 101, classification BEHAVIOURAL_FAILURE. Raw log: `artifacts/critical-behaviour-proofs/mutations/B4-group/mutated.log`

```diff
diff --git a/a/crates/attractor-runtime/src/handlers.rs b/b/crates/attractor-runtime/src/handlers.rs
index 970ebda..dd88195 100644
--- a/a/crates/attractor-runtime/src/handlers.rs
+++ b/b/crates/attractor-runtime/src/handlers.rs
@@ -2956,10 +2956,6 @@ fn run_shell_command(
         let timed_out = timeout.is_some_and(|timeout| started_at.elapsed() >= timeout);
         let interrupted = !timed_out && stop.as_mut().is_some_and(|stop| stop());
         if timed_out || interrupted {
-            spark_agent_adapter::local_environment::terminate_process_group(
-                child.id(),
-                Some(&mut child),
-            );
             let _ = child.kill();
             break (child.wait()?, timed_out, interrupted);
         }
```

`routing_model_contracts::canceling_a_branch_tool_stops_its_descendants_promptly` failed at `crates/attractor-runtime/tests/contracts/routing_model_contracts.rs:2071:5`:

```text
120.715866125s
```

`routing_model_contracts::a_tool_timeout_stops_its_descendants` failed at `crates/attractor-runtime/tests/contracts/routing_model_contracts.rs:2094:5`:

```text
120.658197042s
```

### B4-external

Behaviour: External cancellation stops tools

Break: a branch tool ignores a persisted cancel request

Source: commit `b67f8cb570a07348a106019b50ea2af4c5d9c064`, tree `2f08140e068bc1b83cf8c822aa14559a28f4535b`

Command: `cargo test --all-features -p attractor-runtime --test contracts -- --exact routing_model_contracts::external_cancellation_stops_branch_tools_that_never_return_on_their_own`

Baseline exit 0: external_cancellation_stops_branch_tools_that_never_return_on_their_own passed

Mutated exit 101, classification BEHAVIOURAL_FAILURE. Raw log: `artifacts/critical-behaviour-proofs/mutations/B4-external/mutated.log`

```diff
diff --git a/a/crates/attractor-runtime/src/handlers.rs b/b/crates/attractor-runtime/src/handlers.rs
index 970ebda..91ba5ca 100644
--- a/a/crates/attractor-runtime/src/handlers.rs
+++ b/b/crates/attractor-runtime/src/handlers.rs
@@ -1412,7 +1412,7 @@ impl RuntimeHandlerRunner {
                 return false;
             }
             last_check = Instant::now();
-            runtime.stop_requested()
+            false
         };
         let spawned = run_shell_command(
             &command,
```

`routing_model_contracts::external_cancellation_stops_branch_tools_that_never_return_on_their_own` failed at `crates/attractor-runtime/tests/contracts/routing_model_contracts.rs:1723:5`:

```text
assertion failed: started.elapsed() < std::time::Duration::from_secs(20)
```

### B4-sibling

Behaviour: Sibling cancellation stops tools

Break: a branch tool ignores its fork raising cancellation

Source: commit `b67f8cb570a07348a106019b50ea2af4c5d9c064`, tree `2f08140e068bc1b83cf8c822aa14559a28f4535b`

Command: `cargo test --all-features -p attractor-runtime --test contracts -- --exact routing_model_contracts::a_tool_interrupted_by_its_fork_is_canceled_not_failed`

Baseline exit 0: a_tool_interrupted_by_its_fork_is_canceled_not_failed passed

Mutated exit 101, classification BEHAVIOURAL_FAILURE. Raw log: `artifacts/critical-behaviour-proofs/mutations/B4-sibling/mutated.log`

```diff
diff --git a/a/crates/attractor-runtime/src/handlers.rs b/b/crates/attractor-runtime/src/handlers.rs
index 970ebda..3e279fd 100644
--- a/a/crates/attractor-runtime/src/handlers.rs
+++ b/b/crates/attractor-runtime/src/handlers.rs
@@ -379,9 +379,7 @@ impl HandlerRuntime {
     }
 
     fn canceled_by_owner(&self) -> bool {
-        self.cancel
-            .as_ref()
-            .is_some_and(|cancel| cancel.load(Ordering::SeqCst))
+        false
     }
 
     /// Whether the owning fork or the user asked this invocation to stop.
```

`routing_model_contracts::a_tool_interrupted_by_its_fork_is_canceled_not_failed` failed at `crates/attractor-runtime/tests/contracts/routing_model_contracts.rs:1757:5`:

```text
assertion `left == right` failed
  left: String("completed")
 right: String("canceled")
```

### B4-child

Behaviour: Cancellation stops subflow children

Break: a canceled fork never requests its child subflow cancel

Source: commit `b67f8cb570a07348a106019b50ea2af4c5d9c064`, tree `2f08140e068bc1b83cf8c822aa14559a28f4535b`

Command: `cargo test --all-features -p attractor-runtime --test contracts -- --exact routing_model_contracts::a_blocked_sibling_cancels_a_waiting_child_and_settles_it routing_model_contracts::a_fatal_sibling_cancels_a_waiting_child_and_fails_the_owner routing_model_contracts::external_cancellation_stops_a_subflow_child routing_model_contracts::external_cancellation_stops_a_subflow_child_in_a_fork_branch`

Baseline exit 0: a_blocked_sibling_cancels_a_waiting_child_and_settles_it passed, a_fatal_sibling_cancels_a_waiting_child_and_fails_the_owner passed, external_cancellation_stops_a_subflow_child passed, external_cancellation_stops_a_subflow_child_in_a_fork_branch passed

Mutated exit 101, classification BEHAVIOURAL_FAILURE. Raw log: `artifacts/critical-behaviour-proofs/mutations/B4-child/mutated.log`

```diff
diff --git a/a/crates/attractor-runtime/src/manager_loop.rs b/b/crates/attractor-runtime/src/manager_loop.rs
index c5e2c91..1e1f63a 100644
--- a/a/crates/attractor-runtime/src/manager_loop.rs
+++ b/b/crates/attractor-runtime/src/manager_loop.rs
@@ -667,7 +667,7 @@ fn watch_owner_cancel<T>(
         scope.spawn(|| {
             while !done.load(Ordering::SeqCst) {
                 // The child's record may not exist yet; retry until accepted.
-                if runtime.stop_signaled()
+                if false && runtime.stop_signaled()
                     && crate::controls::RuntimeControls::new(store.clone())
                         .request_cancel(child_run_id)
                         .is_ok()
```

`routing_model_contracts::a_blocked_sibling_cancels_a_waiting_child_and_settles_it` failed at `crates/attractor-runtime/tests/contracts/routing_model_contracts.rs:2363:10`:

```text
the fork settled its waiting child: Timeout
```

`routing_model_contracts::a_fatal_sibling_cancels_a_waiting_child_and_fails_the_owner` failed at `crates/attractor-runtime/tests/contracts/routing_model_contracts.rs:2363:10`:

```text
the fork settled its waiting child: Timeout
```

`routing_model_contracts::external_cancellation_stops_a_subflow_child` failed at `crates/attractor-runtime/tests/contracts/routing_model_contracts.rs:2363:10`:

```text
the fork settled its waiting child: Timeout
```

`routing_model_contracts::external_cancellation_stops_a_subflow_child_in_a_fork_branch` failed at `crates/attractor-runtime/tests/contracts/routing_model_contracts.rs:2363:10`:

```text
the fork settled its waiting child: Timeout
```

### C-inline

Behaviour: Custom-launched child: blocking launcher

Break: the launcher runs outside the cancel watcher

Source: commit `b67f8cb570a07348a106019b50ea2af4c5d9c064`, tree `2f08140e068bc1b83cf8c822aa14559a28f4535b`

Command: `cargo test --all-features -p attractor-runtime --test contracts -- --exact routing_model_contracts::external_cancellation_stops_custom_launched_children routing_model_contracts::sibling_cancellation_stops_custom_launched_children`

Baseline exit 0: external_cancellation_stops_custom_launched_children passed, sibling_cancellation_stops_custom_launched_children passed

Mutated exit 101, classification BEHAVIOURAL_FAILURE. Raw log: `artifacts/critical-behaviour-proofs/mutations/C-inline/mutated.log`

```diff
diff --git a/a/crates/attractor-runtime/src/manager_loop.rs b/b/crates/attractor-runtime/src/manager_loop.rs
index c5e2c91..7424105 100644
--- a/a/crates/attractor-runtime/src/manager_loop.rs
+++ b/b/crates/attractor-runtime/src/manager_loop.rs
@@ -515,7 +515,7 @@ fn autostart_child_pipeline(
         match runtime.run_paths.as_ref() {
             Some(paths) => {
                 let store = RunStore::for_runs_dir(paths.runs_dir.clone());
-                watch_owner_cancel(runtime, &store, &child_run_id, || launcher(request))
+                let _ = store; launcher(request)
             }
             None => launcher(request),
         }
```

`routing_model_contracts::external_cancellation_stops_custom_launched_children` failed at `crates/attractor-runtime/tests/contracts/routing_model_contracts.rs:2363:10`:

```text
the fork settled its waiting child: Timeout
```

`routing_model_contracts::sibling_cancellation_stops_custom_launched_children` failed at `crates/attractor-runtime/tests/contracts/routing_model_contracts.rs:2363:10`:

```text
the fork settled its waiting child: Timeout
```

### C-forward

Behaviour: Custom-launched child: observed child

Break: an observed child is never asked to cancel

Source: commit `b67f8cb570a07348a106019b50ea2af4c5d9c064`, tree `2f08140e068bc1b83cf8c822aa14559a28f4535b`

Command: `cargo test --all-features -p attractor-runtime --test contracts -- --exact routing_model_contracts::external_cancellation_stops_custom_launched_children routing_model_contracts::sibling_cancellation_stops_custom_launched_children`

Baseline exit 0: external_cancellation_stops_custom_launched_children passed, sibling_cancellation_stops_custom_launched_children passed

Mutated exit 101, classification BEHAVIOURAL_FAILURE. Raw log: `artifacts/critical-behaviour-proofs/mutations/C-forward/mutated.log`

```diff
diff --git a/a/crates/attractor-runtime/src/manager_loop.rs b/b/crates/attractor-runtime/src/manager_loop.rs
index c5e2c91..e5dc445 100644
--- a/a/crates/attractor-runtime/src/manager_loop.rs
+++ b/b/crates/attractor-runtime/src/manager_loop.rs
@@ -161,7 +161,7 @@ pub(crate) fn execute_manager_loop(
             observing = false;
             let stopping_since = *stopping_since.get_or_insert_with(Instant::now);
             if !cancel_forwarded {
-                cancel_forwarded = request_child_cancel(&runtime, &context);
+                cancel_forwarded = false && request_child_cancel(&runtime, &context);
             }
             // The next cycle reads the child's terminal status and resolves it.
             if stopping_since.elapsed() < CHILD_SETTLE_GRACE
```

`routing_model_contracts::external_cancellation_stops_custom_launched_children` failed at `crates/attractor-runtime/tests/contracts/routing_model_contracts.rs:2665:9`:

```text
assertion `left == right` failed: detached true
  left: "waiting"
 right: "canceled"
```

`routing_model_contracts::sibling_cancellation_stops_custom_launched_children` failed at `crates/attractor-runtime/tests/contracts/routing_model_contracts.rs:2688:9`:

```text
assertion `left == right` failed: detached true
  left: "waiting"
 right: "canceled"
```

### C-settle

Behaviour: Custom-launched child: settlement

Break: the parent returns right after asking the child to cancel

Source: commit `b67f8cb570a07348a106019b50ea2af4c5d9c064`, tree `2f08140e068bc1b83cf8c822aa14559a28f4535b`

Command: `cargo test --all-features -p attractor-runtime --test contracts -- --exact routing_model_contracts::external_cancellation_stops_custom_launched_children routing_model_contracts::sibling_cancellation_stops_custom_launched_children`

Baseline exit 0: external_cancellation_stops_custom_launched_children passed, sibling_cancellation_stops_custom_launched_children passed

Mutated exit 101, classification BEHAVIOURAL_FAILURE. Raw log: `artifacts/critical-behaviour-proofs/mutations/C-settle/mutated.log`

```diff
diff --git a/a/crates/attractor-runtime/src/manager_loop.rs b/b/crates/attractor-runtime/src/manager_loop.rs
index c5e2c91..a56b663 100644
--- a/a/crates/attractor-runtime/src/manager_loop.rs
+++ b/b/crates/attractor-runtime/src/manager_loop.rs
@@ -164,7 +164,7 @@ pub(crate) fn execute_manager_loop(
                 cancel_forwarded = request_child_cancel(&runtime, &context);
             }
             // The next cycle reads the child's terminal status and resolves it.
-            if stopping_since.elapsed() < CHILD_SETTLE_GRACE
+            if false && stopping_since.elapsed() < CHILD_SETTLE_GRACE
                 && !context_string(&context, "context.stack.child.run_id").is_empty()
             {
                 std::thread::sleep(Duration::from_millis(50));
```

`routing_model_contracts::external_cancellation_stops_custom_launched_children` failed at `crates/attractor-runtime/tests/contracts/routing_model_contracts.rs:2665:9`:

```text
assertion `left == right` failed: detached true
  left: "cancel_requested"
 right: "canceled"
```

`routing_model_contracts::sibling_cancellation_stops_custom_launched_children` failed at `crates/attractor-runtime/tests/contracts/routing_model_contracts.rs:2688:9`:

```text
assertion `left == right` failed: detached true
  left: "cancel_requested"
 right: "canceled"
```

### I-watch

Behaviour: Independent child errors: blocking launcher

Break: seeing a stop in the child watcher marks the invocation interrupted again

Source: commit `b67f8cb570a07348a106019b50ea2af4c5d9c064`, tree `2f08140e068bc1b83cf8c822aa14559a28f4535b`

Command: `cargo test --all-features -p attractor-runtime --test contracts -- --exact routing_model_contracts::an_independent_child_failure_after_sibling_cancellation_stays_an_error`

Baseline exit 0: an_independent_child_failure_after_sibling_cancellation_stays_an_error passed

Mutated exit 101, classification BEHAVIOURAL_FAILURE. Raw log: `artifacts/critical-behaviour-proofs/mutations/I-watch/mutated.log`

```diff
diff --git a/a/crates/attractor-runtime/src/manager_loop.rs b/b/crates/attractor-runtime/src/manager_loop.rs
index c5e2c91..a6635f0 100644
--- a/a/crates/attractor-runtime/src/manager_loop.rs
+++ b/b/crates/attractor-runtime/src/manager_loop.rs
@@ -667,7 +667,7 @@ fn watch_owner_cancel<T>(
         scope.spawn(|| {
             while !done.load(Ordering::SeqCst) {
                 // The child's record may not exist yet; retry until accepted.
-                if runtime.stop_signaled()
+                if runtime.stop_requested()
                     && crate::controls::RuntimeControls::new(store.clone())
                         .request_cancel(child_run_id)
                         .is_ok()
```

`routing_model_contracts::an_independent_child_failure_after_sibling_cancellation_stays_an_error` failed at `crates/attractor-runtime/tests/contracts/routing_model_contracts.rs:2791:17`:

```text
assertion `left == right` failed: detached false tolerate false Runtime
  left: "completed"
 right: "failed"
```

### I-loop

Behaviour: Independent child errors: observed child

Break: seeing a stop in the observation loop marks the invocation interrupted again

Source: commit `b67f8cb570a07348a106019b50ea2af4c5d9c064`, tree `2f08140e068bc1b83cf8c822aa14559a28f4535b`

Command: `cargo test --all-features -p attractor-runtime --test contracts -- --exact routing_model_contracts::an_independent_child_failure_after_sibling_cancellation_stays_an_error`

Baseline exit 0: an_independent_child_failure_after_sibling_cancellation_stays_an_error passed

Mutated exit 101, classification BEHAVIOURAL_FAILURE. Raw log: `artifacts/critical-behaviour-proofs/mutations/I-loop/mutated.log`

```diff
diff --git a/a/crates/attractor-runtime/src/manager_loop.rs b/b/crates/attractor-runtime/src/manager_loop.rs
index c5e2c91..617a692 100644
--- a/a/crates/attractor-runtime/src/manager_loop.rs
+++ b/b/crates/attractor-runtime/src/manager_loop.rs
@@ -157,7 +157,7 @@ pub(crate) fn execute_manager_loop(
                 &context,
             ));
         }
-        if runtime.stop_signaled() {
+        if runtime.stop_requested() {
             observing = false;
             let stopping_since = *stopping_since.get_or_insert_with(Instant::now);
             if !cancel_forwarded {
```

`routing_model_contracts::an_independent_child_failure_after_sibling_cancellation_stays_an_error` failed at `crates/attractor-runtime/tests/contracts/routing_model_contracts.rs:2791:17`:

```text
assertion `left == right` failed: detached true tolerate false Runtime
  left: "completed"
 right: "failed"
```

### I-canceled

Behaviour: Child canceled by its fork is a cancellation

Break: a child settled canceled no longer marks the invocation interrupted

Source: commit `b67f8cb570a07348a106019b50ea2af4c5d9c064`, tree `2f08140e068bc1b83cf8c822aa14559a28f4535b`

Command: `cargo test --all-features -p attractor-runtime --test contracts -- --exact routing_model_contracts::a_blocked_sibling_cancels_a_waiting_child_and_settles_it`

Baseline exit 0: a_blocked_sibling_cancels_a_waiting_child_and_settles_it passed

Mutated exit 101, classification BEHAVIOURAL_FAILURE. Raw log: `artifacts/critical-behaviour-proofs/mutations/I-canceled/mutated.log`

```diff
diff --git a/a/crates/attractor-runtime/src/manager_loop.rs b/b/crates/attractor-runtime/src/manager_loop.rs
index c5e2c91..a2f2f79 100644
--- a/a/crates/attractor-runtime/src/manager_loop.rs
+++ b/b/crates/attractor-runtime/src/manager_loop.rs
@@ -1442,11 +1442,6 @@ fn resolve_child_status(runtime: &HandlerRuntime, context: &ContextMap) -> Optio
     }
     // The child settled canceled. When this invocation asked for that, its
     // branch records cancellation rather than an error.
-    if runtime.stop_signaled() {
-        runtime
-            .interrupted
-            .store(true, std::sync::atomic::Ordering::SeqCst);
-    }
     Some(Outcome::error(
         FailureKind::Runtime,
         "aborted_by_user",
```

`routing_model_contracts::a_blocked_sibling_cancels_a_waiting_child_and_settles_it` failed at `crates/attractor-runtime/tests/contracts/routing_model_contracts.rs:2422:5`:

```text
assertion `left == right` failed: branch sub (index 0) failed at sub: aborted_by_user
  left: "failed"
 right: "completed"
```

### B4-independent

Behaviour: Independent errors are not cancellations

Break: any error after the fork raised cancel is recorded as canceled

Source: commit `b67f8cb570a07348a106019b50ea2af4c5d9c064`, tree `2f08140e068bc1b83cf8c822aa14559a28f4535b`

Command: `cargo test --all-features -p attractor-runtime --test contracts -- --exact routing_model_contracts::an_independent_fatal_sibling_error_after_blocked_fails_the_owner`

Baseline exit 0: an_independent_fatal_sibling_error_after_blocked_fails_the_owner passed

Mutated exit 101, classification BEHAVIOURAL_FAILURE. Raw log: `artifacts/critical-behaviour-proofs/mutations/B4-independent/mutated.log`

```diff
diff --git a/a/crates/attractor-runtime/src/handlers.rs b/b/crates/attractor-runtime/src/handlers.rs
index 970ebda..50bf307 100644
--- a/a/crates/attractor-runtime/src/handlers.rs
+++ b/b/crates/attractor-runtime/src/handlers.rs
@@ -2315,7 +2315,7 @@ impl RuntimeHandlerRunner {
                 // Any other error, of any kind, is the branch's own and keeps
                 // its kind, reason and precedence.
                 let kind = outcome.error_kind().unwrap_or(FailureKind::Runtime);
-                if interrupted.load(Ordering::SeqCst) {
+                if interrupted.load(Ordering::SeqCst) || cancel.load(Ordering::SeqCst) {
                     return record!(
                         "canceled",
                         json!({
```

`routing_model_contracts::an_independent_fatal_sibling_error_after_blocked_fails_the_owner` failed at `crates/attractor-runtime/tests/contracts/routing_model_contracts.rs:1455:13`:

```text
assertion `left == right` failed: false routing
  left: String("canceled")
 right: String("error")
```

### B5-prune

Behaviour: Referenced attachment never deleted

Break: prune selects attached uploads as well as drafts

Source: commit `b67f8cb570a07348a106019b50ea2af4c5d9c064`, tree `2f08140e068bc1b83cf8c822aa14559a28f4535b`

Command: `cargo test --all-features -p spark-http --test contracts -- --exact workspace_conversation_attachment_route_contracts::draft_removal_abandoned_cleanup_and_conversation_deletion_remove_uploads workspace_conversation_attachment_route_contracts::referenced_uploads_survive_deletion_and_cleanup_racing_a_send`

Baseline exit 0: draft_removal_abandoned_cleanup_and_conversation_deletion_remove_uploads passed, referenced_uploads_survive_deletion_and_cleanup_racing_a_send passed

Mutated exit 101, classification BEHAVIOURAL_FAILURE. Raw log: `artifacts/critical-behaviour-proofs/mutations/B5-prune/mutated.log`

```diff
diff --git a/a/crates/spark-storage/src/conversation/attachments.rs b/b/crates/spark-storage/src/conversation/attachments.rs
index 088ae97..ac99efe 100644
--- a/a/crates/spark-storage/src/conversation/attachments.rs
+++ b/b/crates/spark-storage/src/conversation/attachments.rs
@@ -307,8 +307,7 @@ impl ConversationRepository {
             conversation_id,
             project_path,
             |_, attachment| {
-                Ok(!attachment.attached
-                    && OffsetDateTime::parse(&attachment.record.created_at, &Rfc3339)
+                Ok(OffsetDateTime::parse(&attachment.record.created_at, &Rfc3339)
                         .is_ok_and(|created_at| created_at < cutoff))
             },
             Some(cutoff),
```

`workspace_conversation_attachment_route_contracts::draft_removal_abandoned_cleanup_and_conversation_deletion_remove_uploads` failed at `crates/spark-http/tests/contracts/workspace_conversation_attachment_route_contracts.rs:497:5`:

```text
assertion `left == right` failed
  left: ["att-3f1c71d9dc377f0a865b679ea6d77bfb", "att-9eda45dd7f0cef2eb9d208c06b9b911f", "att-interrupted"]
 right: ["att-9eda45dd7f0cef2eb9d208c06b9b911f", "att-interrupted"]
```

`workspace_conversation_attachment_route_contracts::referenced_uploads_survive_deletion_and_cleanup_racing_a_send` failed at `crates/spark-http/tests/contracts/workspace_conversation_attachment_route_contracts.rs:752:5`:

```text
assertion failed: repository.prune_attachments("chat-f", PROJECT,
            past_window()).expect("prune").is_empty()
```

### B5-delete

Behaviour: Referenced attachment never deleted

Break: explicit delete no longer refuses an attached upload

Source: commit `b67f8cb570a07348a106019b50ea2af4c5d9c064`, tree `2f08140e068bc1b83cf8c822aa14559a28f4535b`

Command: `cargo test --all-features -p spark-http --test contracts -- --exact workspace_conversation_attachment_route_contracts::attachments_upload_send_download_and_restore_into_history workspace_conversation_attachment_route_contracts::referenced_uploads_survive_deletion_and_cleanup_racing_a_send`

Baseline exit 0: attachments_upload_send_download_and_restore_into_history passed, referenced_uploads_survive_deletion_and_cleanup_racing_a_send passed

Mutated exit 101, classification BEHAVIOURAL_FAILURE. Raw log: `artifacts/critical-behaviour-proofs/mutations/B5-delete/mutated.log`

```diff
diff --git a/a/crates/spark-storage/src/conversation/attachments.rs b/b/crates/spark-storage/src/conversation/attachments.rs
index 088ae97..1b12956 100644
--- a/a/crates/spark-storage/src/conversation/attachments.rs
+++ b/b/crates/spark-storage/src/conversation/attachments.rs
@@ -283,7 +283,7 @@ impl ConversationRepository {
             if id != attachment_id {
                 return Ok(false);
             }
-            if attachment.attached {
+            if false && attachment.attached {
                 return Err(StorageError::ConversationCommitRejected {
                     conversation_id: conversation_id.to_string(),
                     reason: "This attachment belongs to a sent message and stays until the conversation is deleted.".to_string(),
```

`workspace_conversation_attachment_route_contracts::attachments_upload_send_download_and_restore_into_history` failed at `crates/spark-http/tests/contracts/workspace_conversation_attachment_route_contracts.rs:206:5`:

```text
assertion `left == right` failed
  left: 200
 right: 409
```

`workspace_conversation_attachment_route_contracts::referenced_uploads_survive_deletion_and_cleanup_racing_a_send` failed at `crates/spark-http/tests/contracts/workspace_conversation_attachment_route_contracts.rs:748:5`:

```text
assertion `left == right` failed
  left: 200
 right: 409
```

### B5-recheck

Behaviour: Referenced attachment never deleted

Break: cleanup skips the live-reference recheck behind the barrier

Source: commit `b67f8cb570a07348a106019b50ea2af4c5d9c064`, tree `2f08140e068bc1b83cf8c822aa14559a28f4535b`

Command: `cargo test --all-features -p spark-storage --test contracts -- --exact conversation_commit_contracts::a_directory_with_a_live_reference_survives_the_prune`

Baseline exit 0: a_directory_with_a_live_reference_survives_the_prune passed

Mutated exit 101, classification BEHAVIOURAL_FAILURE. Raw log: `artifacts/critical-behaviour-proofs/mutations/B5-recheck/mutated.log`

```diff
diff --git a/a/crates/spark-storage/src/conversation/attachments.rs b/b/crates/spark-storage/src/conversation/attachments.rs
index 088ae97..addd365 100644
--- a/a/crates/spark-storage/src/conversation/attachments.rs
+++ b/b/crates/spark-storage/src/conversation/attachments.rs
@@ -389,7 +389,8 @@ impl ConversationRepository {
             }
             let cleanup = store.database().after_checkpoint(|view| {
                 for candidate in &candidates {
-                    if !view.has_live_reference_under(&store.home_relative(candidate))? {
+                    {
+                        let _ = view;
                         remove_dir_if_exists(candidate, "delete attachment")?;
                     }
                 }
```

`conversation_commit_contracts::a_directory_with_a_live_reference_survives_the_prune` failed at `crates/spark-storage/tests/contracts/conversation_commit_contracts.rs:1699:5`:

```text
["att-unreferenced", "att-referenced"]
```

### B6-dedupe

Behaviour: Resend creates no second turn or delivery

Break: a resend no longer finds its original acceptance

Source: commit `b67f8cb570a07348a106019b50ea2af4c5d9c064`, tree `2f08140e068bc1b83cf8c822aa14559a28f4535b`

Command: `cargo test --all-features -p spark-workspace --test process_contracts -- --exact conversation_event_normalization_contracts::a_duplicate_send_returns_the_original_acceptance_and_starts_no_second_turn native_live_turn_contracts::a_resent_mid_turn_message_is_delivered_once`

Baseline exit 0: a_duplicate_send_returns_the_original_acceptance_and_starts_no_second_turn passed, a_resent_mid_turn_message_is_delivered_once passed

Mutated exit 101, classification BEHAVIOURAL_FAILURE. Raw log: `artifacts/critical-behaviour-proofs/mutations/B6-dedupe/mutated.log`

```diff
diff --git a/a/crates/spark-workspace/src/conversations.rs b/b/crates/spark-workspace/src/conversations.rs
index 45632ce..7598f3f 100644
--- a/a/crates/spark-workspace/src/conversations.rs
+++ b/b/crates/spark-workspace/src/conversations.rs
@@ -1493,11 +1493,7 @@ impl WorkspaceConversationService {
             // An accepted message is durable as its user turn or, accepted
             // into a running turn, as that turn's user-message segment. A
             // resend of the same request answers with its original acceptance.
-            if let Some(acceptance) =
-                self.existing_acceptance(conversation_id, client_turn_id, &request_sha256)?
-            {
-                return Ok((None, acceptance));
-            }
+            let _ = |c: &str, r: &str, s: &str| self.existing_acceptance(c, r, s);
         }
         let base_revision = snapshot_revision(&snapshot);
         prepare_snapshot_core_defaults(&mut snapshot, conversation_id, &project_path);
```

`conversation_event_normalization_contracts::a_duplicate_send_returns_the_original_acceptance_and_starts_no_second_turn` failed at `crates/spark-workspace/tests/process_contracts/conversation_event_normalization_contracts.rs:4982:9`:

```text
assertion `left == right` failed: accepted
  left: None
 right: Some("turn-0ccb40f8f06e4ca1a30b01909b1aa3f3")
```

`native_live_turn_contracts::a_resent_mid_turn_message_is_delivered_once` failed at `crates/spark-workspace/tests/process_contracts/native_live_turn_contracts.rs:209:18`:

```text
called `Result::unwrap_err()` on an `Ok` value: (None, TurnAcceptance { revision: 8, user_turn_id: "turn-steer1", assistant_turn_id: Some("turn-ca9a813fbc0943b3baff75f65e9c4ec9"), status: "accepted", changes: ChangeBatch { from_revision: 6, watermark: 8, rows: [EntityRow { kind: Conversation, conversation_id: "native-resend", id: "native-resend", collection: None, turn_id: None, position: None, order: None, revision: 8, deleted: false, payload: Some(Object {"created_at": String("2026-11-04T08:40:21Z"), "handle": String("polished-beacon"), "launched_run_ids": Array [], "metadata": Object {"chat_mode": String("chat"), "conversation_handle": String("polished-beacon"), "conversation_id": String(
```

### a-row

Behaviour: (a) Corrupt attachment row never causes deletion

Break: an unparsable attachment row is skipped instead of failing the load

Source: commit `b67f8cb570a07348a106019b50ea2af4c5d9c064`, tree `2f08140e068bc1b83cf8c822aa14559a28f4535b`

Command: `cargo test --all-features -p spark-storage --test contracts -- --exact conversation_commit_contracts::a_corrupt_attachment_row_fails_the_prune_and_deletes_nothing`

Baseline exit 0: a_corrupt_attachment_row_fails_the_prune_and_deletes_nothing passed

Mutated exit 101, classification BEHAVIOURAL_FAILURE. Raw log: `artifacts/critical-behaviour-proofs/mutations/a-row/mutated.log`

```diff
diff --git a/a/crates/spark-storage/src/conversation/store.rs b/b/crates/spark-storage/src/conversation/store.rs
index 838930a..d522059 100644
--- a/a/crates/spark-storage/src/conversation/store.rs
+++ b/b/crates/spark-storage/src/conversation/store.rs
@@ -666,8 +666,7 @@ fn loaded_from_rows(
         );
     }
     for (id, blob_path, status, payload) in stored.attachments {
-        let record: ConversationAttachment = serde_json::from_value(payload)
-            .map_err(|error| invalid(format!("Invalid attachment {id}: {error}")))?;
+        let Ok(record) = serde_json::from_value::<ConversationAttachment>(payload) else { continue };
         loaded.attachments.insert(
             id,
             StoredAttachment {
```

`conversation_commit_contracts::a_corrupt_attachment_row_fails_the_prune_and_deletes_nothing` failed at `crates/spark-storage/tests/contracts/conversation_commit_contracts.rs:1648:10`:

```text
a corrupt row fails the prune: ["att-7d830434445857dce3b6b198afd52439", "att-2a4a49a9b46830be2ed51fd047fffb29"]
```

### b-lag

Behaviour: (b) Lagged reaches summary catch-up

Break: a lagged stream resyncs but does not catch up

Source: commit `b67f8cb570a07348a106019b50ea2af4c5d9c064`, tree `2f08140e068bc1b83cf8c822aa14559a28f4535b`

Command: `cargo test --all-features -p spark-http --lib -- --exact live_queue_tests::a_lagged_stream_catches_up_on_every_summary_change`

Baseline exit 0: a_lagged_stream_catches_up_on_every_summary_change passed

Mutated exit 101, classification BEHAVIOURAL_FAILURE. Raw log: `artifacts/critical-behaviour-proofs/mutations/b-lag/mutated.log`

```diff
diff --git a/a/crates/spark-http/src/workspace.rs b/b/crates/spark-http/src/workspace.rs
index f5225d7..ff32081 100644
--- a/a/crates/spark-http/src/workspace.rs
+++ b/b/crates/spark-http/src/workspace.rs
@@ -1218,7 +1218,7 @@ async fn workspace_live_events(
                         for envelope in lagged_resync_envelopes(&query) {
                             yield Ok::<Bytes, Infallible>(sse_data_frame(&envelope));
                         }
-                        true
+                        false
                     }
                     Received::Changed => true,
                 },
```

`live_queue_tests::a_lagged_stream_catches_up_on_every_summary_change` failed at `crates/spark-http/src/lib.rs:1130:36`:

```text
not an event: : keepalive
```

### c-gap

Behaviour: (c) Role change during a disconnect

Break: catch-up reports no placeholder changes

Source: commit `b67f8cb570a07348a106019b50ea2af4c5d9c064`, tree `2f08140e068bc1b83cf8c822aa14559a28f4535b`

Command: `cargo test --all-features -p spark-http --test process_contracts -- --exact workspace_live_sse_contracts::a_role_summary_change_during_a_disconnect_arrives_on_reconnect`

Baseline exit 0: a_role_summary_change_during_a_disconnect_arrives_on_reconnect passed

Mutated exit 101, classification BEHAVIOURAL_FAILURE. Raw log: `artifacts/critical-behaviour-proofs/mutations/c-gap/mutated.log`

```diff
diff --git a/a/crates/spark-storage/src/conversation/store.rs b/b/crates/spark-storage/src/conversation/store.rs
index 838930a..f7a2583 100644
--- a/a/crates/spark-storage/src/conversation/store.rs
+++ b/b/crates/spark-storage/src/conversation/store.rs
@@ -282,7 +282,7 @@ impl ConversationStore {
             .lock()
             .unwrap_or_else(|poison| poison.into_inner())
             .iter()
-            .filter(|(_, (revision, _))| since < *revision && *revision <= watermark)
+            .filter(|(_, (revision, _))| false && since < *revision && *revision <= watermark)
             .map(|(id, (revision, project_path))| (id.clone(), project_path.clone(), *revision))
             .collect();
         changes.sort();
```

`workspace_live_sse_contracts::a_role_summary_change_during_a_disconnect_arrives_on_reconnect` failed at `crates/spark-http/tests/process_contracts/workspace_live_sse_contracts.rs:2124:5`:

```text
assertion `left == right` failed: {"cursor":null,"payload":{"from_revision":"1","generation":"fc711b73-6ce5-4b51-a9be-1ed4bf8ea46d","rows":[],"watermark":"2"},"project_path":"/private/var/folders/wx/wh9np15d04ncb_k6wlcks2_r0000gn/T/.tmpY9sJrd/project","resource":{"id":null,"kind":"conversations"},"type":"conversations.changes"}
  left: None
 right: Some("stopped")
```

### c-open

Behaviour: (c) Open a chat holding a summary cursor

Break: opening a chat drops the summary scope from the connection

Source: commit `b67f8cb570a07348a106019b50ea2af4c5d9c064`, tree `2f08140e068bc1b83cf8c822aa14559a28f4535b`

Command: `cargo test --all-features -p spark-http --test process_contracts -- --exact workspace_live_sse_contracts::opening_a_chat_while_holding_a_summary_cursor_serves_both_scopes`

Baseline exit 0: opening_a_chat_while_holding_a_summary_cursor_serves_both_scopes passed

Mutated exit 101, classification BEHAVIOURAL_FAILURE. Raw log: `artifacts/critical-behaviour-proofs/mutations/c-open/mutated.log`

```diff
diff --git a/a/crates/spark-http/src/workspace.rs b/b/crates/spark-http/src/workspace.rs
index f5225d7..5dd045e 100644
--- a/a/crates/spark-http/src/workspace.rs
+++ b/b/crates/spark-http/src/workspace.rs
@@ -1009,7 +1009,7 @@ fn initialize_live_scopes(
         envelopes.push(envelope);
         scopes.conversation = Some((conversation_id, project_path, cursor));
     }
-    if query.include_conversations {
+    if query.include_conversations && query.conversation_id.is_none() {
         let project_path = query.conversations_project_path.clone();
         let (envelope, cursor) = match &query.conversations_cursor {
             None => {
```

`workspace_live_sse_contracts::opening_a_chat_while_holding_a_summary_cursor_serves_both_scopes` failed at `crates/spark-http/tests/process_contracts/workspace_live_sse_contracts.rs:3732:9`:

```text
no live envelope
```

### d-sync

Behaviour: (d) Failed trace sync publishes nothing

Break: a failed trace sync is ignored and the commit publishes

Source: commit `b67f8cb570a07348a106019b50ea2af4c5d9c064`, tree `2f08140e068bc1b83cf8c822aa14559a28f4535b`

Command: `cargo test --all-features -p spark-storage --test contracts -- --exact conversation_commit_contracts::a_failed_trace_sync_publishes_nothing`

Baseline exit 0: a_failed_trace_sync_publishes_nothing passed

Mutated exit 101, classification BEHAVIOURAL_FAILURE. Raw log: `artifacts/critical-behaviour-proofs/mutations/d-sync/mutated.log`

```diff
diff --git a/a/crates/spark-storage/src/conversation/commit.rs b/b/crates/spark-storage/src/conversation/commit.rs
index 3c00be4..5da7223 100644
--- a/a/crates/spark-storage/src/conversation/commit.rs
+++ b/b/crates/spark-storage/src/conversation/commit.rs
@@ -454,7 +454,7 @@ fn write_changes(
     let allocated = loaded.next_trace_sequence.saturating_sub(1);
     if projected.is_some() {
         // The applied cursor may only cover durable trace records.
-        sync_trace(root)?;
+        let _ = sync_trace(root);
     }
     let conversation_row = summary_row(conversation_id, loaded)?;
     let (revision, rows) = store.database().write(|tx| {
```

`conversation_commit_contracts::a_failed_trace_sync_publishes_nothing` failed at `crates/spark-storage/tests/contracts/conversation_commit_contracts.rs:1801:5`:

```text
Ok(ConversationCommit { revision: 2, changes: ChangeBatch { from_revision: 1, watermark: 2, rows: [EntityRow { kind: Conversation, conversation_id: "conversation-trace-sync", id: "conversation-trace-sync", collection: None, turn_id: None, position: None, order: None, revision: 2, deleted: false, payload: Some(Object {"created_at": String("2026-11-04T08:44:32Z"), "handle": String("polished-lily"), "launched_run_ids": Array [], "metadata": Object {"chat_mode": String("chat"), "conversation_handle": String("polished-lily"), "conversation_id": String("conversation-trace-sync"), "created_at": String("2026-11-04T08:44:32Z"), "llm_profile": Null, "model": Null, "model_settings": Null, "project_path
```

### e-version

Behaviour: (e) settings_schema_version validated

Break: the version check is removed

Source: commit `b67f8cb570a07348a106019b50ea2af4c5d9c064`, tree `2f08140e068bc1b83cf8c822aa14559a28f4535b`

Command: `cargo test --all-features -p spark-storage --test contracts -- --exact conversation_commit_contracts::a_conversation_with_an_unsupported_settings_schema_version_does_not_load`

Baseline exit 0: a_conversation_with_an_unsupported_settings_schema_version_does_not_load passed

Mutated exit 101, classification BEHAVIOURAL_FAILURE. Raw log: `artifacts/critical-behaviour-proofs/mutations/e-version/mutated.log`

```diff
diff --git a/a/crates/spark-storage/src/conversation/store.rs b/b/crates/spark-storage/src/conversation/store.rs
index 838930a..f1db93a 100644
--- a/a/crates/spark-storage/src/conversation/store.rs
+++ b/b/crates/spark-storage/src/conversation/store.rs
@@ -624,7 +624,7 @@ fn loaded_from_rows(
     };
     let mut meta: ConversationMeta = serde_json::from_value(stored.metadata)
         .map_err(|error| invalid(format!("Invalid conversation metadata: {error}")))?;
-    if meta.settings_schema_version != 1 {
+    if false && meta.settings_schema_version != 1 {
         return Err(invalid(format!(
             "Conversation {conversation_id} has unsupported settings_schema_version {}",
             meta.settings_schema_version
```

`conversation_commit_contracts::a_conversation_with_an_unsupported_settings_schema_version_does_not_load` failed at `crates/spark-storage/tests/contracts/conversation_commit_contracts.rs:1741:14`:

```text
an unsupported version fails: Some(Object {"chat_mode": String("chat"), "conversation_handle": String("mossy-prairie"), "conversation_id": String("conversation-settings-version"), "created_at": String("2026-11-04T08:44:36Z"), "event_log": Array [], "flow_launches": Array [], "flow_run_requests": Array [], "llm_profile": Null, "model": Null, "model_settings": Null, "project_path": String("/projects/settings-version"), "proposed_plans": Array [], "provider": String("codex"), "reasoning_effort": Null, "revision": Number(1), "run_recoveries": Array [], "schema_version": Number(5), "segments": Array [], "settings_schema_version": Number(2), "title": String("Hello"), "turns": Array [Object {"conte
```

### P1-help

Behaviour: Part 1: help writes nothing

Break: serve --help runs runtime initialization, which writes the bundled library, before printing usage

Source: commit `b67f8cb570a07348a106019b50ea2af4c5d9c064`, tree `2f08140e068bc1b83cf8c822aa14559a28f4535b`

Command: `cargo test --all-features -p spark-server --test process_contracts -- --exact server_product_shell_process::subcommand_help_writes_nothing`

Baseline exit 0: subcommand_help_writes_nothing passed

Mutated exit 101, classification BEHAVIOURAL_FAILURE. Raw log: `artifacts/critical-behaviour-proofs/mutations/P1-help/mutated.log`

```diff
diff --git a/a/crates/spark-server/src/lib.rs b/b/crates/spark-server/src/lib.rs
index b3b6545..a53e370 100644
--- a/a/crates/spark-server/src/lib.rs
+++ b/b/crates/spark-server/src/lib.rs
@@ -209,7 +209,7 @@ fn run_server_shell(args: &[String], env: &impl Environment) -> CommandOutput {
     // Help never reaches a command's setup, which writes to the Spark home.
     if args[1..].iter().any(|arg| is_help_arg(arg)) {
         match args[0].as_str() {
-            "serve" => return CommandOutput::stdout(0, SERVE_HELP),
+            "serve" => { let _ = initialize_runtime_with_options(&RuntimeInitializationOptions { data_dir: None, ui_dir: None }, env); return CommandOutput::stdout(0, SERVE_HELP) }
             "init" => return CommandOutput::stdout(0, INIT_HELP),
             "service" => return CommandOutput::stdout(0, SERVICE_HELP),
             _ => {}
```

`server_product_shell_process::subcommand_help_writes_nothing` failed at `crates/spark-server/tests/process_contracts/server_product_shell_process.rs:133:9`:

```text
assertion `left == right` failed: ["serve", "--help"] wrote files
  left: ["/var/folders/wx/wh9np15d04ncb_k6wlcks2_r0000gn/T/.tmp980FPU/spark-home"]
 right: []
```

### P4-never

Behaviour: Part 4: validate when target moved

Break: prepare_integration always says the target has not moved

Source: commit `b67f8cb570a07348a106019b50ea2af4c5d9c064`, tree `2f08140e068bc1b83cf8c822aa14559a28f4535b`

Command: `cargo test --all-features -p spark-assets --test contracts -- --exact resource_contracts::merge_change_skips_validation_only_when_the_target_is_the_merge_base`

Baseline exit 0: merge_change_skips_validation_only_when_the_target_is_the_merge_base passed

Mutated exit 101, classification BEHAVIOURAL_FAILURE. Raw log: `artifacts/critical-behaviour-proofs/mutations/P4-never/mutated.log`

```diff
diff --git a/a/crates/spark-assets/assets/flows/software-development/merge-change/prepare_integration.md b/b/crates/spark-assets/assets/flows/software-development/merge-change/prepare_integration.md
index f273d21..3f3f0fd 100644
--- a/a/crates/spark-assets/assets/flows/software-development/merge-change/prepare_integration.md
+++ b/b/crates/spark-assets/assets/flows/software-development/merge-change/prepare_integration.md
@@ -47,6 +47,7 @@ target_moved=true
 if [ "$(git merge-base "$target_commit" "$source_commit" || true)" = "$target_commit" ]; then
   target_moved=false
 fi
+target_moved=false
 repo_root=$(git rev-parse --show-toplevel)
 # Worktrees live under <repo>/.spark/state, never inside .git: tooling
 # such as Vite refuses to serve or resolve paths matching **/.git/**. The
```

`resource_contracts::merge_change_skips_validation_only_when_the_target_is_the_merge_base` failed at `crates/spark-assets/tests/contracts/resource_contracts.rs:923:9`:

```text
assertion `left == right` failed
  left: String("false")
 right: "true"
```

### P4-always

Behaviour: Part 4: skip only when target is merge base

Break: prepare_integration always says the target moved

Source: commit `b67f8cb570a07348a106019b50ea2af4c5d9c064`, tree `2f08140e068bc1b83cf8c822aa14559a28f4535b`

Command: `cargo test --all-features -p spark-assets --test contracts -- --exact resource_contracts::merge_change_skips_validation_only_when_the_target_is_the_merge_base`

Baseline exit 0: merge_change_skips_validation_only_when_the_target_is_the_merge_base passed

Mutated exit 101, classification BEHAVIOURAL_FAILURE. Raw log: `artifacts/critical-behaviour-proofs/mutations/P4-always/mutated.log`

```diff
diff --git a/a/crates/spark-assets/assets/flows/software-development/merge-change/prepare_integration.md b/b/crates/spark-assets/assets/flows/software-development/merge-change/prepare_integration.md
index f273d21..8367ae2 100644
--- a/a/crates/spark-assets/assets/flows/software-development/merge-change/prepare_integration.md
+++ b/b/crates/spark-assets/assets/flows/software-development/merge-change/prepare_integration.md
@@ -45,7 +45,7 @@ source_commit=$(git rev-parse --verify "${SPARK_SOURCE_REF}^{commit}")
 # past the source branch's merge base.
 target_moved=true
 if [ "$(git merge-base "$target_commit" "$source_commit" || true)" = "$target_commit" ]; then
-  target_moved=false
+  target_moved=true
 fi
 repo_root=$(git rev-parse --show-toplevel)
 # Worktrees live under <repo>/.spark/state, never inside .git: tooling
```

`resource_contracts::merge_change_skips_validation_only_when_the_target_is_the_merge_base` failed at `crates/spark-assets/tests/contracts/resource_contracts.rs:923:9`:

```text
assertion `left == right` failed
  left: String("true")
 right: "false"
```

### L-lock

Behaviour: Product fix: finished run keeps its lock

Break: the executor keeps the launch copy of the lock

Source: commit `b67f8cb570a07348a106019b50ea2af4c5d9c064`, tree `2f08140e068bc1b83cf8c822aa14559a28f4535b`

Command: `cargo test --all-features -p attractor-runtime --test contracts -- --exact routing_model_contracts::a_finished_run_keeps_the_lock_state_taken_after_launch`

Baseline exit 0: a_finished_run_keeps_the_lock_state_taken_after_launch passed

Mutated exit 101, classification BEHAVIOURAL_FAILURE. Raw log: `artifacts/critical-behaviour-proofs/mutations/L-lock/mutated.log`

```diff
diff --git a/a/crates/attractor-runtime/src/executor.rs b/b/crates/attractor-runtime/src/executor.rs
index 402b834..8310b35 100644
--- a/a/crates/attractor-runtime/src/executor.rs
+++ b/b/crates/attractor-runtime/src/executor.rs
@@ -425,7 +425,7 @@ where
         // The lock was taken after this record was built, and terminal writes
         // persist this copy: it must not put a queued state back on disk.
         if let Some(disk) = store.read_run_record(&paths)? {
-            record.execution_lock = disk.execution_lock;
+            let _ = disk;
         }
         if !resumed {
             save_checkpoint_event(
```

`routing_model_contracts::a_finished_run_keeps_the_lock_state_taken_after_launch` failed at `crates/attractor-runtime/tests/contracts/routing_model_contracts.rs:2151:5`:

```text
assertion `left == right` failed
  left: "queued"
 right: "holding"
```
