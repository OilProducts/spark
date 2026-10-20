---
type: agent_task
title: Prepare Workspace
runtime: {}
contracts:
  reads_context:
  - context.request.spec_path
  - context.request.spec_slug
  - context.request.scope_hint
  - context.request.constraints
  - context.request.target_paths
  - context.request.validation_command
  response: status_envelope
retry:
  max_retries: 1
execution:
  llm_model: gpt-6.1-sol
  reasoning_effort: xhigh
routes:
- to: extract_requirements
  extensions:
    loop_restart: false
---

Using context.request.spec_path, optional context.request.spec_slug, context.request.scope_hint, context.request.constraints, context.request.target_paths, and context.request.validation_command when present, create specs/ and .spark/spec-implementation/ at the repository root if they do not exist. Resolve a human-readable workspace slug as follows: if context.request.spec_slug is present, normalize it to lowercase kebab-case; otherwise derive it from the spec filename by lowercasing, replacing non-alphanumeric runs with '-', collapsing duplicate '-', and trimming leading/trailing '-'. Create the committed spec package directory at specs/<slug>/. Copy the source spec into specs/<slug>/source.md. Create the runtime workspace at .spark/spec-implementation/<slug>/. Inside that runtime workspace, create or refresh a spec symlink pointing to specs/<slug>/ so downstream nodes can read the committed package through .spark/spec-implementation/current/spec/. Create or refresh .spark/spec-implementation/current as a symlink to .spark/spec-implementation/<slug>/ so downstream nodes and tool commands can use a stable runtime alias while the slugged workspace remains the source of truth for bookkeeping. Write .spark/spec-implementation/<slug>/state.json as valid JSON with this shape: {"request":{"spec_path":"...","spec_slug":"...","scope_hint":"...","constraints":[...],"target_paths":[...],"validation_command":"..."},"spec":{"slug":"...","package_dir":"specs/<slug>","source_path":"specs/<slug>/source.md"},"runtime":{"path":".spark/spec-implementation/<slug>","current_alias":".spark/spec-implementation/current"},"current_milestone_id":null,"updated_at":"<timestamp>"}. If the repository has a .gitignore file, ensure it ignores .spark/ exactly once: append a minimal .spark/ entry only when missing, and remove duplicate .spark/ lines without disturbing unrelated rules. Respond with ONLY a JSON status envelope, do not include extra top-level keys such as details, and return outcome success only after the committed spec package, the runtime workspace, the spec symlink, the current alias, and the workspace files are written.
