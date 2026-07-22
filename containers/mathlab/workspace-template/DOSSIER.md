# Research Dossier

This file is the problem's memory across sessions. Every flow reads it
before doing anything else and appends a session entry before committing.
Entries are append-only: never rewrite or delete prior content — a failed
attack recorded here is a result, and re-running one blind wastes the
session's budget.

## Problem

(One precise statement of the problem this workspace exists to attack,
with definitions pinned. Revise only by appending numbered statement
versions below, each with the exact change and the evidence that forced
it.)

## Status board

- Current statement version: v1
- Verdict so far: open
- Best verified evidence: none yet
- Smallest open case: unknown

## Established results

(Claims with their labels per AGENTS.md — new proof, formalization of a
known result, located in literature, or computational evidence — each with
a pointer to the artifact or commit that backs it.)

## Failed attacks

(What was tried, why it failed, and what the failure taught. This is the
most valuable section; keep it honest and specific.)

## Verified data

(Computations with their exact scope — parameters covered, cases
exhausted — and where the certificates, programs, and output digests
live.)

## Open subproblems

(Ranked. Include the current stuck lemma and the smallest open case.)

## Session log

(One entry per run, appended at session end: flow used, statement version
attacked, outcome, and the artifacts produced.)
