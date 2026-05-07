---
id: CR-2026-0038-fix-spark-agent-command-surface-packaging
title: Fix Spark Agent Command Surface Packaging
status: completed
type: bugfix
changelog: public
---

## Summary
Packaged/runtime initialization now seeds the default flow catalog with the two core user-facing software-development flows marked `agent_requestable`. Existing catalog choices are preserved, and examples plus child/internal flows remain disabled unless explicitly enabled.

## Validation
Validated with the full suite:

```bash
uv run pytest -q
```

Result: `1980 passed, 26 skipped, 2 warnings`.

## Shipped Changes
- Added default catalog seeding in `src/spark/workspace/flow_catalog.py`.
- Wired catalog seeding into the shared `spark-server init` runtime initialization path in `src/spark/server_cli.py`, which also covers service install and packaged Docker startup paths that call init.
- Kept agent-surface API filtering semantics unchanged while ensuring seeded core flows appear for `surface=agent`.
- Added the optional `spark flow list --text` empty-state message in `src/spark/cli.py`; JSON output remains `[]`.
- Added CLI and workspace flow endpoint tests covering seeded defaults, preservation of existing disabled policy, agent-surface visibility, and text-mode empty output.
