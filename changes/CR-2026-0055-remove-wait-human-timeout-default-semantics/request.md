# Remove `wait.human` Timeout/Default Semantics

## Summary
Make `wait.human` a pure indefinite human gate and remove unused timeout/default infrastructure from the interviewer model. A human gate should route only after an explicit human answer; Spark should not keep dormant timeout/default code for hypothetical future use.

## Key Changes
- Remove `Question.default` and `Question.timeout_seconds` from the interviewer model.
- Remove `AnswerValue.TIMEOUT` if no remaining real code path uses it after cleanup.
- Remove `human.default_choice` from the supported `wait.human` authoring contract.
- `wait.human` behavior becomes:
  - explicit selected answer routes to the matching outgoing edge
  - skipped/no usable answer fails as `human skipped interaction`
  - no outgoing edges fails
  - no timeout/default route exists

## Implementation
- Simplify `WaitHumanHandler`:
  - stop checking for timeout answers
  - remove `_default_choice`
  - remove `InterviewTimeout` emission
  - remove context/default-route behavior tied to `human.default_choice`
- Simplify interviewer/runtime plumbing:
  - remove `default` and `timeout_seconds` fields from `Question`
  - remove broker timeout waiting and default fallback in `HumanGateBroker`
  - remove timeout serialization/deserialization from execution-container question payloads
  - remove any timeout/default-only tests and fixtures
- Remove first-class `human.default_choice` authoring:
  - frontend inspector field
  - canvas toolbar field
  - canonical serializer owned attr
  - contract tests and raw round-trip expectations that treat it as supported
- Update docs/spec/TODO:
  - Attractor spec Section 4.6: `wait.human` waits for explicit selection
  - Attractor spec Section 6: remove timeout/default fields and timeout handling from the generic interviewer model
  - Spark DOT authoring guide: remove `human.default_choice`
  - TODO: mark the wait-human timeout/default drift as resolved by narrowing/removing the feature

## Tests
- Remove tests that only prove `Question.default`, `Question.timeout_seconds`, `AnswerValue.TIMEOUT`, or `human.default_choice` timeout routing.
- Add/keep tests for:
  - `wait.human` builds choices from outgoing edges
  - explicit selected option routes correctly
  - skipped/empty answer fails
  - frontend no longer shows or serializes `human.default_choice`
  - container question payloads no longer include timeout/default fields
- Run focused tests:
  - `uv run pytest -q -x --maxfail=1 tests/handlers/test_wait_human_handler.py tests/interviewer tests/api/test_human_gate_broker.py tests/handlers/test_execution_container.py tests/contracts/frontend`
  - relevant frontend editor/canonical model tests
- Final verification:
  - `uv run pytest -q`

## Assumptions
- This is a hard cleanup; no compatibility shim for `human.default_choice`.
- Timed generic interviewer questions are not part of Spark’s current product behavior.
- Any existing authored flow using `human.default_choice` should be updated rather than supported silently.
