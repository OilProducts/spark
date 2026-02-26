# Attractor Implementation Checklist

Companion to `/Users/chris/tinker/sparkspawn/attractor-spec.md`.

Use this as the execution plan and verification ledger for full spec coverage. Tasks are ordered to match the spec section/subsection sequence.

Status key:
- `[ ]` not started
- `[~]` in progress
- `[x]` complete

---

## 1. Overview and Goals

### 1.1 Problem Statement
- [x] [1.1-01] Implement at least one reference workflow that chains multiple LLM stages with conditions, human approval, and parallel execution.
- [x] [1.1-02] Verify workflows are defined as graph structure (nodes/edges/attrs), not imperative control flow code.
- [x] [1.1-03] Add a deterministic replay test proving identical routing given identical outcomes/context.

### 1.2 Why DOT Syntax
- [x] [1.2-01] Restrict ingestion to DOT `digraph` workflows and reject unsupported DOT features.
- [x] [1.2-02] Add visualization export path (DOT -> Graphviz render artifact) for operator inspection.
- [x] [1.2-03] Validate `.dot` files are diff-friendly in CI (format/lint check with stable ordering).

### 1.3 Design Principles
- [x] [1.3-01] Enforce declarative execution model: engine chooses runtime flow from graph + outcomes.
- [x] [1.3-02] Ensure handler system is pluggable and registry-driven.
- [x] [1.3-03] Persist checkpoint after each stage and support resume from checkpoint.
- [x] [1.3-04] Support human-gate decision points with external interviewer implementations.
- [x] [1.3-05] Keep routing edge-driven (conditions/labels/weights), not handler-internal branching.

### 1.4 Layering and LLM Backends
- [x] [1.4-01] Keep orchestration layer backend-agnostic via `CodergenBackend` interface.
- [x] [1.4-02] Ensure pipeline definition is backend-invariant (switch backend without DOT changes).
- [~] [1.4-03] Emit runtime events suitable for TUI/web/IDE consumers with no UI coupling in engine.

---

## 2. DOT DSL Schema

### 2.1 Supported Subset
- [x] [2.1-01] Accept only one directed graph per file (`digraph`).
- [x] [2.1-02] Reject HTML-like labels and other out-of-subset constructs.
- [x] [2.1-03] Enforce typed attributes with defaults at parse/normalize stage.

### 2.2 BNF-Style Grammar
- [x] [2.2-01] Implement parser productions for graph statements (`graph/node/edge/subgraph/attr decl`).
- [x] [2.2-02] Support chained edge syntax (`A -> B -> C`) with optional trailing attr block.
- [x] [2.2-03] Parse attribute blocks with comma-separated key/value pairs and qualified keys.
- [x] [2.2-04] Support value lexing for string/integer/float/boolean/duration.
- [x] [2.2-05] Add parser tests for optional semicolons and mixed statement ordering.

### 2.3 Key Constraints
- [x] [2.3-01] Reject multiple graphs, undirected graphs, and `strict` graphs.
- [x] [2.3-02] Enforce node ID regex `[A-Za-z_][A-Za-z0-9_]*`.
- [x] [2.3-03] Enforce comma separation inside attr blocks.
- [x] [2.3-04] Strip `//` and `/* */` comments before parse.

### 2.4 Value Types
- [x] [2.4-01] Parse quoted strings with escapes (`\\n`, `\\t`, escaped quotes, escaped backslash).
- [x] [2.4-03] Parse booleans as typed values.
- [x] [2.4-04] Parse durations with units (`ms|s|m|h|d`) and normalize representation.

### 2.5 Graph-Level Attributes
- [x] [2.5-01] Implement `goal` extraction and mirror to context key `graph.goal`.
- [x] [2.5-02] Implement `label` as graph display metadata.
- [x] [2.5-03] Parse and validate `model_stylesheet` text.
- [x] [2.5-04] Apply `default_max_retry` fallback for nodes without `max_retries`.
- [x] [2.5-05] Implement graph-level `retry_target` and `fallback_retry_target` for goal-gate recovery.
- [x] [2.5-06] Implement `default_fidelity` fallback in fidelity resolution.

