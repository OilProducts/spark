# Readable Flow Formatter

## Summary
Add a readable traversal formatter for Spark DOT flows and make it the default persisted format for UI/API saves. Keep canonical formatting available only for semantic equivalence/signature use, and remove tests that require authored flows to match a canonical alphabetical DOT layout.

## Key Changes
- Add a new Python formatter API in `attractor.dsl.formatter`, e.g. `format_readable_dot(graph)`, with this layout:
  - `digraph ... {` first, then graph/default blocks.
  - Start node first, resolved the same way runtime/validator resolves starts: prefer the single `shape=Mdiamond` start, otherwise `start`/`Start`.
  - Traverse reachable nodes depth-first from start using existing outgoing edge order.
  - Emit a blank line before each node statement.
  - Immediately after each node, emit all outgoing edges from that node in existing edge order.
  - For loops/back-edges to already emitted nodes, emit the leaving edge in place but do not recurse again.
  - For multiple branches, visit targets in the same order as the outgoing edges appear.
  - Append any unreachable nodes afterward in original parse order, each with their outgoing edges.
- Preserve existing attribute serialization helpers, but do not sort nodes/edges globally in readable formatting. Attribute ordering may stay stable using the current known/preferred attr ordering where already established.
- Keep `format_dot()` / `canonicalize_dot()` or equivalent canonical helpers for semantic signatures, equality tests, and normalized internal comparisons only.
- Change `canonicalize_graph_source()` and API save flow persistence to use readable formatting, while leaving `semantic_signature()` on canonical/normalized output.
- Add `spark flow format`:
  - `spark flow format --file path.dot --text` prints readable DOT to stdout.
  - `spark flow format --file path.dot --write` rewrites the file.
  - `--text` is the default behavior when `--write` is absent.
  - Parse/validation errors should use the existing CLI error style and nonzero exits.

## Tests
- Replace `tests/dsl/test_formatter.py` exact canonical layout assertions with behavior-focused readable formatter tests:
  - start node appears before downstream nodes regardless of source declaration order.
  - each node is preceded by a blank line.
  - outgoing edges appear immediately after their source node.
  - branch order follows existing edge order.
  - loops emit the back-edge without duplicating nodes.
  - unreachable nodes are appended after reachable traversal.
- Keep or add canonical tests only for semantic equivalence/idempotence, not as an authored-file format contract.
- Update API save tests that currently compare saved content to `canonicalize_dot(...)`; assert the saved DOT parses, validates, preserves semantics, and follows readable ordering where user-visible formatting matters.
- Add CLI tests for `spark flow format --file` stdout and `--write`.
- Run targeted triage with `uv run pytest -q -x --maxfail=1 tests/dsl/test_formatter.py tests/api/test_flow_save_validation.py tests/test_cli.py`, then full suite with `uv run pytest -q`.

## Assumptions
- “Encountered order” means depth-first traversal from the start node, using existing outgoing edge order.
- “Decision points” preserve author/editor edge order, not label/condition/weight sorting.
- UI/API saves should persist readable traversal DOT rather than canonical alphabetical DOT.
- Canonical DOT remains allowed internally for equivalence checks, but tests should not enforce it as the saved/readable file shape.
