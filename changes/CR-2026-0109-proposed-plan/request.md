## Settings provider and model dropdowns

Replace the two Settings text inputs with the existing `NativeSelect` component used beneath project chat. Keep the current settings layout and immediate persistence.

### Implementation

- Populate the provider dropdown from existing providers and configured LLM profiles. Preserve any saved value missing from that list.
- Populate models from chat’s existing model-discovery endpoint for the active project, filtered by provider. For configured profiles, use that profile’s model list.
- Fall back to existing model suggestions when discovery is unavailable or no project is selected. Show a brief loading or discovery-error message without overwriting saved settings.
- Include “Custom model…” to reveal a labeled text input. Existing unlisted models remain visible and editable. Include an empty “Use handler default” option.
- Preserve the current model when switching providers, displaying it as custom if necessary. Refresh the available choices without silently changing the saved model.
- Keep existing provider/profile mapping, persistence, and reasoning-effort behavior. No backend, API, or storage changes.

### Verification

Update the existing Settings tests to cover dropdown selection and persistence, provider-dependent models, configured profiles, custom entry, saved unlisted values, and discovery loading/failure. Verify stale discovery responses cannot replace the current project’s choices.

Run the affected frontend tests, then the repository’s `just test` gate.

### Defaults

Custom model entry is supported as requested. This change applies only to the global Settings controls; project chat remains unchanged.