### 2.6 Node Attributes
- [x] [2.6-01] Support and persist all node attrs from spec table.
- [x] [2.6-03] Implement prompt fallback to `label` for LLM stages.
- [x] [2.6-04] Implement `max_retries` semantics as additional attempts.
- [x] [2.6-05] Enforce `goal_gate` tracking and exit blocking behavior.
- [x] [2.6-06] Implement node-level retry target fallback chain.
- [x] [2.6-07] Implement fidelity/thread/session attrs (`fidelity`, `thread_id`).
- [x] [2.6-08] Parse and apply stylesheet `class` selectors (comma-separated classes).
- [x] [2.6-09] Enforce `timeout` in handler execution paths.
- [x] [2.6-10] Resolve model attrs (`llm_model`, `llm_provider`, `reasoning_effort`) with precedence rules.
- [x] [2.6-11] Implement `auto_status` status synthesis behavior.
- [x] [2.6-12] Implement `allow_partial` behavior when retries exhaust.

### 2.7 Edge Attributes
- [x] [2.7-02] Parse/evaluate edge `condition` expressions.
- [x] [2.7-04] Implement edge-level `fidelity` and `thread_id` overrides for target node execution.
- [x] [2.7-05] Implement `loop_restart` semantics to relaunch run with fresh logs.

### 2.8 Shape-to-Handler-Type Mapping
- [x] [2.8-01] Implement canonical shape->type mapping exactly as specified.
- [x] [2.8-03] Add mapping coverage tests for all listed shapes (`Mdiamond`, `Msquare`, `box`, `hexagon`, `diamond`, `component`, `tripleoctagon`, `parallelogram`, `house`).

### 2.9 Chained Edges
- [x] [2.9-02] Apply shared edge attr block to each expanded edge.
- [x] [2.9-03] Add parser normalization test for chain equivalence.

### 2.10 Subgraphs
- [x] [2.10-03] Derive stylesheet classes from subgraph labels (normalize case/spaces/symbols).

### 2.11 Node and Edge Default Blocks
- [x] [2.11-01] Implement scoped `node [...]` defaults for subsequent node declarations.
- [x] [2.11-02] Implement scoped `edge [...]` defaults for subsequent edge declarations.
- [x] [2.11-03] Ensure explicit per-node/per-edge attrs always win over defaults.

### 2.12 Class Attribute
- [x] [2.12-01] Parse comma-separated node classes into normalized class list.
- [x] [2.12-02] Match `.class` stylesheet selectors against parsed class list.
- [x] [2.12-03] Add tests for multiple-class matching and precedence interactions.

### 2.13 Minimal Examples
- [x] [2.13-01] Add parser+validator fixture for simple linear workflow example.
- [x] [2.13-02] Add execution fixture for branching condition example.
- [x] [2.13-03] Add interviewer fixture for human-gate example with labeled options.

---

## 3. Pipeline Execution Engine

### 3.1 Run Lifecycle
- [x] [3.1-01] Implement lifecycle phases: `PARSE -> VALIDATE -> INITIALIZE -> EXECUTE -> FINALIZE`.
- [x] [3.1-02] In initialize, create run directory, seed context/checkpoint, apply transforms.
- [x] [3.1-03] In finalize, persist final checkpoint and completion events, then clean resources.

### 3.2 Core Execution Loop
- [x] [3.2-01] Resolve start node by shape/id rules; fail fast if ambiguous/missing.
- [x] [3.2-02] For each node: execute handler with retry policy and persist outcome.
- [x] [3.2-03] Merge `context_updates`, set `outcome`, set `preferred_label` when present.
- [x] [3.2-04] Save checkpoint after each stage with current node and completed list.
- [x] [3.2-05] Select next edge via algorithm in 3.3; handle no-edge failure cases.
- [x] [3.2-06] Implement `loop_restart` branch behavior.
- [x] [3.2-07] Stop at terminal node only after goal-gate enforcement.

### 3.3 Edge Selection Algorithm
- [x] [3.3-01] Implement condition-pass candidate evaluation.
- [x] [3.3-03] Implement `suggested_next_ids` matching fallback.
- [x] [3.3-05] Implement lexical tiebreaker by target node ID.
- [x] [3.3-06] Add deterministic routing tests that cover all five selection steps.

### 3.4 Goal Gate Enforcement
- [x] [3.4-01] Track outcomes for all nodes with `goal_gate=true`.
- [x] [3.4-02] Block terminal exit if any goal-gate node is non-success/non-partial-success.
- [x] [3.4-03] Resolve retry target chain: node retry target -> node fallback -> graph retry target -> graph fallback.
- [x] [3.4-04] Fail run when no valid retry target exists.

