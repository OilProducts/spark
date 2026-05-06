# Flow Run Results

## Summary

Add a first-class “run result” concept so completed flows have a reliable output surface outside the current artifact/journal browsing path.

The result source is configurable per flow, can optionally be summarized with a canned or custom prompt, and falls back to the last successful non-terminal node before `done` when no explicit source is configured.

## Key Changes

- Add supported graph attributes:
  - `spark.result_node`: optional node id whose response should be treated as the flow result source.
  - `spark.result_summary_enabled`: optional boolean string, default `false`.
  - `spark.result_summary_prompt`: optional custom summarization prompt; when omitted and summaries are enabled, use Spark’s default result-summary prompt.
- Result source selection:
  - Prefer `spark.result_node` when set and valid.
  - Otherwise infer the source from the final successful work node before an `Msquare` exit node.
  - Ignore `Mdiamond` start nodes and `Msquare` exit nodes.
  - If no source can be found, mark the result as unavailable instead of failing the run.
- Result materialization:
  - After a run reaches a terminal state, write `result/result.json` and `result/result.md` under the run root.
  - If summarization is disabled, `result/result.md` contains the selected source node’s raw `response.md`.
  - If summarization is enabled, run the configured/default summary prompt against the selected source response and store the summary in `result/result.md`.
  - Preserve the raw source by recording its artifact path in metadata; do not overwrite node `response.md`.

## API And UI

- Add `GET /attractor/pipelines/{pipeline_id}/result`.
  - Return run id, status, result state, source node id, source artifact path, whether the displayed body is raw or summary, and `body_markdown`.
  - Return `pending` for active or queued runs, `unavailable` when no result source exists, and `error` only when result resolution itself fails.
- Add a “Result” panel/card in the Runs view above artifacts.
  - Show the summarized or raw result body.
  - Include a small link/action to open the source artifact when available.
- Add result controls in flow graph settings.
  - Result node selector populated from graph nodes.
  - Summary enabled toggle.
  - Summary prompt textarea shown only when summary is enabled.
- Add a “View result” affordance to conversation flow-launch cards when a run id exists.
  - It should fetch and display the same result endpoint, not duplicate result logic.

## Test Plan

- Backend tests:
  - Explicit `spark.result_node` selects the configured node response.
  - Missing `spark.result_node` falls back to the last successful non-terminal predecessor of `done`.
  - Summary disabled returns raw source response.
  - Summary enabled writes and returns summarized output using a stub backend.
  - Missing/invalid result source returns `unavailable` without changing terminal run status.
- Frontend tests:
  - Graph settings round-trip the new graph attrs.
  - Runs panel renders pending, unavailable, raw result, and summarized result states.
  - Conversation flow-launch card exposes result access for launched runs.
- Contract/docs:
  - Update DOT authoring guidance for the three new graph attrs.
  - Run the relevant focused tests first, then `uv run pytest -q` before completion.

## Assumptions

- No CLI result command in v1; the API/UI path is the product fix. A future `spark run result --run <id> --text` can reuse the same endpoint.
- Summarization is opt-in by default to preserve existing flow behavior and avoid hidden extra model work.
- Summary generation failures should degrade to the raw result and record the summary error in result metadata.
