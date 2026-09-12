# CR-2026-0118 validation evidence

## F3/F6 inherited flow reference follow-up — 2026-09-12

Candidate-reference traversal now resolves each authored flow node's model/profile pair using the existing LLM resolution functions and flow defaults before applying the existing profile-model validator. The reproduction with defaults `team`/`m1` and node `work` selecting `m2` rejects removal of `m2` in both Validate and Save, identifying `flow inherited.yaml.nodes.work.execution.llm_profile`. Save's reference lock and document-lock recheck are unchanged. Flow resolution is in memory only; historical snapshots remain excluded.

The existing profile settings suite passed **13 tests**, including the new reproduction, byte-for-byte persistence checks after both rejection paths, valid endpoint/label/model-list edits, and explicit node profile/model precedence. Existing locked-writer and historical-capture tests also passed. The first focused attempt failed only because the new assertion omitted the `.yaml` suffix from the actual diagnostic; the corrected assertion and final implementation passed.

```sh
cargo test -p spark-workspace --test profile_settings
```

Focused log: `/tmp/cr0118-inherited-focused.log`, SHA-256 `bb59e103be88c9b742496c417aadc2691c7dba9e7e188efff70147291e67a935`.

The subsequent exact required isolated-home gate **passed, exit 0**, at 2026-09-12 09:38 UTC: workspace/all-features Rust tests, 76 frontend files / 572 tests, production frontend build, lint (0 errors / 15 existing warnings), and release all-features Desktop build. Existing ignored tests and build warnings remain. Final `git diff --check` passed.

```sh
sh -c 'SPARK_HOME=$(mktemp -d "${TMPDIR:-/tmp}/spark-cr-2026-0118.XXXXXX") && export SPARK_HOME && just test && npm --prefix frontend run lint && cargo build --release -p spark-desktop --bin spark-desktop --all-features'
```

Gate log: `/tmp/cr0118-inherited-gate.log`, SHA-256 `72d4f952cffa5f456fac727b4b8ca9f8b5cb33a692e88c13e944c4a2622e0bc2`.

Original review evidence was inspected and retained unchanged at `/var/folders/wx/wh9np15d04ncb_k6wlcks2_r0000gn/T/cr0118-review-reference-w2_eiglo/`: `result.json` (SHA-256 `ee3785f173f26f31100ecd381ce42cdd25ba347c8c84bf4ae7ebecd3906bca7e`) and `flows/inherited.yaml` (SHA-256 `b234d915afc5d77b58c48c37e355e5023ce2a32722854806b58f6f75d0eeed4a`). The flow was promoted into the existing suite without a model call.

Earlier browser coverage and the unavailable Docker/interactive Desktop checks below remain the runtime evidence and limitations for the preserved implementation. No implementation restart or cleanup was performed.

## F7 review follow-up — 2026-09-12

The remaining project execution parser/dialog gap is corrected. Scoped errors retain the original stored value and revision; malformed types and invalid references show the server error with an enabled replacement selector. Save requires an explicit profile or workspace-default draft. The existing endpoint and navigation protection are unchanged.

Focused frontend command passed: 2 files / 12 tests, including both malformed numeric selections and invalid profile references through the real parser.

```sh
npm --prefix frontend run test:unit -- src/features/settings/__tests__/SettingsRepair.test.tsx src/features/settings/__tests__/ProjectSettingsProtection.test.tsx
```

All 14 existing settings browser scenarios, including the promoted repair reproduction, passed against a fresh disposable home. Both replacement and reset require Save, preserve unrelated project metadata, and reject an old revision with HTTP 409 and byte-for-byte unchanged repaired contents. The suite also exercised the conversation and profile editors without a model call.

Run from `frontend/`:

```sh
SPARK_SETTINGS_TEST_HOME="$PWD/.tmp-cr0118-f7-bnnipixo/home-final" PLAYWRIGHT_BROWSERS_PATH=/Users/chris/Library/Caches/ms-playwright ./node_modules/.bin/playwright test --config=.tmp-cr0118-f7-bnnipixo/playwright-final.config.ts
```