### 3.5 Retry Logic
- [x] [3.5-01] Build max attempts from `max_retries` + 1 semantics.
- [x] [3.5-02] Retry on `RETRY` outcomes and retryable exceptions until attempts exhausted.
- [x] [3.5-03] Reset retry counter on success/partial success.
- [x] [3.5-04] Honor `allow_partial=true` conversion after retry exhaustion.

### 3.6 Retry Policy
- [x] [3.6-01] Implement retry policy object (`max_attempts`, backoff config, `should_retry`).
- [x] [3.6-02] Implement delay formula with cap and optional jitter.
- [x] [3.6-03] Implement preset policies (`none`, `standard`, `aggressive`, `linear`, `patient`).
- [x] [3.6-04] Implement default retryability predicate by error class/status code.

### 3.7 Failure Routing
- [x] [3.7-02] Fallback to node `retry_target` then `fallback_retry_target`.
- [x] [3.7-03] Terminate with failure reason when no route exists.

### 3.8 Concurrency Model
- [x] [3.8-01] Keep top-level traversal single-threaded.
- [x] [3.8-02] Support branch concurrency only inside parallel handlers.
- [x] [3.8-03] Ensure branch contexts are isolated and only handler-declared updates flow back.

---

## 4. Node Handlers

### 4.1 Handler Interface
- [x] [4.1-01] Standardize handler execute signature and return `Outcome` contract.
- [x] [4.1-02] Pass `node`, `context`, `graph`, and `logs_root` to every handler call.
- [x] [4.1-03] Add handler conformance tests for contract shape.

### 4.2 Handler Registry
- [x] [4.2-02] Implement resolution order: explicit node type -> shape map -> default handler.
- [x] [4.2-03] Add resolution tests for each precedence level.

### 4.3 Start Handler
- [x] [4.3-01] Implement no-op start handler returning SUCCESS.
- [x] [4.3-02] Ensure lint enforces exactly one start node.

### 4.4 Exit Handler
- [x] [4.4-01] Implement no-op exit handler returning SUCCESS.
- [x] [4.4-02] Keep goal-gate logic in engine, not exit handler.

### 4.5 Codergen Handler (LLM Task)
- [x] [4.5-01] Build prompt from node prompt/label with `$goal` expansion.
- [x] [4.5-02] Write `prompt.md` before backend call and `response.md` afterward.
- [x] [4.5-03] Support backend return as text or full `Outcome`.
- [x] [4.5-04] Serialize `status.json` from final outcome.
- [x] [4.5-05] Return simulation response when backend is absent.

#### CodergenBackend Interface
- [x] [4.5b-01] Define backend interface returning `String | Outcome` for a stage invocation.
- [x] [4.5b-02] Add adapter tests for multiple backend implementations.

### 4.6 Wait For Human Handler
- [x] [4.6-01] Build answer options from outgoing edges (label fallback to target node ID).
- [x] [4.6-02] Parse accelerator keys from supported label patterns.
- [x] [4.6-03] Ask interviewer and map answer to selected edge/target.
- [x] [4.6-04] Implement timeout/default-choice behavior (`human.default_choice`).
- [x] [4.6-05] Return `suggested_next_ids` + `human.gate.*` context updates.

### 4.7 Conditional Handler
- [x] [4.7-01] Implement pass-through handler that returns SUCCESS.
- [x] [4.7-02] Keep actual condition routing in engine selector logic.

### 4.8 Parallel Handler
- [x] [4.8-01] Execute outgoing branches with bounded parallelism (`max_parallel`).
- [x] [4.8-02] Support join policies (`wait_all`, `k_of_n`, `first_success`, `quorum`).
- [x] [4.8-03] Support error policies (`fail_fast`, `continue`, `ignore`).

### 4.9 Fan-In Handler
- [x] [4.9-02] Support LLM-based ranking when prompt is present.
- [x] [4.9-03] Support heuristic ranking fallback.

### 4.10 Tool Handler
- [x] [4.10-01] Execute `tool_command` with timeout handling.
- [x] [4.10-02] Return FAIL when command missing or execution errors.
- [x] [4.10-03] Store command output in context updates/log artifacts.

### 4.11 Manager Loop Handler
- [x] [4.11-01] Implement child pipeline supervision loop with configurable poll interval/max cycles.
- [x] [4.11-02] Implement observe/steer/wait action set and stop-condition evaluation.
- [x] [4.11-03] Implement child status/outcome checks and fail/success resolution.
- [ ] [4.11-04] Emit intervention and telemetry artifacts for supervisor decisions.

