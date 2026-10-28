---
type: Decision
title: Vendor established standards
id: ADR-0001
status: Accepted
date: 2026-10-28
---

# ADR-0001: Vendor established standards

## Status

Accepted

## Context

The seed's first draft was a single 65 KB installation prompt that invented its
own conventions for requirements, records and change tracking. Human engineering
organizations faced the same problems long before LLMs and settled on documented
standards for them.

## Decision

The seed adopts established standards where they are reasonable. It vendors them
into `standards/`, in full or in part, with their provenance and licenses:

- BCP 14 (RFC 2119 and RFC 8174) for requirement levels;
- Semantic Versioning 2.0.0 for versioned software, including the seed itself;
- Keep a Changelog 1.1.0 for changelogs;
- Make a README for READMEs.

Copyrighted standards such as ISO/IEC/IEEE 29148 are cited and paraphrased, not
copied. The git branching model is still open (ADR-0002).

## Consequences

- Agents and people get conventions they may already know, with the source text
  at hand.
- Vendored copies have to be updated deliberately; the standards README records
  each source and its version.
- The seed carries about 35 KB of reference text that is read on demand, not
  pinned into prompts.

## Source

Operator, 2026-10-28: "Lets start a draft of our own agent org setup, I'd like to
stick to well tested standards when reasonable, that means 2119 compliant where
appropriate, semantic versioning for software, git flow or some (tbd) git
management that helps streamline, the "Keep a changelog" and "make a readme"
industry standards, etc.  I want to bring those in to the draft now, either in
full or in part, essentially "vendoring" the parts we want."
