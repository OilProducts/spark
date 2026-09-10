# Separate project initialization from metadata updates

## Summary

Remove repeated project metadata writes from conversation event ingestion and other storage operations. Preserve automatic project initialization where callers currently depend on it, and make registration/opening explicit.

Keep the 50,000-event test unchanged and report its before/after runtime.

## Implementation

- Make `ProjectRegistry::ensure_project_paths` idempotent: retain path validation and directory creation, but leave an existing valid `project.toml` untouched. Initialize missing metadata; retain existing repair/default behavior for incomplete or malformed records, preserving recoverable settings and custom display names.
- Add `ProjectRegistry::register_project(&str) -> Result<ProjectRecord>`. It ensures initialization and explicitly refreshes `last_opened_at`, preserving `created_at`, custom names, and optional settings.
- Make `read_project_record` resolve paths and read without initialization or writes. Missing projects return `None`.
- Keep `update_project_record` able to initialize missing projects. For existing projects, write only the requested update and preserve `last_opened_at`.
- Reuse existing path-resolution and atomic-write helpers. Add no dependencies, caching layer, or changes to event persistence.

## Caller migration and compatibility

- Change `WorkspaceProjectService::register_project` and test setup that uses reads as registration to call the explicit registry registration method. Preserve execution-profile validation and selection.
- Keep conversation commits, provider-event and trace appends, runtime-session writes, conversation reads/listing/deletion, and task operations using idempotent initialization. Their existing ability to initialize unregistered projects remains.
- Leave flow-launch execution-profile lookup using the now-pure read; missing projects continue to yield no default profile.
- Preserve frontend `last_accessed_at` updates. Stop incidental reads, events, and state updates from refreshing `last_opened_at`.
- Keep HTTP interfaces and persisted field names unchanged. No migration is required.

## Verification

Extend existing contract tests to cover:

- Fresh initialization creates the expected directories and metadata; repeated initialization preserves metadata and recreates missing directories.
- Explicit registration refreshes a fixed old opening timestamp while preserving creation time, custom name, favorites, active conversation, and execution profile.
- Reading a missing project returns `None` without creating its directory; reading an existing project performs no metadata rewrite.
- Updating existing metadata preserves unspecified fields and opening time; updating a missing project still initializes it.
- Conversation commits and task operations still work without prior registration; absent execution-profile lookup retains its fallback.
- Multiple provider events persist correctly while project metadata contents and modification time remain unchanged. On Unix, also compare file identity to detect atomic replacement with identical contents.
- Legacy optional-field defaults, initialization repair behavior, path validation, and provider-write failure protection remain covered.

Use fixed historical timestamps rather than sleeps. Run affected storage and workspace contract tests, time the unchanged 50,000-event test before and after under comparable warm-build conditions, then run `just test`. Report timings without adding a wall-clock assertion.

## Assumptions

- `last_opened_at` records explicit registration/opening; frontend activity continues to use `last_accessed_at`.
- Removing implicit registration from conversation/task reads is outside this change.
- Stress-test resizing, unrelated persistence optimization, and a broader metadata-concurrency redesign are deferred.
