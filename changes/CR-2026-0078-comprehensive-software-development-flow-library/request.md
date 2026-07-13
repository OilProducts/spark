# Comprehensive Software-Development Flow Library

## Summary

Build a curated library of ten user-facing, intent-oriented flows backed by shared hidden primitives. Mutating runs execute on isolated Git branches and worktrees, while read-only flows operate against a selected commit or checkout. Existing flow names and contracts remain stable wrappers.

Deliver incrementally: foundation, read-only flows, mutating flows, then local integration.

## Flow Catalog

1. **Explore Codebase** — Map architecture, execution paths, conventions, and relevant files without editing.
2. **Audit Codebase Health** — Produce prioritized, evidence-backed correctness, maintainability, testing, security, and performance findings.
3. **Investigate Bug** — Reproduce or characterize a failure, identify the root cause, and produce a repair-ready diagnosis.
4. **Plan Change** — Normalize a prompt or artifact into a decision-complete implementation plan with acceptance criteria and validation.
5. **Implement Change** — Plan, implement, test, review, and commit a bounded feature, fix, refactor, test, or documentation change.
6. **Implement Spec Program** — Preserve the existing large-work flow, using milestones and shared implementation primitives.
7. **Review Change** — Review a diff or branch and record findings; optionally create a separate fix branch, apply accepted high-confidence fixes, validate, and commit them.
8. **Repair Validation** — Diagnose failing tests, builds, linters, or checks; repair the underlying behavior and commit the validated result.
9. **Update Dependencies** — Update selected dependencies and lockfiles, inspect compatibility and security implications, run validation, and commit the result.
10. **Merge Change** — Locally integrate a completed Spark branch into a clean target branch, resolve bounded conflicts, validate, and create an explicit merge commit.

Keep **Implement Change Request** as a stable launcher whose existing artifact contract delegates to Implement Change behavior.

## Foundation and Shared Primitives

- Introduce a common task contract accepting either an inline objective or repository-relative artifact, plus optional target paths, constraints, acceptance criteria, validation command, base ref, and Git-result settings.
- Normalize every run into durable state under `.spark/software-development/runs/<run-id>/`; keep these artifacts uncommitted unless a flow explicitly produces an existing durable spec or change-request record.
- Add runtime-managed execution workspace policies:
  - `read_only`: inspect the selected checkout/ref without mutation.
  - `isolated_branch`: create `spark/<flow-id>/<run-id>` from committed HEAD or the selected base ref in a Spark-managed external worktree.
  - `repository_integration`: acquire an exclusive repository lock for merge operations.
- Identify repositories through Git’s canonical common directory so all linked worktrees share the same integration lock and run registry.
- Seed isolated runs from committed state only. Warn that source-checkout changes are excluded; never stash, copy, or commit the developer’s dirty state.
- Default successful bounded flows to one validated commit. Spec implementation may commit once per completed milestone.
- Record branch, base commit, worktree, commits, validation evidence, result summary, and base divergence in the durable run result.
- On success, remove the run worktree but retain its branch. Preserve failed or blocked worktrees for diagnosis.
- Report base-branch drift without automatically rebasing.

Hidden reusable worker flows should cover task normalization, repository inspection, isolation preparation, planning, implementation, validation discovery/execution, independent review, result recording, commit finalization, and cleanup.

## Merge Behavior

- Permit only one Merge Change run per canonical Git repository at a time.
- Require the target checkout to be clean; never auto-stash user changes.
- Create a temporary integration branch and worktree from the target, merge the source there, resolve only behavior-preserving conflicts, and run the repository’s full validation gate.
- After successful validation, fast-forward the clean target checkout to the validated merge commit.
- Remove the integration and source worktrees on success. Retain both branches by default.
- Preserve the integration worktree and stop when conflicts require product judgment, validation fails, the target moves during integration, or the target checkout becomes dirty.

## Delivery Sequence

1. Add canonical repository identity, repository-scoped locking, worktree lifecycle management, common task/result contracts, and catalog workspace policies.
2. Add Explore Codebase, Audit Codebase Health, Investigate Bug, and Plan Change.
3. Add Implement Change, Review Change, Repair Validation, and Update Dependencies; refactor existing launchers onto the shared primitives.
4. Add Merge Change and integration-worktree handling.
5. Enable all ten launchers by default while keeping worker flows non-requestable.

## Test Plan

- Validate every authored flow against the flow schema and verify only launcher flows are agent-requestable.
- Test inline and artifact inputs normalize to equivalent durable task state.
- Test multiple mutating runs from one repository receive distinct branches and worktrees and can execute concurrently.
- Verify dirty source changes are excluded and never modified, stashed, or committed.
- Verify successful runs commit, record evidence, report base divergence, and clean their worktrees; failed runs remain inspectable.
- Verify review-only runs do not mutate and optional fixes are made on a new isolated branch.
- Verify dependency updates preserve manifest/lockfile consistency and exercise the repository validation gate.
- Verify repository identity and merge locking work across linked Git worktrees.
- Test clean merges, bounded conflict resolution, validation failure, target movement, dirty targets, and failed integration preservation.
- Run the repository’s full required validation gate before completion:
  - `cargo fmt --all -- --check`
  - `cargo test --workspace --all-features`
  - `npm --prefix frontend run test:unit`
  - `npm --prefix frontend run build`

## Assumptions

- The library covers the code lifecycle but excludes pushing, pull-request creation, deployment, rollback, and incident operations.
- Human input is requested only for material ambiguity, risky behavioral decisions, or conflicts that cannot be resolved safely.
- Documentation, test improvement, performance work, security remediation, features, fixes, and refactors use Implement Change unless their orchestration matches another specialized flow.
- Existing Implement Change Request and Implement Spec Program names and input contracts remain compatible.
- Merge Change creates an explicit merge commit and does not delete source branches automatically.
