# Finish the settings overhaul with targeted corrections

## Summary

Keep the implementation in `.spark/checkouts/run-18d45752f4ec6770` and address the concrete deficiencies identified in `reports/CR-2026-0117-evaluation.md`.

Preserve the agreed architecture, inheritance behavior, configuration editor, and client preferences. This follow-up corrects integration defects and finishes cleanup; it does not reopen the overhaul’s design.

Committing, merging, releasing, and restarting the canceled workflow are outside this plan.

## 1. Make settings edits preserve valid configuration

**Conversation overrides — F1**

- Use the existing complete `ModelSettings` group when editing conversation settings.
- Initialize overrides from the effective group. Changing model or reasoning effort must preserve the selected LLM profile.
- Clear the previous provider/profile selection only when the user explicitly changes that selection.
- Send the grouped `model_settings` payload through the existing conversation settings endpoint. Keep scalar compatibility handling for older callers.
- Extend the contextual provider selector to include configured profiles using the existing selection options. Preserve “Use defaults” and source reporting.

**Profile editing and validation — F3/F6**

- Extend the existing profile-reference validation to evaluate references against candidate profile contents, including edits that retain the profile ID.
- Reject changes that remove a selected model, invalidate a referenced default, or disable a referenced execution profile. Identify the affected configuration.
- Reuse existing domain validators and reference traversal; exclude historical snapshots.
- Use the same domain checks for `settings validate` and Save. Save retains its locked revision/reference recheck.
- Rejected updates must leave the persisted document unchanged. Valid endpoint, label, or model-list changes must remain possible.

## 2. Resolve execution settings in the correct environment

**Desktop startup — F2**

- Resolve supported environment overrides during Desktop bootstrap while retaining its explicit app-owned Spark home.
- Capture the resulting startup-only paths after resolution.
- Preserve those actual startup choices for new work until restart; continue applying live settings changes according to the existing rules.
- Test both startup environment precedence and subsequent restart-only edits.

**Containers — F4**

- Separate portable configuration capture from host executable discovery. Container launches must not capture automatically discovered host binaries as container executable paths.
- Resolve executable and agent-home paths in the execution target before starting the agent. Preserve native execution behavior and freeze target-resolved choices for active work.
- Keep image selection and explicit mount behavior in the existing execution-profile mechanism. Do not introduce a new platform abstraction or profile format.
- Do not forward automatically derived host-home paths as container-home defaults.
- Extend the existing container environment boundary to forward credential variables referenced by captured provider/profile configuration, alongside its conventional allowlist.
- Forward only the required referenced variables; never place credential values in captured settings, artifacts, or logs.

## 3. Make editor feedback truthful and recoverable

**Pending navigation — F5**

- Pass `dirty` and `pending` separately to the existing navigation-protection hook.
- Use its existing pending-save behavior so the UI cannot offer “Discard and leave” for an already-submitted save.
- Correct all affected callers. Do not add cancellation machinery or another navigation framework.

**Effective values and errors — F7**

- Report current stored configuration, active startup-only values, and values used by new clients/work accurately.
- Resolve `client_api_base_url` for a new CLI invocation using the same precedence as the CLI. Keep the running server’s address separate.
- Add concise source information for configurable runtime, connection, provider, and agent values where CLI/environment overrides or restart retention affect the result. Display that information alongside the affected controls.
- Preserve existing model inheritance source reporting.
- For parseable configuration with an invalid section or reference, return its stored value and scoped validation errors while keeping unaffected settings usable. Do not invent an effective fallback.
- Adapt the editor to allow repair of the affected configuration without disabling unrelated sections.
- Malformed, unreadable documents may still produce an actionable document-level error; a new raw-file repair editor is outside scope.

## 4. Finish cleanup and verify the corrections

**Cleanup — F8**

- Reconcile stale documentation with the implemented preferences and migration behavior.
- Remove the obsolete `ui-defaults.json` source only after successful import and backup.
- Make cleanup retryable when migration is already marked complete; do not reimport or overwrite authoritative settings.
- Preserve migration backups.
- Update the implementation result to describe verified behavior and remaining limitations. Remove demands for exactly five phase commits or instructions that send a commit-prohibited worker back to commit.

**Focused regression checks**

- An inherited profile survives reasoning/model edits; explicit provider/profile changes and resetting to defaults work.
- Invalid referenced-profile edits and deletions fail consistently in validation and Save, without changing disk contents.
- Valid edits to referenced profiles still succeed.
- Desktop honors environment-selected agent paths at startup and retains restart-only values afterward.
- Container dispatch does not carry host-discovered executable paths; custom credential references reach the worker without entering persisted snapshots.
- Navigation during a pending save cannot claim to discard that save.
- Effective connection values match new-client resolution, and invalid sections remain repairable.
- Legacy-source cleanup succeeds after migration and can retry without reimport.

Promote the audit reproductions into the appropriate existing test suites. Keep the reports as evidence.

Run affected tests first, followed by the repository gate using an isolated writable `SPARK_HOME`, frontend lint/build, and the Desktop build. Exercise the changed conversation/profile editor paths and one representative real container scenario using disposable configuration and no paid model call.

Report unavailable runtime checks as unverified. Do not convert an environment limitation into another implementation loop or claim that an unperformed check passed.

Completion means the bounded defects are corrected and supported by recorded checks. Delivery history is a separate decision.
