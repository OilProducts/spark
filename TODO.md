# TODO

- [x] Normalize checkpoint semantics so `current_node` has one stable meaning in both persistence and resume logic, and update the Attractor spec accordingly.
- [x] Mirror the documented `graph.*` context namespace in the engine itself instead of only seeding it through the API bootstrap path.
- [ ] Complete `wait.human` timeout/default wiring by setting `Question.default` / `Question.timeout_seconds` where the interviewer contract expects them, or narrow the Attractor spec to the implemented behavior.
- [ ] Complete `stack.manager_loop` telemetry ingestion beyond the current runtime context snapshot.
- [x] Narrow the `stack.manager_loop` automatic steering contract: observe progress telemetry without treating it as a stall heuristic, auto-steer only from child failure context, and cap repeated automatic steering for the same child run, target node, and failure reason within one manager invocation.
- [ ] Expose the full implemented manager-loop authoring surface in the UI and preview payload, including at least `manager.steer_cooldown` and `stack.child_autostart`, which the runtime already honors but the current editor does not treat as first-class fields.
- [x] Document the full manager-loop node attribute surface in the Attractor spec appendix, including `manager.poll_interval`, `manager.max_cycles`, `manager.stop_condition`, `manager.actions`, `manager.steer_cooldown`, and `stack.child_autostart`.
- [ ] Decide whether the validator rule that every non-exit node must have an outgoing edge is intended Attractor behavior; if so, document it, otherwise relax it.
- [ ] Document the implemented `retry_policy` node attribute and named preset semantics as part of the Attractor DSL, or remove/hide that attribute surface if it is meant to stay implementation-specific.
- [x] Document the parallel-handler DSL additions `join_k` and `join_quorum` in the Spark guide and Attractor spec.
- [ ] Define the flow trigger/automation system, including how users associate flows with triggers, what the first trigger should be, and whether trigger-driven repo mutation is ever allowed by default.
- [ ] Build the harness for agent-driven acceptance workflows under `tests/acceptance/agent-workflows`.
- [ ] Add first-class structured UI authoring for subgraphs and scoped `node[...]` / `edge[...]` defaults so these no longer require raw DOT editing.
- [ ] Audit Spark against the original Attractor spec/API to identify any remaining runtime/editor contract drift versus intentional product-layer extensions after the known drift candidates above are handled.

## Feature candidates

- [ ] Add configurable workspace navigation so specialized project views can be enabled without crowding the default Spark tools.
