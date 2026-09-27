# Reusable model chooser

## Summary

Replace the five separate model choosers in the frontend with one `ModelChooser` component backed by one `useModelOptions(projectPath)` hook, so every place that picks a provider, model, and reasoning effort offers the same choices.

## Why

The choosers disagree today:

| Where | Implementation | Models offered | Efforts |
|---|---|---|---|
| Settings (workspace and project defaults) and the mission Model menu | `features/settings/ModelSettingsFields.tsx` | discovery, else static suggestions; custom allowed | fixed list |
| Chat composer | `features/projects/components/ProjectConversationSurface.tsx` with option building in `features/projects/hooks/useProjectsHomeController.ts` | discovery for Codex only, with its own fetch and cache | from model metadata |
| Flow graph defaults | `features/editor/components/graph-settings/GraphSettingsSections.tsx` | static suggestions | fixed list |
| Node inspector | `features/editor/components/NodeInspectorPanel.tsx` | static suggestions in a free-text input | fixed list |
| Canvas node inline edit | `features/workflow-canvas/TaskNode.tsx` | static suggestions | fixed list |

So the flow editor does not offer models Codex actually reports, efforts differ by screen, and model discovery is fetched twice.

## Component

- `ModelChooser` takes `value: ModelSettings` (`provider`, `llm_profile`, `model`, `reasoning_effort`) and `onChange`. Persistence stays with the caller: the chat saves on change; settings and missions keep Save and Discard.
- Props:
  - `projectPath`, for discovery;
  - `inheritLabel`, naming what an empty choice means here (for example "Workspace default", "Graph default", "Provider default");
  - `layout`: `fields` (labelled, stacked) or `compact` (inline);
  - `disabled`.
- Options, identical everywhere:
  - Providers: the known providers plus configured LLM profiles.
  - Models: the selected profile's models; otherwise the discovered models for that provider (Codex and Claude Code); otherwise static suggestions. A custom model can always be entered. A saved value that is not listed shows as "(custom)".
  - Efforts: the model's supported efforts from discovery metadata when known; otherwise the standard list.
  - Discovery states: say so when discovery is loading or unavailable, and fall back to suggestions.
- `useModelOptions(projectPath)` fetches discovery once per project and shares the result across every mounted chooser. It refreshes on the existing `spark:codex-connected` event.

## Adoption

- Settings workspace defaults, project defaults, and the mission Model menu: replace `ModelSettingsFields`, then delete it.
- Chat composer: render `ModelChooser` with `layout="compact"`, and remove the controller's option building (`buildModelOptions`, `buildReasoningEffortOptions`) and its separate discovery cache. Keep in the controller the chat's own rules: blocking Send when the selected Codex model is not available, the availability message, and falling back to the discovered default model when none is set.
- Flow graph defaults, node inspector, and canvas node: render `ModelChooser`, converting to and from the flow attributes (`llm_provider`, `llm_profile` where supported, `llm_model`, `reasoning_effort`) at the call site.
- Keep existing test ids and accessible labels where tests or smoke specs depend on them, or update those tests.

## Verification

- Component tests:
  - provider, profile, model, and effort selection emit the expected value;
  - discovered models take precedence over suggestions, and profile models over both;
  - efforts follow the selected model's metadata;
  - custom model entry works, and an unlisted saved value appears as custom;
  - empty choices use `inheritLabel`;
  - discovery loading and unavailable states;
  - one discovery fetch for several choosers on the same project.
- Existing suites for settings, missions, the chat composer, the graph settings, the node inspector, and the task node pass, updated only where markup changed.
- Browser smoke: settings, missions, and the editor specs pass after `npm --prefix frontend run build`.
- `just test`.

## Out of scope

- Backend changes. The discovery endpoint and model settings APIs stay as they are.
- Changing how the chat decides whether it can send.

## Hand-off

Write `changes/CR-2026-0125-reusable-model-chooser/result.md` (frontmatter with id, title, status, type, changelog; sections Summary, Validation, Shipped Changes) when the work is complete.
