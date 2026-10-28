---
type: Decision
title: Git branching model
id: ADR-0002
status: Accepted
date: 2026-10-28
---

# ADR-0002: Git branching model

## Status

Accepted

## Context

This decision belongs to the `software` profile (ADR-0006). The model has to
work for the project, which an organization changes through flow runs, and for
the org repo, which the Chief of Staff changes directly.

How Spark's flows work today: each change gets its own branch,
`spark/implement-change/run-<id>`, in a separate worktree. The flow reviews the
work on that branch, and `merge-change` merges it into `main` once it passes the
validation gate. Nothing is released from a long-lived branch.

That is GitHub flow, minus the hosting platform. GitHub describes GitHub flow as
"a lightweight, branch-based workflow": create a branch, make changes, open a
pull request, address review comments, merge, delete the branch ("Your changes
will not end up on the default branch until you merge your branch"). Spark keeps
the unit (one branch per change, reviewed before merge) and replaces the pull
request with the flow's own isolated review and gate.

Trunk-based development is different in its defining practice: developers
"collaborate on code in a single branch called 'trunk'", with team members
committing to it "at least once every 24 hours". Short-lived branches are
allowed when scaled, but the point is integration frequency, not review per
branch. Spark's runs are short-lived, but review per branch is what defines
them.

Candidates:

| Model | Shape | Fits |
|---|---|---|
| git-flow (Driessen, "A successful Git branching model", 2010) | Long-lived `main` and `develop`, plus feature, release and hotfix branches | Software that is "explicitly versioned", or that must "support multiple versions … in the wild". In a note dated 2020-03-05, its author advises teams doing continuous delivery to "adopt a much simpler workflow (like GitHub flow)". |
| GitHub flow | One branch per change, reviewed, then merged into the default branch | Continuous delivery with review per change; what Spark's flows already do |
| Trunk-based development | Everyone commits to `main` at least daily; branches, if any, last hours | Teams with very strong automated checks and frequent small commits |

Release tagging with SemVer (ADR-0001) works with any of them.

## Decision

GitHub flow.

- Every change gets its own branch, is reviewed on it (in-flow review stands in
  for a pull request where there is no hosting platform), and is merged into
  `main` only after passing the validation gate. `main` stays releasable.
- Nothing is committed directly to `main`.
- Releases are SemVer tags on `main`, with the changelog's `Unreleased` section
  moved under the new version.
- git-flow's release and hotfix branches only for a project that must support
  more than one released version at once.

This applies to the project. The org repo is changed by the Chief of Staff
directly, with every change committed with its reason (ORG-031).

## Consequences

- Every change to the project, including documentation and records, reaches
  `main` through a reviewed branch. Direct commits to `main`, which have
  happened before this decision, are no longer made.
- `main` is always releasable, so a release is a tag plus a changelog update,
  not a branch.
- Spark's existing flows already work this way. Changes made outside a flow need
  a branch and a review too, for example by running `merge-change` on a branch.

## Source

Operator, 2026-10-28: "I thought GitHub flow was more similar to how we're
working, rather than trunk based." The first draft of this record had proposed
trunk-based development; it was corrected to GitHub flow after checking both
definitions at their sources:
<https://docs.github.com/en/get-started/using-github/github-flow> and
<https://trunkbaseddevelopment.com/>. Accepted by the operator, 2026-10-28:
"I approve ADR-0002 as written."
