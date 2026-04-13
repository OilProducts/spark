# TODO

- [ ] Item 13.3-02 follow-up: keep medium-graph optimization behavior in production, but gate/hide profiling-debug UI readouts in normal production UX (expose via a developer/debug flag).
- [ ] Audit Spark against the original Attractor spec/API to identify true runtime/editor contract drift versus product-layer extensions.
- [ ] Align failure routing with the Attractor spec’s documented failure order so `FAIL` does not fall through ordinary unconditional edges before checking fail-specific routing.
- [ ] Normalize checkpoint semantics so `current_node` has one stable meaning in both persistence and resume logic, and update the Attractor spec accordingly.
- [ ] Mirror the documented `graph.*` context namespace in the engine itself instead of only seeding it through the API bootstrap path.
- [ ] Either wire the existing `ArtifactStore` into normal pipeline execution/runtime surfaces or narrow the Attractor spec so it does not promise a first-class artifact store that the engine does not use.
- [ ] Complete `wait.human` timeout/default wiring by setting `Question.default` / `Question.timeout_seconds` where the interviewer contract expects them, or narrow the Attractor spec to the implemented behavior.
- [ ] Finish or narrow the `stack.manager_loop` observe/steer contract. The current supervisory loop framework exists, but telemetry ingestion and steering behavior are still mostly stubs.
- [ ] Evaluate progress-aware manager steering for `stack.manager_loop` nodes. One possible direction: keep `manager.steer_cooldown` as a throttle, but only steer when child telemetry indicates stall/regression (for example unchanged active stage, repeated retries, repeated failures, or lack of new artifacts) and record the intervention reason.
- [ ] Expose the full implemented manager-loop authoring surface in the UI and preview payload, including at least `manager.steer_cooldown` and `stack.child_autostart`, which the runtime already honors but the current editor does not treat as first-class fields.
- [ ] Document the full manager-loop node attribute surface in the Attractor spec appendix, including `manager.poll_interval`, `manager.max_cycles`, `manager.stop_condition`, `manager.actions`, `manager.steer_cooldown`, and `stack.child_autostart`.
- [ ] Decide whether the validator rule that every non-exit node must have an outgoing edge is intended Attractor behavior; if so, document it, otherwise relax it.
- [ ] Document the implemented `retry_policy` node attribute and named preset semantics as part of the Attractor DSL, or remove/hide that attribute surface if it is meant to stay implementation-specific.
- [ ] Document or remove the parallel-handler DSL additions `join_k` and `join_quorum`, which are implemented but not covered in the current Attractor spec.
- [ ] Define the flow trigger/automation system, including how users associate flows with triggers, what the first trigger should be, and whether trigger-driven repo mutation is ever allowed by default.
- [ ] Build the harness for agent-driven acceptance workflows under `tests/acceptance/agent-workflows`.
- [ ] Add first-class structured UI authoring for subgraphs and scoped `node[...]` / `edge[...]` defaults so these no longer require raw DOT editing.
