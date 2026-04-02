## Test Execution Policy
- Always run tests with `uv run pytest`
- Before reporting completion of a code change, run the full suite with `uv run pytest -q` unless the user asks otherwise.
- For failure triage, prefer `uv run pytest -q -x --maxfail=1 <path-or-nodeid>` to get the first actionable error quickly.
- Write tests against observable behavior (UI interactions, state transitions, API responses), not exact source-code strings or snippet presence checks.

## UI Spec Guidance
- Figma answers: what should this look like?
- Spec answers: what should happen?
- Tests answer: does it actually do that?
