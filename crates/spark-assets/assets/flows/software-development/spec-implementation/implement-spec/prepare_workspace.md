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
  response: decision_envelope
retry:
  max_retries: 1
execution:
  llm_model: gpt-6.1-sol
  reasoning_effort: xhigh
routes:
  blocked:
    to: failed
    description: This node cannot complete its bounded task without unavailable input or authority and no honest local workaround or deferral remains; record the blocker and its evidence in notes and any required artifacts.
  done:
    to: extract_requirements
    description: The bounded work required by this node’s prompt is complete, including its required outputs; this does not assert that the whole workflow objective is achieved.
  unfinished:
    to: prepare_workspace
    description: Required work for this bounded node invocation remains and can be continued in the same session; no completed judgment is being asserted.
---

Using context.request.spec_path, optional context.request.spec_slug, context.request.scope_hint, context.request.constraints, context.request.target_paths, and context.request.validation_command when present, create specs/ and .spark/state/spec-implementation/ at the repository root if they do not exist. Resolve a human-readable workspace slug as follows: if context.request.spec_slug is present, normalize it to lowercase kebab-case; otherwise derive it from the spec filename by lowercasing, replacing non-alphanumeric runs with '-', collapsing duplicate '-', and trimming leading/trailing '-'. Create the committed spec package directory at specs/<slug>/. Copy the source spec into specs/<slug>/source.md. Create the runtime workspace at .spark/state/spec-implementation/<slug>/. Inside that runtime workspace, create or refresh a spec symlink pointing to specs/<slug>/ so downstream nodes can read the committed package through .spark/state/spec-implementation/current/spec/. Create or refresh .spark/state/spec-implementation/current as a symlink to .spark/state/spec-implementation/<slug>/ so downstream nodes and tool commands can use a stable runtime alias while the slugged workspace remains the source of truth for bookkeeping. Write .spark/state/spec-implementation/<slug>/state.json as valid JSON with this shape: {"request":{"spec_path":"...","spec_slug":"...","scope_hint":"...","constraints":[...],"target_paths":[...],"validation_command":"..."},"spec":{"slug":"...","package_dir":"specs/<slug>","source_path":"specs/<slug>/source.md"},"runtime":{"path":".spark/state/spec-implementation/<slug>","current_alias":".spark/state/spec-implementation/current"},"current_milestone_id":null,"updated_at":"<timestamp>"}. If .spark/state/.gitignore does not exist, create it containing exactly `*` so the state directory ignores itself; do not edit any other ignore file. Do not include extra top-level keys such as details in the response, and choose `done` only after the committed spec package, the runtime workspace, the spec symlink, the current alias, and the workspace files are written. Respond with ONLY a JSON decision envelope.