### 4.12 Custom Handlers
- [ ] [4.12-01] Document and support custom handler registration by type string.
- [ ] [4.12-02] Catch handler exceptions and convert to FAIL outcomes.
- [ ] [4.12-03] Enforce statelessness/synchronization expectations for handler implementations.

---

## 5. State and Context

### 5.1 Context
- [ ] [5.1-01] Implement thread-safe context map with read/write locking semantics.
- [ ] [5.1-02] Implement context helpers: `set`, `get`, `get_string`, `append_log`, `snapshot`, `clone`, `apply_updates`.
- [ ] [5.1-03] Seed built-in keys (`outcome`, `preferred_label`, `graph.goal`, etc.) at appropriate lifecycle points.
- [ ] [5.1-04] Enforce namespace conventions (`context.*`, `graph.*`, `internal.*`, `parallel.*`, `stack.*`, `human.gate.*`, `work.*`).

### 5.2 Outcome
- [ ] [5.2-01] Define full outcome payload fields (`status`, `preferred_label`, `suggested_next_ids`, `context_updates`, `notes`, `failure_reason`).
- [ ] [5.2-02] Implement valid status enum and routing semantics for each status.
- [ ] [5.2-03] Ensure stage status transitions are persisted in `status.json` artifacts.

### 5.3 Checkpoint
- [ ] [5.3-01] Persist checkpoint JSON with timestamp/current node/completed/retries/context/logs.
- [ ] [5.3-02] Restore checkpoint for resume and continue from correct next node.
- [ ] [5.3-03] Restore retry counters and context values exactly.
- [ ] [5.3-04] Implement post-resume fidelity degradation rule for previous `full` stage.

### 5.4 Context Fidelity
- [ ] [5.4-01] Implement supported fidelity modes (`full`, `truncate`, `compact`, `summary:low`, `summary:medium`, `summary:high`).
- [ ] [5.4-02] Implement fidelity precedence: edge -> target node -> graph default -> `compact`.
- [ ] [5.4-03] Implement thread-key resolution precedence for `full` fidelity.
- [ ] [5.4-04] Verify session reuse/isolation behavior with thread keys.

### 5.5 Artifact Store
- [ ] [5.5-01] Implement artifact registry with metadata (`ArtifactInfo`) and typed retrieval.
- [ ] [5.5-02] Implement file-backing threshold behavior (default 100KB).
- [ ] [5.5-03] Implement `store`, `retrieve`, `has`, `list`, `remove`, `clear`.

### 5.6 Run Directory Structure
- [ ] [5.6-01] Create run root with `checkpoint.json`, `manifest.json`, stage directories, and `artifacts/`.
- [ ] [5.6-02] Ensure each stage directory includes `prompt.md`, `response.md`, `status.json`.
- [ ] [5.6-03] Add integrity test that verifies directory structure after end-to-end run.

---

## 6. Human-in-the-Loop (Interviewer Pattern)

### 6.1 Interviewer Interface
- [ ] [6.1-01] Implement `ask`, `ask_multiple`, and `inform` interface contract.
- [ ] [6.1-02] Add adapter compatibility tests for all built-in interviewer variants.

### 6.2 Question Model
- [ ] [6.2-01] Implement full question payload (`text`, `type`, `options`, `default`, `timeout_seconds`, `stage`, `metadata`).
- [ ] [6.2-02] Validate question types and option schema.

### 6.3 Answer Model
- [ ] [6.3-01] Implement answer payload (`value`, `selected_option`, `text`).
- [ ] [6.3-02] Support `YES/NO/SKIPPED/TIMEOUT` answer values.

### 6.4 Built-In Interviewer Implementations
- [ ] [6.4-01] Implement `AutoApproveInterviewer` behavior for yes/no and multiple-choice.
- [ ] [6.4-02] Implement `ConsoleInterviewer` input handling and option matching.
- [ ] [6.4-03] Implement `CallbackInterviewer` delegation path.
- [ ] [6.4-04] Implement `QueueInterviewer` deterministic dequeue behavior.
- [ ] [6.4-05] Implement `RecordingInterviewer` wrapper and durable recording storage.

### 6.5 Timeout Handling
- [ ] [6.5-01] Apply default answer when timeout occurs and default exists.
- [ ] [6.5-02] Return `TIMEOUT` answer when no default exists.
- [ ] [6.5-03] Implement `human.default_choice` resolution for `wait.human` nodes.

---

## 7. Validation and Linting

