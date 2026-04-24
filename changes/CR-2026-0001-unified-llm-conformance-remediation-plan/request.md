# Unified LLM Conformance Remediation Plan

## Summary
Address the three audit findings by making the implementation match the original spec where it is normative, and by removing one misleading derived-artifact promise that is optional in the original spec. The remediation should be code and test changes only; do not rewrite the original spec.

## Key Changes
- **OpenAI tool payloads:** update `OpenAIAdapter` request translation so SDK `Tool` definitions serialize as `{"type":"function","function":{...}}`, with `name`, `description`, and `parameters` nested under `function`; update named `ToolChoice` to `{"type":"function","function":{"name":"..."}}`.
- **Unified data model validation:** enforce known `ContentKind` tagged-union invariants in `ContentPart` construction:
  - `TEXT` requires only `text`; `IMAGE` only `image`; `AUDIO` only `audio`; `DOCUMENT` only `document`; `TOOL_CALL` only `tool_call`; `TOOL_RESULT` only `tool_result`; `THINKING`/`REDACTED_THINKING` only `thinking`.
  - `REDACTED_THINKING` requires `ThinkingData.redacted is True`; `THINKING` requires `redacted is False`.
  - Custom string content kinds remain extension-friendly and are not subject to these known-kind payload rules.
- **Message role constraints:** enforce original spec direction constraints in `Message.__post_init__` for known content kinds:
  - `TEXT`: system/user/assistant/developer/tool
  - `IMAGE`: user/assistant
  - `AUDIO` and `DOCUMENT`: user only
  - `TOOL_CALL`, `THINKING`, `REDACTED_THINKING`: assistant only
  - `TOOL_RESULT`: tool only
- **Provider normalizers:** adjust any provider response normalization that currently builds thinking parts with both `text` and `thinking`, so it creates valid `ContentPart(kind=THINKING, thinking=ThinkingData(...))`.
- **Anthropic structured output:** remove advertised forced-tool fallback/support. Keep schema-instruction as the only Anthropic structured-output strategy, and reject explicit `strategy="forced-tool"` as unsupported without advertising it from `structured.py` or derived architecture-facing docs.

## Test Plan
- Update OpenAI adapter tests to expect nested `function` tool definitions and named tool choice.
- Add public data-model tests proving invalid tagged-union payload combinations and invalid role/kind message combinations raise at construction time.
- Add regression tests that provider response normalization still returns valid text, tool-call, tool-result, and thinking content parts after stricter constructors.
- Update structured-output tests so Anthropic provider options no longer include `fallback: "forced-tool"` and explicit forced-tool strategy remains rejected.
- Run required gates: `uv run pytest -q`, `uv run ruff check .`, and `uv build`.

## Assumptions
- The original spec controls over derived artifacts.
- Breaking permissive construction of invalid public `ContentPart`/`Message` objects is acceptable because those objects contradicted the original spec.
- Anthropic tool-based extraction remains out of scope because the original spec presents it as an alternative, not mandatory, strategy.
