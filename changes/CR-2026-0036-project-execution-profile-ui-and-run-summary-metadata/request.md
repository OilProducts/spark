# Project Execution Profile UI And Run Summary Metadata

## Summary
Expose per-project execution profile defaults from the active project controls, and make run execution placement visible in the Runs tab summary. Keep workspace/global execution profile inventory in Settings, keep per-run overrides in Execution, and add a small project-scoped settings surface for project defaults.

## Key Changes
- Add a **Project Settings** dialog from the navbar project control area, enabled only when an active project exists.
  - Entry point: small settings/sliders icon next to the existing project Add/Clear/Remove controls.
  - Dialog title: active project label/path.
  - Field: `Default execution profile`.
  - Options: `Use workspace default` plus enabled profiles from `/workspace/api/settings`.
  - Display validation/config load errors from workspace settings and disable saving when profiles cannot be loaded or are invalid.
- Wire project setting persistence through the existing backend endpoint.
  - Extend frontend `updateProjectStateValidated` payload typing to include `execution_profile_id?: string | null`.
  - On save, call `PATCH /workspace/api/projects/state` with `project_path` and `execution_profile_id`.
  - Update `projectRegistry[projectPath].executionProfileId` from the returned project record.
  - Use `null` for “Use workspace default”.
- Keep launch behavior unchanged except for making the project default easier to set.
  - Execution launch panel continues to show effective profile and per-run override.
  - Precedence remains: run override, project default, workspace default, `native`.
- Add execution placement metadata to the run summary.
  - Add an **Execution** summary section in `RunSummaryCard` when execution metadata exists.
  - Show profile id, mode, container image, worker label/id, mapped project path, and worker runtime root when present.
  - For old runs with no execution metadata, omit the section rather than inventing a default.

## Tests
- Add/update frontend behavior tests for project settings:
  - Opening Project Settings from the navbar with an active project.
  - Rendering enabled execution profiles and the workspace-default option.
  - Saving a selected profile sends `execution_profile_id`.
  - Saving workspace default sends `execution_profile_id: null`.
  - Store/project registry updates from the API response.
  - Invalid workspace execution settings surface an error and block save.
- Add/update run summary tests:
  - A run with execution metadata displays its profile/mode and remote worker/container details.
  - A legacy run without execution metadata does not show a misleading execution section.
- Keep tests behavior-focused: assert user-visible fields and API payloads, not source strings or implementation internals.

## Assumptions
- Project settings are Spark workspace metadata, not files in the user’s project repo.
- The backend project record and `PATCH /workspace/api/projects/state` are the intended persistence path.
- Global Settings remains workspace-level inventory; project defaults belong in the project-scoped dialog.
