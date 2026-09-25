# Ponytail: Minimal Effective Engineering

Build the smallest correct solution that satisfies the real requirement. Prefer existing, boring, well-understood mechanisms over new code, new abstractions, or new dependencies.

Apply this to every coding task unless the user explicitly asks for a fuller implementation.

## The Ladder

Before writing code, understand the task and trace the real flow end to end.  Reason about the need from first principles, what actually is required to solve the problem at hand.  Then stop at the first rung that holds:

1. Does this need to exist at all? If it is speculative, skip it and say so.
2. Does this already exist in the codebase? Reuse the existing helper, util, type, or pattern.
3. Does the standard library do this? Use it.
4. Does a native platform feature cover it? Use it.
5. Does an already-installed dependency solve it? Use it.
6. Can it be one line? Make it one line.
7. Only then, write the minimum code that works.

If two rungs both work, take the higher one.

## Bug Fixes

Fix root causes, not symptoms.  Reason about the problem from first principles.

Before editing a function, grep its callers. Prefer one fix in the shared path over scattered guards in each call site. A tiny patch in the wrong place is not a minimal solution; it is a second bug.

## Rules

- No unrequested abstractions.
- No interface with one implementation.
- No factory for one product.
- No config for a value that never changes.
- No boilerplate or scaffolding “for later.”
- No new dependency when stdlib, platform features, existing code, or a few clear lines will do.
- Prefer deletion over addition.
- Prefer boring code over clever code.
- Touch the fewest files possible.
- Shortest working diff wins, after understanding the problem.
- If two stdlib options are equally small, choose the one with better edge-case behavior.
- For complex requests, ship the simpler useful version and state what was skipped and when to add it.
- Mark deliberate simplifications with a `ponytail:` comment when the shortcut has a known ceiling.

Example comments:

```python
# ponytail: O(n^2) is fine for current list sizes; index by id if this grows.
```

```js
// ponytail: global lock; use per-account locks if throughput matters.
```

## Checks

Minimal code without verification is unfinished.

For non-trivial logic, leave a runnable check: a small test, an assert-based demo, or a `__main__` self-check. Test in proportion to risk: cover each behavior the change could plausibly break, at the boundary where it's observable. Don't restate the implementation, duplicate an existing check, or add scaffolding (new frameworks, one-off fixtures) the check doesn't need. Keep checks fast and deterministic.

Trivial one-liners do not need tests.

## Do Not Simplify Away

Never remove or weaken:

- Input validation at trust boundaries.
- Error handling that prevents data loss.
- Security controls.
- Accessibility basics.
- Calibration or tuning needed for real hardware.
- Anything the user explicitly requested after you raised the simpler option.

## Output Style

For coding tasks: code first. Then, if useful, at most a few short lines:

`skipped: [what was omitted], add when [condition]`

Do not write long design notes unless the user explicitly asks for explanation, review, or a report.

## This repo

### Validation

- `just test` is the full gate: fmt check, all Rust tests, frontend unit tests, frontend build. It takes several minutes after a rebuild of core crates.
- While iterating, run only the affected tests. Scope Rust test commands like the full gate so they reuse its build: `cargo test --workspace --all-features --test <name> [filter]`. `cargo test -p <crate>` builds a different feature set and recompiles.
- Run `just test` once before handing work off, in the foreground, and report the result.
- When reviewing, rerun `just test` only if the tree changed after the last green run you can verify. Otherwise run targeted checks for the areas you are judging.

### Where tests go

Add a Rust test as a module under the crate's existing `tests/contracts/` or `tests/process_contracts/`. Do not add new top-level files under `tests/`: each becomes its own test binary to link and launch.

### Change requests

Change requests live in `changes/CR-YYYY-NNNN-<slug>/` as `request.md` and `result.md`. Number a new one as the highest existing number plus one, taken from the full, untruncated directory listing. `changes/` is gitignored; a change request is tracked only when `.gitignore` un-ignores it explicitly.

### Browser smoke tests

`npm --prefix frontend run ui:smoke` serves the built `frontend/dist`. Run `npm --prefix frontend run build` first, or the smoke tests exercise stale code.

### Flows

Runs use the installed flows in `$SPARK_HOME/flows`, which is outside this repo and not tracked by git. `crates/spark-assets/assets/flows` holds the bundled copies. Some installed flows, such as `software-development/implement-change-request.yaml`, exist only in `$SPARK_HOME/flows`.
