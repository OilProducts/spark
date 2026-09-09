# CR-2026-0109 result

Replaced global Settings provider and model inputs with full-width NativeSelect controls in the existing layout. Provider choices reuse the existing provider/profile catalog and retain saved unlisted selections. Model choices use the active project's existing chat discovery client, filtered by provider, or configured profile model lists, with existing suggestions as fallback.

Added handler-default and custom-model choices, a labeled custom input, and brief discovery loading/error feedback. Provider changes and discovery completion preserve the saved model and reasoning effort. Cancelled requests and project-scoped results prevent stale choices after project changes. Persistence uses the existing settings actions; project chat, backend, APIs, and storage are unchanged.

Updated Settings coverage in GraphSettings and ContractBehavior tests for selection/persistence, provider filtering, profiles, custom entry, saved unlisted values, discovery loading/failure, and stale successes/failures.

Validation:
- Affected frontend tests: 71 passed across 2 files.
- Repository `just test`: passed (Rust formatting, all-feature workspace tests, full frontend unit suite, TypeScript check and production build).
