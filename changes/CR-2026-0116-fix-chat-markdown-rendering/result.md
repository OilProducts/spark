# Result

Implemented the shared Home/Run Markdown upgrade without persisted-message schema or HTTP API changes. The superseded run request was not launched.

- Retained react-markdown; added GFM, untrusted KaTeX math, syntax highlighting, safe AST line breaks, descending headings, paragraph spacing, and contained wide content.
- Added accessible external web links, HTTP/HTTPS-only Tauri opener permissions, literal local-path copying, instance-local footnotes, and lazy web images with failure/local-path fallbacks.
- Added lazy strict Mermaid rendering with scanning/error diagrams disabled, one in-flight render per component, latest-source coalescing, stale/unmount protection, source disclosure, and existing copy eligibility.
- Expanded thinking uses the shared renderer, preserving bold headings and default collapse; headingless content expands under “Thinking”. User/tool text remains literal. Existing message memoization, streaming, scrolling, and copy feedback are retained.
- Updated UI/workspace specifications and transcript tests. Added 20 focused renderer/lifecycle tests and a real-browser math/Mermaid streaming and containment scenario. Fenced copying includes nested containers and original line endings.

## Review correction

Fixed `safeBreaksAndCode` to recover Markdown using the full document, preserving reference-link/image and footnote resolution across recovered fragments. AST-selected break placeholders retain source lengths and positions; standalone breaks preserve block boundaries, and inline breaks remain inside their formatting. Definition collection runs after recovery so literal referenced paths copy without URL encoding. Raw HTML remains suppressed and code copying reads the original source.

Eight additional actual-renderer regressions cover all three break spellings with document-level references, remote/local images, exact referenced-path copying, and instance-local footnotes whose definitions are outside the recovered fragment. They also cover definitions recovered in another HTML fragment, original CRLF code copying, nested inline formatting, and literal placeholder-like text. The existing HTML suppression, soft-break, code-content, copying, thinking, and Mermaid lifecycle regressions remain in place. Correction scope is the shared transformation, its tests, and this result/evidence.

## Earlier validation

- Correction validation: focused renderer/history tests passed (55/55). [Focused log](evidence/revision-focused.log).
- `just test`: passed after the correction, including Rust workspace/desktop contracts, 61 frontend files / 525 tests, and frontend build. [Complete log](evidence/revision-just-test.log).
- `npm --prefix frontend run lint`: rerun after smoke; two errors / 15 warnings. [Correction lint log](evidence/revision-lint.log). An unchanged HEAD archive reproduces both errors: SettingsPanel restricted import and generated smoke fixture lint-rule lookup. [Paired baseline comparison](evidence/comparison.md).
- Full `npm --prefix frontend run ui:smoke`: 24/30 passed, including real math/Mermaid streaming and containment and editor navigation. Unchanged HEAD passes 22/29 and reproduces all six changed-build failures at the same assertions, plus editor navigation. [Correction smoke log](evidence/revision-smoke.log), [initial changed log](evidence/smoke.log), [baseline log](evidence/baseline-smoke.log). The full gate remains nonzero; unrelated failures are demonstrated by paired execution, not assumed to predate this change.
- Desktop build passed. UI-driven desktop verification on 2026-09-10 at 17:04–17:05 HST activated HTTP and HTTPS by actual mouse clicks and Enter on focused links. Each opened another tab in the configured system browser (Chrome). HTTP redirected to HTTPS while preserving its distinct marker. Spark remained on the conversation. [Four timestamped observations](evidence/desktop-actions.json), [screenshot](evidence/desktop-renderer.png), [action script](evidence/desktop-check.py), [accessibility output](evidence/desktop-actions.log), [binary/assets hashes](evidence/desktop-build-sha256.txt).

Desktop verification used a separate process and [fixture](evidence/desktop-fixture.js) supplying a synthetic assistant conversation to the production frontend bundle. The fixture mocked conversation fetches only; it did not replace the shared renderer, Tauri bridge, opener, or browser. Keyboard activation used accessibility focus followed by Enter. The generated index was restored byte-for-byte afterward. The binary was built with a temporary identifier override; no persisted capability changes were made for verification. This was agent-operated GUI verification, not a claim of a separate human tester.

No commit, branch, additional worktree, superseded run, persisted schema change, or HTTP API change was created.

## Gate-blocker stabilization

