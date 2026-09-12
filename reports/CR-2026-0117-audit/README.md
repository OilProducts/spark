**Audit evidence for CR-2026-0117**

The [report](/Users/chris/projects/spark/reports/CR-2026-0117-evaluation.md) evaluates the uncommitted implementation in `.spark/checkouts/run-18d45752f4ec6770`, based on commit `9839f9f0a2eeed5d5352ab7045b8bcf6564b19f8`. These probes were added outside that implementation worktree.

- `api_probe.py` creates its own temporary Spark home, starts a loopback server, makes settings-only requests, and terminates that server. It makes no model calls. `api-results.txt` records F1, F3, F6, and F7.
- `desktop_precedence.rs` calls the actual Desktop bootstrap, configuration reader, and startup-retention functions. `desktop-precedence-results.txt` records the failed environment-precedence assertion (F2).
- `pending-navigation.test.tsx` renders the actual Runtime editor and dialog/navigation hooks, with the network save promise held pending. `pending-navigation-results.txt` records the failed navigation assertion (F5).
- `validation-summary.txt` records the independently run existing gates and their limitations. The focused failing probes are additional checks; they are not failures in the existing 567-test frontend suite.

From `/Users/chris/projects/spark`, rerun the API probe with:

```sh
python3 reports/CR-2026-0117-audit/api_probe.py .spark/checkouts/run-18d45752f4ec6770/target/debug/spark-server
```

The printed successful saves and subsequent bad states demonstrate the defects. The probe itself exits successfully when it obtains the observations; it is an observational reproduction rather than a fixed regression test.

Rerun the Desktop assertion after building the all-features worktree libraries:

```sh
python3 reports/CR-2026-0117-audit/run_desktop_probe.py
```

Rerun the component assertion:

```sh
.spark/checkouts/run-18d45752f4ec6770/frontend/node_modules/.bin/vitest run --config reports/CR-2026-0117-audit/vitest.config.mjs
```

Both assertion probes fail on the audited implementation. No real Desktop window, agent, or browser is started by them. The small runner/config files use the audited worktree path so they exercise those files rather than the main checkout.

Browser access to the isolated audit server was denied by automatic approval review. No browser workaround was attempted. Container findings are based on inspected dispatch/capture/environment code and need a real container check after correction.
