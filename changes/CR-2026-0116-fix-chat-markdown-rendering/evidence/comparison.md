# Validation comparison

Baseline: `00dc9449caf17595f6eef4f67de7afbe989df876`, extracted with `git archive HEAD` into the directory recorded in baseline-path.txt. No branch or worktree was created. The archive used the same installed node_modules via symlink, browser cache, Cargo target cache, Playwright configuration, and default 8 workers. Its own frontend was built before smoke. Both runs used `PLAYWRIGHT_BROWSERS_PATH=/Users/chris/Library/Caches/ms-playwright npm run ui:smoke`.

Baseline: 22/29 passed. Changed build: 24/30 passed, including the added real Markdown smoke. Editor navigation failed on baseline and passed on the changed build. Every changed-build failure reproduces on baseline at the same assertion:

- e2e/smoke/editor-diagnostics.spec.ts:95:1 › prompt edits trigger live preview diagnostics before blur for item 5.1-03 — `editor-diagnostics.spec.ts:131:66` in both runs.
- e2e/smoke/editor-diagnostics.spec.ts:288:1 › inline node and edge diagnostic badges render for item 7.1-02 — `editor-diagnostics.spec.ts:333:66` in both runs.
- e2e/smoke/editor-diagnostics.spec.ts:553:1 › stylesheet parse diagnostics render in graph settings for item 6.5-02 — `editor-diagnostics.spec.ts:594:69` in both runs.
- e2e/smoke/editor-diagnostics.spec.ts:615:1 › stylesheet selector/effective previews render in graph settings for item 6.5-03 — `editor-diagnostics.spec.ts:634:27` in both runs.
- e2e/smoke/editor-save.spec.ts:17:1 › raw YAML save blocks parse errors and hydrates valid handoff — `editor-save.spec.ts:133:56` in both runs.
- e2e/smoke/rust-product-shell.spec.ts:30:1 › Rust product shell serves the built SPA and owns core browser routes — `rust-product-shell.spec.ts:76:51` in both runs.

The shared failures are missing Add Node controls (prompt diagnostics and badges), missing stylesheet controls (both scenarios), missing Raw YAML control, and `/var` versus `/private/var` project-path equality. See paired smoke logs for complete expected/received values and locators.

Bare `npm run lint` after smoke yields the same two errors and 15 warnings in both trees: SettingsPanel restricted API import, and generated `.tmp-ui-smoke/.../plugin-entry.ts` referencing an unavailable ESLint rule. See paired lint logs. These failures were reproduced, not inferred from file history.

## Document-definition correction rerun

The correction passes 55 focused renderer/history tests and `just test` (525 frontend tests plus Rust/desktop contracts and production build). The installed browser cache was reused with the same full smoke command: 24/30 passed, including real math/Mermaid. All six failures match the baseline assertion locations listed above. See [revision smoke](revision-smoke.log), [focused tests](revision-focused.log), and [repository gate](revision-just-test.log).

[Revision lint](revision-lint.log), run after smoke completed, retains the baseline two errors and 15 warnings. A concurrent lint attempt was interrupted by smoke resetting its generated fixture directory; the final sequential lint run is the recorded result. Original baseline and initial changed-build logs remain intact.