The new regression waits for project activation's metadata PATCH before injecting malformed configuration. Its first attempt exposed that fixture race. An intermediate browser retry used the wrong working directory and reused conversation state; the final run used the frontend working directory and a fresh home. All artifacts were subsequently retained under `frontend/.tmp-ui-smoke/cr0118-f7-bnnipixo/`; the configuration and logs retain the original run paths.

The first exact gate attempt stopped on a local HTTP fixture read timeout (`WouldBlock`) in `workflow_executes_with_captured_profile_after_profile_file_changes`. That test passed independently before rerunning the exact gate. No unrelated implementation changes were made for that transient failure. The next gate run passed all Rust tests, 572 frontend tests, and the frontend build, then lint discovered four errors in generated third-party plugin files inside the disposable browser home. The artifacts were retained under the existing ignored smoke-test directory before the final exact gate rerun; no source lint rules were changed.

Live Docker execution remains unverified because the daemon is unavailable, as recorded below; no additional container implementation loop was attempted.

Final exact gate **passed, exit 0**, at 2026-09-12 09:24 UTC: workspace/all-features Rust tests, 76 frontend files / 572 tests, production frontend build, lint (0 errors / 15 existing warnings), and the all-features release Desktop build. Existing ignored tests and build warnings remain. Final `git diff --check` passed.

```sh
sh -c 'SPARK_HOME=$(mktemp -d "${TMPDIR:-/tmp}/spark-cr-2026-0118.XXXXXX") && export SPARK_HOME && just test && npm --prefix frontend run lint && cargo build --release -p spark-desktop --bin spark-desktop --all-features'
```

### F7 retained logs

| Log | SHA-256 |
| --- | --- |
| `/tmp/cr0118-f7-gate-final.log` | `f3d00d80462c023214e46090128fc90af0410e621e4b163f98112004667517bb` |
| `/tmp/cr0118-f7-focused.log` | `fa26bda8ef01217b6e0345f16c67a27018f6def717530921e33924d74633fc0c` |
| `/tmp/cr0118-f7-gate.log` | `b9b04fd7abd41df92e5792f28238edd45fea8f43db010103a781da6865a3b542` |
| `/tmp/cr0118-f7-gate-recheck.log` | `f3c15619821ad55154965cace2901cab337c5c499944ad8b4002538556cccd71` |
| `/tmp/cr0118-f7-gate-retry.log` | `6eabb034d9595cac050dc40953fcbabd76a1e9049fa9216dc80adbf374fb0fe2` |
| `/tmp/cr0118-f7-build.log` | `85b84e89473e4f9ba2fba2714c6d5062f157f3d0705173733f44d5b8a1b562ca` |
| `/tmp/cr0118-f7-browser.log` | `548e56ff0b288d8465da0e0c35b88191d474ad278ef35152a5e513bce810a2a9` |
| `/tmp/cr0118-f7-browser-retry.log` | `b87470b9d7a31c2898ecba33bd994305a7188863321cf31b377e08f5774c5e5b` |
| `/tmp/cr0118-f7-browser-final.log` | `6d299a25cee2cee2914c354a7514654bfefba810eca153036d039faaae98994b` |
| `/tmp/cr0118-f7-lint.log` | `9a10877f0a78806da5c4d9351c23b99817fac454ced4daab3008213021e9c224` |

## Earlier correction evidence

Verified at 2026-09-12 09:01 UTC. Worktree HEAD remains `eca8b851f137cc36bed0898092675a734b53fd10`; the implementation changes are uncommitted. The preserved source worktree remained unchanged and clean.

## Final gate — passed, exit 0

```sh
sh -c 'SPARK_HOME=$(mktemp -d "${TMPDIR:-/tmp}/spark-cr-2026-0118.XXXXXX") && export SPARK_HOME && just test && npm --prefix frontend run lint && cargo build --release -p spark-desktop --bin spark-desktop --all-features'
```

