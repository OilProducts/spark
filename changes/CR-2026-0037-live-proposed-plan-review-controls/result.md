---
id: CR-2026-0037-live-proposed-plan-review-controls
title: Live Proposed Plan Review Controls
status: completed
type: bugfix
changelog: public
---

## Summary
Completed the live proposed-plan review control fix. Streamed `segment_upsert` events can now carry the durable artifact record for the segment being updated, so plan review controls can appear from the live stream without waiting for a follow-up full snapshot refresh.

## Validation
Ran `uv run pytest -q`.

Result: 1964 passed, 26 skipped, 2 warnings.

## Shipped Changes
Backend conversation segment event construction now attaches only the matching sidecar artifact for plan, flow run request, and flow launch segments that have an `artifact_id`.

Frontend stream event parsing, cache merge logic, and stream refresh fallback handling now accept artifact sidecars, merge them into normalized artifact state, rebuild the affected timeline row, and avoid snapshot refresh when the matching sidecar is already present.

Tests were updated across backend API behavior, stream parser coverage, project home state reducer coverage, and ProjectsPanel live review behavior. The old refresh-dependent plan review expectation was removed while snapshot recovery behavior remains covered.
