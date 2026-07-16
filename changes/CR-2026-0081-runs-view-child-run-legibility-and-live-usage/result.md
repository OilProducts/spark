# Result

Child runs now inherit their parent's project scope while retaining their isolated worktree, use authored flow and node names, and remain legible in the Runs sidebar and graph card. In-flight per-node codergen usage is projected latest-snapshot-first until completed request usage becomes authoritative, without changing completed totals.

The Runs UI uses compact wrapping graph controls with an explicit canvas-edge inset, wrapping child metadata, and no redundant Child chip. Worker flows now label every node.

Live usage record writes are throttled once per run across concurrent and sequential codergen nodes and merge only the three usage fields. Rust contracts exercise the default child-launch/storage path and streaming usage persistence, including an API-level parent-project runs filter, worktree retention, parsed titles, throttling, and concurrent status preservation.