Workspace Rust tests with all features passed. The frontend passed 76 files / 570 tests and its TypeScript/production build. Lint had 0 errors and 15 warnings. The release Desktop build passed. Existing ignored tests remain ignored; Vite still reports large-chunk warnings.

Focused checks preceded the gate: profile candidate/reference/persistence checks; storage migration/cleanup and CLI precedence; Desktop bootstrap and retention in a subprocess; scoped settings reads and repair; conversation captures/history; container target-shell and credential-boundary tests; and launch-to-worker dispatch through the existing executor. The affected frontend run passed 15 files / 74 tests.

## Real browser — 5 passed, exit 0

The existing `frontend/e2e/smoke/settings-editor.spec.ts` scenarios ran against a disposable server/home using the local Playwright dependency and installed Chromium. The final configuration and browser output are retained under `frontend/.tmp-cr0118-browser-ckyal_bb/`. `PLAYWRIGHT_BROWSERS_PATH=/Users/chris/Library/Caches/ms-playwright` points at the installed cache; the agent's inherited HOME otherwise selects an empty cache.

The selected scenarios covered runtime Save/Discard/navigation/conflicts/reload, profile CRUD/conflicts/mount validation, connection values, provider/agent settings, and conversation reasoning/model/profile selection and resetting defaults. The conversation scenario verified an empty turn list throughout: no model request was submitted.

An earlier browser attempt confirmed the expected HTTP 409 when the next section was edited before receiving a shared-document revision update. The sequential provider/agent test now reloads the saved revision between edits; the separate conflict scenarios remain intact and pass. Earlier whole-read HTTP 400 assertions were updated to the requested scoped-error HTTP 200 contract, retaining rejection and no-write assertions for invalid updates/workflow starts.

## Unverified runtime checks

`docker version` found Docker CLI 29.4.3 but could not connect to `/var/run/docker.sock` because no daemon socket exists. No real container was started. Target-shell and fake-Docker dispatch tests are automated evidence, not a live container claim. The executor freezes target paths for its active lifetime; reconstruction resolves them in the reconstructed target.

An interactive native Desktop GUI restart was not performed; bootstrap/retention subprocess tests and the release build passed. No paid model call, commit, push, merge, release, canceled-workflow restart or source-worktree cleanup was performed. Disposable homes and artifacts were retained.

## Retained local logs

| Log | SHA-256 |
| --- | --- |
| `/tmp/cr0118-final-gate.log` | `f335a844d59c732565f2df41272fd888dbfe2917808ec54f1deaa8122d81812f` |
| `/tmp/cr0118-final-focused.log` | `b5da15b824dfb74a46a06b74f2683c24b40aec458d0e6924537f7c26efbd3566` |
| `/tmp/cr0118-history-focused.log` | `98cdea2f71e3f8559acfdc32658008131bb5c958ee92a61f55e17ddbb41dfc9d` |
| `/tmp/cr0118-container-tests.log` | `61caa9437a0380082992a8d0fe5e1cdc54e368c2a3f6fb59f214a3a041dfe3c1` |
| `/tmp/cr0118-dispatch.log` | `47f08737ca4b57ea7f2cdf93f2c8236f1dac5d2e0ff0c85a7b373ed562cf1a1a` |
| `/tmp/cr0118-focused-ui.log` | `71a391e814a5528b2023699eb0b6a4b19cfd880506fed739aef355181fcb5017` |
| `/tmp/cr0118-browser-final.log` | `b2caefb1469d4980392c15359c23bf462ac088c4df09e0720c5270c89ced31fd` |
| `/tmp/cr0118-docker.log` | `3bdf27cbe5684ce9c631d2d7027b5e5de60bbc1f2d512233a42cc57f7930e326` |

The original independent audit and reproduction sources remain under `reports/CR-2026-0117-*`. These remain historical evidence; the new regressions are in the repository's existing suites.
