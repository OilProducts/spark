# Move Codergen Loop To Top-Level `agent` Package

## Summary
- Move the coding agent implementation from `src/unified_llm/agent/` to `src/agent/`.
- Make `agent` the only supported public import surface for the codergen loop.
- Remove the old `unified_llm.agent` import surface instead of adding a compatibility shim.
- Keep the agent library layered on the existing Unified LLM SDK by importing SDK types from `unified_llm`.

## Key Changes
- Update implementation package layout:
  - Move all current agent modules and `profiles/` under `src/agent/`.
  - Remove `src/unified_llm/agent/`.
  - Change internal SDK-bound imports such as `from ..client import Client` to absolute imports like `from unified_llm.client import Client`.
  - Keep intra-agent imports relative within `agent`, for example `from .types import SessionConfig`.

- Update public API and docs:
  - Change examples and documented import surface from `unified_llm.agent` to `agent`.
  - Update `README.md` bootstrap usage to import `Session`, profiles, and environments from `agent`.
  - Update logging expectations from `unified_llm.agent.*` to `agent.*`.

- Update specification artifacts:
  - Revise `specs/coding-agent/architecture.md` so the canonical topology and stable import surface are `src/agent/` and `agent`.
  - Revise `specs/coding-agent/contract-decisions.json` CD-001 to say the agent is a top-level package layered on `unified_llm`, not a subpackage inside it.
  - Revise `specs/coding-agent/requirements.json` file references from `src/unified_llm/agent/...` to `src/agent/...`.
  - Preserve the source spec intent: library-first, no required CLI, direct use of `Client.complete()` / `Client.stream()`, no SDK `generate()` tool loop.

- Update tests:
  - Rewrite imports from `unified_llm.agent` to `agent`.
  - Rewrite submodule imports such as `unified_llm.agent.history` to `agent.history`.
  - Update log-capture assertions to expect `agent.session`.
  - Keep tests under `tests/agent/`; the test directory name still matches the package domain and does not define an import package.

## Test Plan
- Run targeted import and package-surface tests first:
  - `uv run pytest -q -x --maxfail=1 tests/agent/test_session.py`
  - `uv run pytest -q -x --maxfail=1 tests/agent/test_provider_profiles.py`
- Run the full suite before reporting completion:
  - `uv run pytest -q`
- If import failures are broad, triage with:
  - `uv run pytest -q -x --maxfail=1 tests/agent`

## Assumptions
- `agent` means the top-level Python import package `src/agent`, not a separate distribution or separate `pyproject.toml`.
- The existing project distribution name in `pyproject.toml` remains `unified_llm` for now; setuptools will still discover both `unified_llm` and `agent` under `src/`.
- No backward compatibility shim is added for `unified_llm.agent`; old imports should fail after the migration.
