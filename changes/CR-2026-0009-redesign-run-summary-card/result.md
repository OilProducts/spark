---
id: CR-2026-0009-redesign-run-summary-card
title: Redesign Run Summary Card
status: completed
type: feature
changelog: public
---

## Summary
Delivered the Runs tab selected-run summary redesign as an operator-oriented panel focused on current activity, outcome, scope, and usage. The implementation is frontend/spec only and does not change backend schemas or transport.

## Validation
- `npm --prefix frontend run test:unit -- RunsPanel` passed with 17 tests.
- `npm --prefix frontend run build` passed; Vite reported the existing large chunk warning.
- `uv run pytest -q` passed with 1679 tests and 26 skipped.

## Shipped Changes
- `frontend/src/features/runs/components/RunSummaryCard.tsx` now renders ordered `Now`, `Outcome`, `Scope`, and `Usage` sections, keeps run actions in the header area, groups per-model usage under Usage, compacts git and lineage details, and hides empty or low-value diagnostic rows unless relevant.
- `frontend/src/features/runs/RunsPanel.tsx` now feeds the summary a count of completed nodes and a waiting signal when pending operator questions are visible.
- `frontend/src/features/runs/__tests__/RunsPanel.test.tsx` and `frontend/e2e/smoke/runs-observability.spec.ts` now assert the new section-level summary contract, conditional working-directory display, compact lineage, failure reason handling, live activity facts, and usage breakdown behavior.
- `specs/spark-ui-ux.md` documents the new selected-run summary guidance and the working-directory display rule.