### 7.1 Diagnostic Model
- [ ] [7.1-01] Implement diagnostic structure with rule/severity/message/node/edge/fix fields.
- [ ] [7.1-02] Block execution when any ERROR diagnostic exists.
- [ ] [7.1-03] Preserve WARNING/INFO diagnostics in API responses and UI surfaces.

### 7.2 Built-In Lint Rules
- [ ] [7.2-01] Implement `start_node` rule.
- [ ] [7.2-02] Implement `terminal_node` rule.
- [ ] [7.2-03] Implement `reachability` rule.
- [ ] [7.2-04] Implement `edge_target_exists` rule.
- [ ] [7.2-05] Implement `start_no_incoming` rule.
- [ ] [7.2-06] Implement `exit_no_outgoing` rule.
- [ ] [7.2-07] Implement `condition_syntax` rule.
- [ ] [7.2-08] Implement `stylesheet_syntax` rule.
- [ ] [7.2-09] Implement `type_known` warning rule.
- [ ] [7.2-10] Implement `fidelity_valid` warning rule.
- [ ] [7.2-11] Implement `retry_target_exists` warning rule.
- [ ] [7.2-12] Implement `goal_gate_has_retry` warning rule.
- [ ] [7.2-13] Implement `prompt_on_llm_nodes` warning rule.

### 7.3 Validation API
- [ ] [7.3-01] Implement `validate(graph, extra_rules)` composition path.
- [ ] [7.3-02] Implement `validate_or_raise` with aggregated error raising.
- [ ] [7.3-03] Add API endpoint coverage tests for error/warning payload shape.

### 7.4 Custom Lint Rules
- [ ] [7.4-01] Implement `LintRule` plugin registration and execution.
- [ ] [7.4-02] Guarantee built-in rules run before custom rules.

---

## 8. Model Stylesheet

### 8.1 Overview
- [ ] [8.1-01] Parse stylesheet from graph attribute and apply as defaults-only transform.
- [ ] [8.1-02] Ensure node explicit attrs override stylesheet-inferred values.

### 8.2 Stylesheet Grammar
- [ ] [8.2-01] Implement parser for `Rule+` grammar with selector/declaration blocks.
- [ ] [8.2-02] Restrict properties to `llm_model`, `llm_provider`, `reasoning_effort`.
- [ ] [8.2-03] Enforce class name format and declaration syntax validation.

### 8.3 Selectors and Specificity
- [ ] [8.3-01] Implement selector matching for `*`, `.class`, and `#node_id`.
- [ ] [8.3-02] Implement specificity ordering and tie-break by later rule of equal specificity.

### 8.4 Recognized Properties
- [ ] [8.4-01] Apply arbitrary string values for `llm_model`.
- [ ] [8.4-02] Apply provider keys for `llm_provider`.
- [ ] [8.4-03] Validate `reasoning_effort` values (`low|medium|high`).

### 8.5 Application Order
- [ ] [8.5-01] Implement precedence order: node attr -> stylesheet -> graph default -> system default.
- [ ] [8.5-02] Run stylesheet transform post-parse/pre-validate.
- [ ] [8.5-03] Ensure transform only fills missing model-related attrs.

### 8.6 Example
- [ ] [8.6-01] Add fixture that reproduces universal/class/id precedence exactly as documented example.
- [ ] [8.6-02] Assert resolved model/provider/reasoning for `plan`, `implement`, and `critical_review`.

---

## 9. Transforms and Extensibility

### 9.1 AST Transforms
- [ ] [9.1-01] Implement transform interface (`apply(graph) -> graph`) and pipeline execution order.
- [ ] [9.1-02] Prevent destructive in-place mutation of original parsed graph.

### 9.2 Built-In Transforms
- [ ] [9.2-01] Implement variable expansion transform for `$goal` in prompts.
- [ ] [9.2-02] Implement stylesheet application transform.
- [ ] [9.2-03] Implement runtime preamble transform for non-`full` fidelity handoff.

### 9.3 Custom Transforms
- [ ] [9.3-01] Implement custom transform registration API.
- [ ] [9.3-02] Preserve deterministic execution order of custom transforms.
- [ ] [9.3-03] Add tests for transform chaining and conflict precedence.

### 9.4 Pipeline Composition
- [ ] [9.4-01] Support sub-pipeline execution pattern for handler-driven child graphs.
- [ ] [9.4-02] Support transform-based graph merging for modular pipelines.

