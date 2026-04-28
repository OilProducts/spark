# Provider Configuration Preflight And Non-Retryable Setup Failures

## Summary
Add a launch-time provider readiness check that catches obviously missing local credential material before a flow run is scheduled, and classify provider setup/auth failures as non-retryable if they still occur during execution. Keep the check intentionally shallow: verify local configuration presence, not network validity or inference availability.

## Key Changes
- Add provider configuration validation for the effective launch provider before creating/scheduling a pipeline run.
- For API providers, validate required env material only:
  - `openai`: `OPENAI_API_KEY`
  - `anthropic`: `ANTHROPIC_API_KEY`
  - `gemini`: `GEMINI_API_KEY` or `GOOGLE_API_KEY`
- For `codex`, validate local Codex runtime/auth material only:
  - Build/resolve the normal Codex runtime environment.
  - Confirm resolved `CODEX_HOME` exists after runtime setup.
  - Confirm `CODEX_HOME/auth.json` exists.
  - Do not start `codex app-server`, call `model/list`, start a thread, or run a turn in preflight.
- Apply the same validation to parent launches, retry/continue launches, and first-class child pipeline launches so inherited providers fail consistently.

## Failure Behavior
- If preflight fails, reject the launch before the run is scheduled/started with a clear user-facing validation error naming the provider and missing material.
- If provider setup still fails during node execution, return/mark the failure as non-retryable configuration/setup failure rather than default retryable runtime failure.
- Do not add fallback provider behavior. A selected provider that is not configured should fail clearly, not silently switch to another backend.

## Implementation Notes
- Reuse current provider resolution semantics: graph default, requested launch provider, and inherited child-run provider should keep their existing precedence.
- Reuse existing Attractor provider classification helpers where applicable, especially `resolve_effective_llm_provider` and `node_uses_llm_backend`.
- Avoid `UnifiedLlmClient.from_env()` for validation because it constructs adapter clients; use a small explicit env-material check instead.
- For Codex, use the existing `build_codex_runtime_environment()` behavior as the source of truth for runtime seeding and path resolution.
- Keep validation scoped to runs that actually need an LLM-backed node. Non-LLM-only flows should not require provider credentials.

## Test Plan
- Update existing launch/API tests to cover missing API provider key rejection before scheduling.
- Update existing launch/API tests to cover missing Codex `auth.json` rejection before scheduling.
- Update existing execution/backend tests so provider setup failures are non-retryable.
- Cover inherited provider validation for child pipeline launch or retry/continue paths using existing test patterns.
- Run `uv run pytest -q`.

## Assumptions
- Preflight checks local credential/config presence only; they do not prove credentials are valid remotely.
- `config.toml` is not required for Codex preflight when a model is explicitly selected or discoverable later.
- Missing provider configuration is a launch validation/configuration problem, not a retryable runtime problem.
