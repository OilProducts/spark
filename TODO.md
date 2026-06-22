# TODO

- [ ] Complete `stack.manager_loop` telemetry ingestion beyond the current runtime context snapshot.
- [ ] Expose the full implemented manager-loop authoring surface in the UI and preview payload, including at least `manager.steer_cooldown` and `stack.child_autostart`, which the runtime already honors but the current editor does not treat as first-class fields.
- [ ] Decide whether the validator rule that every non-exit node must have an outgoing edge is intended Attractor behavior; if so, document it, otherwise relax it.
- [ ] Decide whether trigger-launched flows may mutate project repositories by default, and document/enforce the chosen policy.
- [ ] Build an executable harness for the existing agent-driven acceptance workflow assets under `tests/acceptance/agent-workflows`.
- [ ] Add first-class structured UI authoring for subgraphs and scoped `node[...]` / `edge[...]` defaults so these no longer require raw DOT editing.
- [ ] Audit Spark against the original Attractor spec/API to identify any remaining runtime/editor contract drift versus intentional product-layer extensions after the known drift candidates above are handled.

## Feature candidates

- [ ] Add configurable workspace navigation so specialized project views can be enabled without crowding the default Spark tools.