### 9.5 HTTP Server Mode
- [ ] [9.5-01] Implement `POST /pipelines` start endpoint.
- [ ] [9.5-02] Implement `GET /pipelines/{id}` status/progress endpoint.
- [ ] [9.5-03] Implement `GET /pipelines/{id}/events` SSE stream.
- [ ] [9.5-04] Implement `POST /pipelines/{id}/cancel` endpoint.
- [ ] [9.5-05] Implement `GET /pipelines/{id}/graph` visualization endpoint.
- [ ] [9.5-06] Implement `GET /pipelines/{id}/questions` endpoint.
- [ ] [9.5-07] Implement `POST /pipelines/{id}/questions/{qid}/answer` endpoint.
- [ ] [9.5-08] Implement `GET /pipelines/{id}/checkpoint` endpoint.
- [ ] [9.5-09] Implement `GET /pipelines/{id}/context` endpoint.
- [ ] [9.5-10] Verify human-gate web controls operate entirely through run-scoped APIs.

### 9.6 Observability and Events
- [ ] [9.6-01] Emit pipeline lifecycle events (`Started`, `Completed`, `Failed`).
- [ ] [9.6-02] Emit stage lifecycle events (`StageStarted`, `StageCompleted`, `StageFailed`, `StageRetrying`).
- [ ] [9.6-03] Emit parallel block lifecycle events.
- [ ] [9.6-04] Emit interview lifecycle events.
- [ ] [9.6-05] Emit checkpoint-saved events.
- [ ] [9.6-06] Support observer callback consumption and streaming consumption.

### 9.7 Tool Call Hooks
- [ ] [9.7-01] Implement `tool_hooks.pre` command invocation before each tool call.
- [ ] [9.7-02] Implement `tool_hooks.post` command invocation after each tool call.
- [ ] [9.7-03] Pass tool metadata via env + stdin JSON to hooks.
- [ ] [9.7-04] Ensure non-zero hook exit is recorded but non-blocking for tool execution.

---

## 10. Condition Expression Language

### 10.1 Overview
- [ ] [10.1-01] Keep expression language minimal and deterministic for routing.
- [ ] [10.1-02] Reject unsupported operators/syntax during validation.

### 10.2 Grammar
- [ ] [10.2-01] Parse clauses joined by `&&`.
- [ ] [10.2-02] Support keys: `outcome`, `preferred_label`, `context.<path>`.
- [ ] [10.2-03] Support operators `=` and `!=` with typed literals.

### 10.3 Semantics
- [ ] [10.3-01] Evaluate clauses left-to-right with logical AND semantics.
- [ ] [10.3-02] Treat missing context keys as empty string.
- [ ] [10.3-03] Use exact case-sensitive string comparison.

### 10.4 Variable Resolution
- [ ] [10.4-01] Resolve `outcome` and `preferred_label` from current stage outcome.
- [ ] [10.4-02] Resolve `context.*` keys with fallback unprefixed lookup.
- [ ] [10.4-03] Resolve unknown keys to empty string.

### 10.5 Evaluation
- [ ] [10.5-01] Return true for empty condition.
- [ ] [10.5-02] Evaluate `!=` and `=` clauses correctly.
- [ ] [10.5-03] Support bare-key truthy checks.
- [ ] [10.5-04] Add parser/evaluator tests for mixed-clause expressions.

### 10.6 Examples
- [ ] [10.6-01] Add routing tests for `outcome=success` and `outcome=fail`.
- [ ] [10.6-02] Add routing test for conjunction with `context.tests_passed=true`.
- [ ] [10.6-03] Add routing tests for inequality and `preferred_label` matching.

### 10.7 Extended Operators (Future)
- [ ] [10.7-01] Document unsupported future operators as non-implemented features.
- [ ] [10.7-02] Add validation guardrails so unsupported operators fail with clear diagnostics.

---

## 11. Definition of Done

### 11.1 DOT Parsing
- [ ] [11.1-01] Convert each DoD bullet in spec 11.1 into an automated parser test.
- [ ] [11.1-02] Ensure parser test suite fails CI on any unsupported-grammar regression.

### 11.2 Validation and Linting
- [ ] [11.2-01] Convert each DoD bullet in spec 11.2 into validator tests.
- [ ] [11.2-02] Assert `validate_or_raise` behavior and diagnostic payload shape.

### 11.3 Execution Engine
- [ ] [11.3-01] Convert each DoD bullet in spec 11.3 into execution tests.
- [ ] [11.3-02] Add deterministic edge-selection conformance tests.

