---
type: Standard
title: Writing requirements and decisions
---

# Writing requirements and decisions

How an agent org writes its spec, its decisions and any requirements it produces.
This document is written in our own words. Where it follows a published standard,
it says so; ISO/IEC/IEEE texts are copyrighted and are not vendored.

## Normative keywords (BCP 14)

Every document that states requirements begins with this sentence, which is the
boilerplate from [RFC 8174](rfc8174.txt):

> The key words "MUST", "MUST NOT", "REQUIRED", "SHALL", "SHALL NOT", "SHOULD",
> "SHOULD NOT", "RECOMMENDED", "NOT RECOMMENDED", "MAY", and "OPTIONAL" in this
> document are to be interpreted as described in BCP 14 [RFC2119] [RFC8174] when,
> and only when, they appear in all capitals, as shown here.

- Use the strength you mean. MUST and SHALL are absolute. SHOULD allows a
  justified, recorded exception. MAY is genuinely optional. A rule that is a
  judgment call is SHOULD, not MUST.
- Only the capitalised forms carry the meaning ([RFC 8174](rfc8174.txt)).
- [RFC 2119](rfc2119.txt) asks that the keywords be used sparingly, where they
  are actually required. Explanatory text uses ordinary language.

## A single requirement

These qualities follow ISO/IEC/IEEE 29148 ("Systems and software engineering —
Life cycle processes — Requirements engineering"). A requirement is:

- **Necessary**: removing it would leave a real need unmet.
- **Singular**: it states one thing. Three "and also" clauses are three
  requirements.
- **Unambiguous**: it has one reasonable reading.
- **Feasible**: it can be met with the means available.
- **Verifiable**: someone who did not write it can tell whether it holds.
- **Correct and traceable**: it reflects what its source asked for, and names
  that source.

Each requirement has:

| Field | Content |
|---|---|
| ID | Stable, never reused or renumbered, for example `ORG-012`. A withdrawn requirement keeps its ID and is marked withdrawn. |
| Statement | One sentence with one BCP 14 keyword. |
| Rationale | Why it exists. |
| Source | Where it came from: an operator quote (with date), a decision record, or "proposed" with its author. A requirement an agent proposed is never attributed to the operator. |
| Verification | One of the four methods below. |
| Scenarios | One or more WHEN/THEN scenarios, required when the verification method is Demonstration or Test. |

### Verification methods

From 29148 and common systems-engineering practice:

- **Inspection**: examine the artifact (a file exists, a section is present).
- **Analysis**: reason from evidence (logs, run records, diffs).
- **Demonstration**: operate the system and observe the behaviour.
- **Test**: run a defined procedure with expected results and record the outcome.

A requirement is not met until its verification has been carried out and
recorded. A documented capability is not an operational one.

### Statement and scenario patterns

Statements follow the EARS patterns (Mavin et al., "Easy Approach to
Requirements Syntax", 2009) where they fit:

| Pattern | Form |
|---|---|
| Ubiquitous | The <system> SHALL <response>. |
| Event-driven | WHEN <trigger>, the <system> SHALL <response>. |
| State-driven | WHILE <state>, the <system> SHALL <response>. |
| Unwanted behaviour | IF <condition>, THEN the <system> SHALL <response>. |
| Optional feature | WHERE <feature is present>, the <system> SHALL <response>. |

Scenarios use WHEN/THEN:

```markdown
#### Scenario: Ambiguous target
- **WHEN** the user asks to "improve our testing"
- **THEN** the Coordinator asks whether that means the project's tests, the agents' testing procedures, or both, before starting any work
```

### A set of requirements

The set is complete (it covers what was asked), consistent (no two requirements
conflict) and comprehensible. Whoever changes a requirement checks the rest of
the set for conflicts in the same change.

## Stakeholder needs versus requirements

29148 separates stakeholder requirements, meaning what the stakeholder said,
from the requirements derived from them.

- The **operator brief** records the operator's own words, quoted and dated.
- The **spec** holds derived requirements, and each one cites its source.
- Anything an agent infers or proposes names the role that proposed it, and is
  never attributed to the operator.

## Decisions

Decisions are recorded as Architecture Decision Records (after Michael Nygard,
"Documenting Architecture Decisions", 2011), one file each, numbered and never
renumbered:

- **Status**: Proposed, Accepted, Rejected, Deprecated, or Superseded by ADR-NNNN.
- **Context**: the forces at play, facts only.
- **Decision**: what was decided, in active voice.
- **Consequences**: what becomes easier and harder, including costs.
- **Source**: who decided, quoted and dated if it was the operator.

An accepted decision is changed by writing a new decision that supersedes it,
not by editing it.

## Responsibilities

The spec assigns responsibilities with a RACI table: for each activity, who is
**R**esponsible (does it), **A**ccountable (owns the outcome), **C**onsulted and
**I**nformed. Each activity has exactly one A. Adding or removing a standing role
updates the table in the same change.