- Added `.tmp-ui-smoke/**` to ESLint global ignores. Smoke-generated plugin bundles remain present; the import-boundary rule remains enabled.
- Moved Settings model discovery state and requests into `features/settings/hooks/useModelDiscovery.ts`, preserving loading/failure feedback, project matching, cancellation, provider filtering and saved defaults. Existing settings regressions pass (18/18).
- Corrected new editor nodes to declare `kind: agent_task`, making prompt controls available immediately. Updated smoke selectors to the current `+ Node` and `YAML` controls, wait for graph hydration, and use diagnostic tokens distinct from flow names.
- Synthetic project registration now stubs host CLI model discovery; previously pending discovery coincided with blocked flow loads in the isolated first scenario. The four editor scenarios now pass in isolation.
- Replaced two obsolete stylesheet smoke scenarios with metadata preview diagnostics and metadata YAML round-trip coverage. The existing GraphSettings contract explicitly requires stylesheet controls to be absent following CR-2026-0072's FlowDefinition cutover; no removed editor feature was restored.
- Canonicalized the temporary project root using Node `realpathSync`, matching the server's existing canonical-path contract on macOS. The product-shell and YAML-save scenarios pass individually within the targeted run.

These fixes do not change Markdown rendering. The initial attempt to rerun smoke tests found no browser in the runtime's default cache; subsequent executions use the existing browser via `PLAYWRIGHT_BROWSERS_PATH=/Users/chris/Library/Caches/ms-playwright`. See [settings tests](evidence/blockers-settings.log), [lint with artifacts](evidence/blockers-lint.log), [targeted shell/save results and intermediate editor failure](evidence/blockers-isolated-final.log), and [passing isolated editor scenarios](evidence/blockers-editor-final.log).

## Final validation

The prescribed `just test && npm --prefix frontend run lint && npm --prefix frontend run ui:smoke` completed with exit code **0** after stabilization. Rust workspace tests and desktop contracts passed (including the scoped opener permission contract); all **61 frontend files / 525 tests** passed; the frontend build passed; lint passed with **0 errors / 15 existing warnings**, with smoke artifacts present; all **30 browser smoke scenarios** passed, including real math/Mermaid streaming and containment and every stabilized editor/path scenario. [Gate summary](evidence/blockers-gate-summary.log).

The prior UI-driven desktop link verification above remains applicable: these blocker fixes do not change the renderer, opener, or desktop capabilities. `git diff --check` also passes. No commit, push, branch/worktree creation, or cleanup was performed by this revision.


## URL normalization correction

The shared web-link handler now passes `new URL(href).href` to Tauri's opener after the existing validation. This normalizes scheme and host casing before case-sensitive capability matching, while preserving path/query/fragment casing. Literal local-path copying, unsafe-scheme rejection, and HTTP/HTTPS-only permissions remain unchanged. Four actual-renderer regressions cover uppercase and mixed-case HTTP and HTTPS, including default-port normalization, and assert exact opener arguments.

- Focused renderer/history tests: **59/59 passed**. [Log](evidence/url-normalization-focused.log).
- Prescribed `just test && npm --prefix frontend run lint && npm --prefix frontend run ui:smoke`, with `PLAYWRIGHT_BROWSERS_PATH=/Users/chris/Library/Caches/ms-playwright`: **exit 0**. Rust/desktop checks, **529 frontend tests**, frontend build, lint (**0 errors / 15 warnings**), and **30 smoke scenarios** passed, including real math/Mermaid. [Gate log](evidence/url-normalization-gate.log).
- Fresh desktop build: **passed**, using a temporary app identifier without changing repository configuration or capabilities. [Build log](evidence/url-normalization-desktop-build.log), [binary/assets hashes](evidence/url-normalization-desktop-build-sha256.txt).
- Agent-operated GUI verification on 2026-09-10 at 17:52–17:53 HST: mixed-case `hTtP` and `hTtPs` links with uppercase hosts each opened exactly one matching Chrome tab by mouse and by Enter (**4/4 actions**). Spark retained the conversation. The fixture supplies conversation data only; the production renderer, Tauri bridge, and scoped opener are real. [Assertions and timestamped browser URLs](evidence/url-normalization-desktop-actions.json), [action script](evidence/url-normalization-desktop-check.py), [fixture](evidence/url-normalization-desktop-fixture.js), [accessibility log](evidence/url-normalization-desktop-actions.log). The screenshot includes an unrelated Chrome keychain dialog; successful navigation is established by the browser-tab assertions.

The temporary frontend index injection was restored byte-for-byte. `git diff --check` passed. No commit, push, branch/worktree creation, superseded run launch, or permission expansion was performed.
