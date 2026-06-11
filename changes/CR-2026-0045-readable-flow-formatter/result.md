---
id: CR-2026-0045-readable-flow-formatter
title: Readable Flow Formatter
status: completed
type: feature
changelog: public
---

## Summary
Implemented a readable traversal-based DOT formatter for Spark flows and made it the persisted format used by flow saves. Canonical formatting remains available for semantic comparison and signatures.

## Validation
- `uv run pytest -q` passed with 2008 passed, 26 skipped, and 2 warnings.

## Shipped Changes
- Added `format_readable_dot()` and `canonicalize_readable_dot()` in `attractor.dsl.formatter`, exporting them through `attractor.dsl`.
- Updated `canonicalize_graph_source()` so UI/API flow saves persist readable DOT while semantic equivalence still uses canonical formatting.
- Added `spark flow format --file`, with stdout formatting by default and `--write` for in-place rewrites, including parse and validation error handling.
- Reworked formatter, API save, and CLI tests around observable readable formatting behavior instead of requiring authored flows to match canonical alphabetical layout.
