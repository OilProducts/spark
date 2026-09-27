# Model picker

## Summary

Replace the inside of `ModelChooser` (`frontend/src/components/model-chooser/ModelChooser.tsx`), today three native selects for provider, model, and effort, with one button that opens a searchable picker. Choosing a model chooses its provider. The button always shows what will actually run, including what "default" resolves to. Every caller of `ModelChooser` gets the picker without call-site changes beyond one optional prop.

## Why

Three selects put the provider first although the user is choosing a model, hide what "Provider default" resolves to, offer efforts the chosen model may not support, and take three controls wherever they appear, which is heavy in the chat composer and on canvas nodes.

## The button

- Explicit choice: the model's display name and effort, for example `Opus 5.5 · High`.
- Inherited choice: `Default: <resolved model> · <resolved effort>`, in muted text. The resolved value comes from the new optional `inherited` prop (a `ModelSettings` the caller already knows, such as the workspace or graph default). When `inherited` is absent, resolve from the discovered default for the provider (`is_default`), and when that is unknown too, show `inheritLabel`.
- `layout="compact"`: the button alone. `layout="fields"`: the same button full width under a "Model" label.
- Keeps `disabled` and `invalidModel` (invalid shows the existing error styling and message).

## The picker

A popover (Radix Popover from the installed `radix-ui`; no new dependency) containing, top to bottom:

1. A search input, focused on open, filtering by model name, display name, provider, and profile.
2. "Use default", showing what it resolves to. Choosing it sets provider, profile, and model to null.
3. Models grouped by provider (Codex, Claude Code, and other providers with suggestions) and then by LLM profile, using `useModelOptions` exactly as today: a profile's models, discovered models with their display names and default markers, otherwise suggestions. The current choice is marked.
4. When the search text matches nothing, or matches no model exactly: *Use "text" as a custom model* for the provider in context (the current provider, else the first group). This replaces the separate custom-model input.
5. Effort as a row of toggle buttons: "Default" plus only the efforts the highlighted or selected model supports (from discovery metadata; the standard list when unknown). A saved effort the model does not list stays visible, marked custom.

Behavior:

- Choosing a model sets provider or profile and model together and keeps the popover open for an effort choice. Choosing an effort, or pressing Enter on a model, closes it.
- A provider whose discovery is unavailable shows that in its group header, with its suggestions still listed. While discovery loads, show a loading row.
- Keyboard: typing filters, arrow keys move through models, Enter chooses, Escape closes and returns focus to the button. Use listbox and option roles so screen readers announce the list and the selection.
- `onChange` fires once per committed choice, not per keystroke in the search box.

## Call sites

Pass `inherited` where the caller already has the resolved default: chat (the conversation's effective or workspace default), workspace and project settings, the mission Model menu, graph defaults (the workspace default), and node inspector and canvas node (the graph's model settings). Nothing else changes at call sites. The chat keeps its own send rules and save queue.

## Verification

- Component tests:
  - the button shows an explicit choice and a resolved default;
  - search filters across groups;
  - choosing a model sets provider or profile and model in one `onChange`;
  - "Use default" clears them;
  - custom entry appears for unmatched text and commits it;
  - efforts follow the model's metadata, and an unsupported saved effort shows as custom;
  - unavailable and loading discovery states;
  - keyboard navigation and Escape focus return;
  - no `onChange` while typing in search.
- Update the existing settings, missions, chat composer, graph settings, node inspector, and task node tests for the new markup.
- Browser smoke: settings, missions, chat, and editor specs pass after `npm --prefix frontend run build`. Capture screenshots of the chat composer and the editor inspector at 1440 and 390 in light and dark.
- `just test`.

## Out of scope

- Backend changes.
- A sign-in action for unavailable providers (tracked as a separate mission).

## Hand-off

Write `changes/CR-2026-0126-model-picker/result.md` (frontmatter with id, title, status, type, changelog; sections Summary, Validation, Shipped Changes) when the work is complete.
