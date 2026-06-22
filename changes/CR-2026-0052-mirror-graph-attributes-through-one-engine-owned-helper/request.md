# Mirror Graph Attributes Through One Engine-Owned Helper

## Summary
Make the `graph.*` context namespace consistent by using one shared graph-attribute seeding helper for both API-launched runs and direct `PipelineExecutor` runs. The current API path mirrors all graph attributes, while the engine only mirrors `graph.goal` and `graph.default_max_retries`; the fix is to move Spark to the Metalshop-style cleaner model where the engine calls the shared helper.

## Key Changes
- Add or reuse a shared `graph_attr_context_seed(graph)` helper from `attractor.graph_prep` as the single source of truth for graph context initialization.
- Update `PipelineExecutor._mirror_graph_attrs(...)` to call `context.apply_updates(graph_attr_context_seed(self.graph))` instead of manually setting only selected keys.
- Keep existing semantics:
  - all graph attributes become `graph.<attr_name>` in context
  - `graph.goal` defaults to `""` when absent
  - `graph.default_max_retries` is normalized through `resolve_default_max_retries_value(...)`
  - host `launch_context` remains restricted to `context.*` and cannot override `graph.*`
- Leave API behavior unchanged except for routing it through the same shared helper if needed.
- Update `TODO.md` after the behavior is implemented: mark/remove the `graph.*` drift item.

## Test Plan
- Add a direct engine test proving `PipelineExecutor(...).run(Context())` mirrors arbitrary graph attrs, for example `default_fidelity`, `stack.child_dotfile`, or a custom graph attr, into `result.context["graph.<key>"]`.
- Keep or add coverage proving typed `graph.default_max_retries` remains an integer and missing `goal` still yields `graph.goal == ""`.
- Keep API checkpoint/context coverage proving API-launched runs still expose graph attrs.
- Run focused tests first:
  - `uv run pytest -q -x --maxfail=1 tests/engine/test_executor.py tests/api/test_pipeline_context_endpoint.py tests/api/test_backend_invariance.py`
- Before reporting completion, run:
  - `uv run pytest -q`

## Assumptions
- This is a hard cleanup, not a compatibility layer: direct engine execution should match API seeding.
- No public API schema changes are needed.
- The existing uncommitted `TODO.md` artifact cleanup should be committed separately or included only if the user explicitly wants housekeeping bundled with this implementation.
