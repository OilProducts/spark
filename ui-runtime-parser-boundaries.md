# Runtime/Parser Boundaries for UI Work

Checklist item: [1.3-01]

This document defines runtime/parser boundaries so UI implementation work remains aligned with `ui-spec.md` and `attractor-spec.md`.

## Source of Truth

- `ui-spec.md` section `1.3 Non-Goals` explicitly includes: "Replacing the DOT runtime parser or executor".
- `attractor-spec.md` remains authoritative for DOT grammar, parse behavior, and execution semantics.

## UI-owned responsibilities

- Collect user input through structured forms and raw DOT editing surfaces.
- Preserve user intent during save/load by serializing valid DOT without dropping supported attributes.
- Surface parser/runtime diagnostics, validation errors, and execution state to operators.
- Prevent UI-only defaults from silently changing configured behavior.

## Runtime/parser-owned responsibilities

- Parse DOT and normalize graph structure.
- Resolve runtime semantics for handlers, routing, retries, checkpointing, and execution events.
- Enforce grammar/runtime constraints and return canonical diagnostics.
- Execute pipeline logic and state transitions.

## Boundary Rules

1. UI layers may format or validate input, but do not reinterpret execution semantics.
2. UI changes must not modify parser outputs or runtime decision logic.
3. Feature parity work may expose existing capabilities, but no runtime parser changes are allowed for UI-only tasks.
4. When UI validation differs from parser validation, parser/runtime behavior wins and must be surfaced verbatim.
5. Any required semantic change must be proposed and implemented in parser/runtime code first, then reflected in UI.