### 11.4 Goal Gate Enforcement
- [ ] [11.4-01] Convert each DoD bullet in spec 11.4 into goal-gate tests.

### 11.5 Retry Logic
- [ ] [11.5-01] Convert each DoD bullet in spec 11.5 into retry-policy tests.

### 11.6 Node Handlers
- [ ] [11.6-01] Convert each DoD bullet in spec 11.6 into per-handler contract tests.

### 11.7 State and Context
- [ ] [11.7-01] Convert each DoD bullet in spec 11.7 into context/checkpoint/artifact tests.

### 11.8 Human-in-the-Loop
- [ ] [11.8-01] Convert each DoD bullet in spec 11.8 into interviewer/human-gate tests.

### 11.9 Condition Expressions
- [ ] [11.9-01] Convert each DoD bullet in spec 11.9 into evaluator tests.

### 11.10 Model Stylesheet
- [ ] [11.10-01] Convert each DoD bullet in spec 11.10 into stylesheet transform tests.

### 11.11 Transforms and Extensibility
- [ ] [11.11-01] Convert each DoD bullet in spec 11.11 into transform/HTTP integration tests.

### 11.12 Cross-Feature Parity Matrix
- [ ] [11.12-01] Execute the entire parity matrix and persist a pass/fail report artifact.
- [ ] [11.12-02] Fail release gate if any matrix row is unchecked.

### 11.13 Integration Smoke Test
- [ ] [11.13-01] Implement exact smoke-test pipeline from spec in CI integration suite.
- [ ] [11.13-02] Assert parse/validate/execute outcomes and all required stage artifacts.
- [ ] [11.13-03] Assert goal-gate and checkpoint postconditions exactly as listed.

---

## Appendix A: Complete Attribute Reference

### Graph Attributes
- [ ] [A.G-01] Verify parser accepts and stores every listed graph attribute key.
- [ ] [A.G-02] Add validation/default-resolution coverage for each graph attribute.
- [ ] [A.G-03] Ensure `stack.*` and `tool_hooks.*` graph attrs are wired into manager/hook features.

### Node Attributes
- [ ] [A.N-01] Verify parser accepts and stores every listed node attribute key.
- [ ] [A.N-02] Add runtime behavior test for each non-display node attr (`max_retries`, `goal_gate`, `retry_target`, `fidelity`, `timeout`, `auto_status`, `allow_partial`).
- [ ] [A.N-03] Add precedence tests for model-related node attrs vs stylesheet/defaults.

### Edge Attributes
- [ ] [A.E-01] Verify parser accepts and stores every listed edge attribute key.
- [ ] [A.E-02] Add routing tests for `condition`, `weight`, and `label` interplay.
- [ ] [A.E-03] Add runtime tests for `fidelity`, `thread_id`, and `loop_restart` behavior.

---

## Appendix B: Shape-to-Handler-Type Mapping
- [ ] [B-01] Add one conformance test per shape-to-handler mapping row.
- [ ] [B-02] Assert no mapping drift between parser normalization, registry resolver, and docs.

---

## Appendix C: Status File Contract
- [ ] [C-01] Enforce `status.json` schema for all non-terminal stages.
- [ ] [C-02] Validate required enum/value constraints for `outcome` and optional fields.
- [ ] [C-03] Ensure `context_updates` merge semantics match engine behavior.
- [ ] [C-04] Implement `auto_status=true` synthesized file behavior when handler does not write status.

---

## Appendix D: Error Categories
- [ ] [D-01] Classify runtime errors into retryable/terminal/pipeline categories.
- [ ] [D-02] Route retryable errors through retry policy and backoff path.
- [ ] [D-03] Route terminal errors immediately to fail routing (no retries).
- [ ] [D-04] Surface pipeline structural errors during validation whenever possible.

---

## Release Gate Checklist
- [ ] [RG-01] Every subsection above has at least one completed implementation task.
- [ ] [RG-02] Every DoD matrix item (Section 11 + parity matrix) is linked to an automated test or explicit manual test record.
- [ ] [RG-03] API contract matches Section 9.5 endpoints and SSE semantics.
- [ ] [RG-04] Artifact directory and status file contract validations pass on integration smoke run.

---

