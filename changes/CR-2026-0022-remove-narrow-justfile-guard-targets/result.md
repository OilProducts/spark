---
id: CR-2026-0022-remove-narrow-justfile-guard-targets
title: Remove Narrow Justfile Guard Targets
status: completed
type: internal
changelog: internal
---

## Summary

Removed the narrow `dot-lint` and `parser-unsupported-grammar` Justfile recipes while keeping the underlying checks covered by the normal pytest suite. CI now relies on the full Python test suite instead of invoking separate guard-target steps for those two cases.

## Validation

- `just --list` passed and no longer lists `dot-lint` or `parser-unsupported-grammar`.
- `uv run pytest -q tests/repo_hygiene/test_dot_format_lint.py` passed with 7 tests.
- `uv run pytest -q tests/dsl/test_parser.py -k unsupported_grammar_regression` passed with 5 selected tests and 54 deselected.
- `uv run pytest -q` passed with 1727 tests and 26 skipped.

## Shipped Changes

- `justfile`: removed the `dot-lint` and `parser-unsupported-grammar` recipes.
- `.github/workflows/ci.yml`: removed the dedicated CI steps that called those Justfile recipes.
- `tests/repo_hygiene/test_dot_format_lint.py`: removed repository-hygiene assertions that required the narrow recipes and CI command strings to exist.
- `README.md`: removed the `just dot-lint` command-list entry.
