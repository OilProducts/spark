# TODO

- [ ] Item 13.3-02 follow-up: keep medium-graph optimization behavior in production, but gate/hide profiling-debug UI readouts in normal production UX (expose via a developer/debug flag).
- [ ] Audit Spark Spawn against the original Attractor spec/API to identify true runtime/editor contract drift versus product-layer extensions.
- [ ] Define the flow trigger/automation system, including how users associate flows with triggers, what the first trigger should be, and whether trigger-driven repo mutation is ever allowed by default.
- [ ] Define the future work tracker/kanban system that approved execution cards feed into, including tracker data model, lifecycle, and agent pickup/assignment mechanics.
- [ ] Build the harness for agent-driven acceptance workflows under `tests/acceptance/agent-workflows`.
- [ ] Add first-class structured UI authoring for subgraphs and scoped `node[...]` / `edge[...]` defaults so these no longer require raw DOT editing.