## Deferred Tasks
- [ ] [4.2-01] Implement registration API with replacement behavior for duplicate keys. Deferred because `HandlerRegistry.register` already replaces existing entries via direct map assignment and this is checklist state drift.
- [ ] [2.6-02] Resolve `type` override before shape-based handler mapping. Deferred because `HandlerRegistry.resolve_handler_type` already checks explicit `type` before shape mapping and tests already cover this precedence (`tests/handlers/test_handlers.py`), so this is checklist state drift.
- [ ] [2.8-02] Ensure explicit `type` attribute overrides shape mapping. Deferred because explicit `type` precedence is already implemented in `HandlerRegistry.resolve_handler_type` and validated by `tests/handlers/test_handlers.py::test_registry_resolution_by_shape_and_type`, so this is checklist state drift.
- [ ] [2.3-05] Accept optional statement semicolons. Deferred because parser behavior is already implemented and covered by existing tests (`tests/dsl/test_parser.py`), so this is checklist state drift rather than a code gap.
- [ ] [2.4-02] Parse signed integers and floats. Deferred because signed integer/float parsing is already implemented in `attractor/dsl/parser.py` and exercised by parser tests, so this item is currently non-actionable checklist drift.
- [ ] [2.7-01] Implement edge `label` for preferred-label routing. Deferred because preferred-label edge routing is already implemented in `attractor/engine/routing.py` and covered by `tests/engine/test_routing.py::test_preferred_label_then_suggested_ids`, so this is checklist state drift.
- [ ] [2.7-03] Implement `weight` for deterministic prioritization. Deferred because deterministic weight routing is already implemented in `attractor/engine/routing.py` (`_best_by_weight_then_lexical`) and covered by `tests/engine/test_routing.py::test_weight_then_lexical_tiebreak_for_unconditional`, so this is checklist state drift.
- [ ] [2.9-01] Expand chained declarations into pairwise edges. Deferred because parser chain expansion already emits pairwise edges in `attractor/dsl/parser.py::parse_node_or_edge` and is covered by parser tests (`tests/dsl/test_parser.py`), so this is checklist state drift.
- [ ] [2.10-01] Flatten subgraph wrappers while preserving contained nodes/edges. Deferred because `parse_statement` already flattens subgraph bodies directly into the top-level `DotGraph` in `attractor/dsl/parser.py`, and the remaining explicit edge-retention coverage gap is better tracked under parser DoD test-conversion work (`11.1-01`) than this implementation task.
- [ ] [2.10-02] Apply subgraph-local defaults to enclosed nodes unless overridden. Deferred because parser subgraph scoping already applies `node [...]` defaults via child scope inheritance in `attractor/dsl/parser.py` and parser coverage exists in `tests/dsl/test_parser.py::test_parse_subgraph_scope_defaults`, so this is checklist state drift.
- [ ] [3.3-02] Implement normalized preferred-label matching (case/trim/accelerator stripping). Deferred because label normalization (case/trim/accelerator prefix stripping) is already implemented in `attractor/engine/routing.py::_normalize_label` and exercised by `tests/engine/test_routing.py::test_preferred_label_then_suggested_ids`, so this is checklist state drift.
- [ ] [3.3-04] Implement weight-descending selection for unconditional edges. Deferred because unconditional-edge routing already uses descending `weight` selection via `attractor/engine/routing.py::_best_by_weight_then_lexical` and is covered by routing tests, so this is checklist state drift.
- [ ] [3.7-01] Route fail outcomes to fail-edge (`condition="outcome=fail"`) when present. Deferred because fail-edge prioritization for `FAIL` outcomes is already implemented in `attractor/engine/executor.py::_select_route_edge` and covered by `tests/engine/test_retry_goal_gate.py::test_failure_routing_prefers_outcome_fail_edge_over_other_true_conditions`, so this is checklist state drift.
- [ ] [4.8-04] Serialize branch results into `parallel.results`. Deferred because `ParallelHandler` already emits `context_updates["parallel.results"]` and existing tests assert populated branch-result payloads (`tests/handlers/test_handlers.py`, `tests/integration/test_parity_matrix.py`), so this is checklist state drift.
- [ ] [4.9-01] Read `parallel.results` and fail when empty. Deferred because `FanInHandler.execute` already reads `parallel.results` and returns `FAIL` with `"No parallel results to evaluate"` when normalized results are empty, so this is checklist state drift.
- [ ] [4.9-04] Publish selected candidate via `parallel.fan_in.*` context keys. Deferred because `FanInHandler.execute` already sets `parallel.fan_in.best_id` and `parallel.fan_in.best_outcome`, and tests assert both keys in `tests/handlers/test_handlers.py`, so this is checklist state drift.
