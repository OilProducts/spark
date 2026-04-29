# Remove Narrow Justfile Guard Targets

## Summary
Remove the two justfile targets that exist only because repository-hygiene tests force them: `dot-lint` and `parser-unsupported-grammar`. The underlying tests already live in the normal pytest suite, so CI and local full-suite runs will continue covering them through `uv run pytest -q`.

## Key Changes
- Delete the `dot-lint` and `parser-unsupported-grammar` recipes from `justfile`.
- Delete the repo-hygiene tests that assert those recipes and CI command strings exist.
- Update `.github/workflows/ci.yml` to remove the separate DOT lint and parser unsupported grammar steps, leaving the normal Python test suite as the source of coverage.
- Update README command listings to remove `just dot-lint` and any mention of `just parser-unsupported-grammar`.

## Test Plan
- Run `just --list` and confirm `dot-lint` and `parser-unsupported-grammar` are no longer listed.
- Run `uv run pytest -q tests/repo_hygiene/test_dot_format_lint.py`.
- Run `uv run pytest -q tests/dsl/test_parser.py -k unsupported_grammar_regression`.
- Run full suite: `uv run pytest -q`.

## Assumptions
- The intent is to simplify the public justfile surface, not to remove DOT lint or parser regression coverage.
- CI should rely on the full Python suite for these checks instead of naming special just targets.
