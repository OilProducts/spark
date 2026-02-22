# Attractor Spec Compliance Plan

Goal: reach 100% compliance with `attractor-spec.md` through incremental, test-backed milestones.

Execution mode:
- Complete milestones in order.
- One git commit per completed milestone.
- Keep API behavior working while progressively replacing prototype code.

## Milestone 0: Plan Baseline
Scope:
- Capture this ordered implementation plan in-repo so progress survives context compaction.
Done when:
- Plan file exists with milestones, acceptance criteria, and commit strategy.

## Milestone 1: DOT AST + Parser + Validator
Spec targets:
- Section 2 (DOT DSL Schema)
- Section 7 (Validation and Linting)
- DoD 11.1 + 11.2
Implementation:
- Create `attractor/dsl/models.py` typed AST objects.
- Create `attractor/dsl/parser.py` with strict DOT subset parsing:
  - one digraph only
  - directed edges only
  - attr blocks, typed values, comments stripped
  - chained edges expanded
- Create `attractor/dsl/validator.py` with diagnostic model and core lint rules:
  - start/exit constraints
  - reachability
  - edge target existence
  - condition syntax checks (structural)
- Add parser/validator tests in `tests/dsl/`.
Done when:
- Parser and validator unit tests pass.
- Existing prototype still imports/runs.

## Milestone 2: Engine Core + Routing + Condition Eval
Spec targets:
- Section 3.1-3.3
- Section 10
- DoD 11.3 (core parts) + 11.9
Implementation:
- Create `attractor/engine/outcome.py`, `context.py`, `routing.py`, `executor.py`.
- Implement edge selection priority:
  1) condition
  2) preferred label
  3) suggested_next_ids
  4) weight
  5) lexical tiebreak
- Implement condition expression grammar/evaluation (`=`, `!=`, `&&`, vars: outcome/preferred_label/context.*).
- Add tests in `tests/engine/`.
Done when:
- Routing and condition tests pass against documented examples.

## Milestone 3: State, Checkpoint, Artifacts, Resume
Spec targets:
- Section 5
- Appendix C
- DoD 11.7
Implementation:
- Create `attractor/engine/checkpoint.py` schema and persistence.
- Create run directory layout writer and stage artifact writers:
  - `prompt.md`, `response.md`, `status.json`
- Save checkpoint after each node completion.
- Resume from checkpoint path and continue from saved node.
- Add tests for save/resume equivalence.
Done when:
- Resume test reproduces same final outcome as clean run.

## Milestone 4: Handler Registry + Built-in Handlers + Interviewer
Spec targets:
- Section 4
- Section 6
- DoD 11.6 + 11.8
Implementation:
- Create `attractor/handlers/base.py`, `registry.py`.
- Implement built-ins:
  - start, exit, codergen, wait.human, conditional, parallel, parallel.fan_in, tool
- Create interviewer interfaces/implementations:
  - AutoApprove, Console, Callback, Queue
- Support shape-to-handler mapping + explicit `type` override.
- Add tests for handler dispatch and behaviors.
Done when:
- All required handlers execute via registry in tests.

## Milestone 5: Goal Gates + Retry + Failure Routing
Spec targets:
- Section 3.4-3.7
- DoD 11.4 + 11.5
Implementation:
- Implement `goal_gate` tracking and terminal gate enforcement.
- Implement retry policy with node + graph defaults.
- Implement failure routing precedence:
  - fail edge
  - node retry_target
  - node fallback_retry_target
  - graph retry_target/fallback
- Add tests for retry exhaustion and gate routing.
Done when:
- Goal-gate and retry tests pass.

## Milestone 6: Stylesheet + Transforms Extensibility
Spec targets:
- Section 8
- Section 9.1-9.4
- DoD 11.10 + 11.11
Implementation:
- Create `attractor/transforms/base.py`, `stylesheet.py`, `variables.py`.
- Parse/apply model stylesheet with specificity ordering.
- Implement transform registry and execution order.
- Built-in variable expansion transform for `$goal`.
- Add tests for selector precedence and overrides.
Done when:
- Stylesheet and transform tests pass.

## Milestone 7: API Integration + Parity Matrix + Smoke
Spec targets:
- Section 9.5 + 9.6 (where applicable)
- DoD 11.12 + 11.13
Implementation:
- Refactor FastAPI server to call new parser/validator/executor pipeline.
- Preserve `/run`, `/reset`, websocket events.
- Add parity test matrix file and integration smoke tests from spec scenarios.
- Update minimal frontend wiring if needed for new runtime data.
Done when:
- Parity matrix checks pass in test suite.
- Integration smoke test passes.

## Commit Strategy
- Commit once per milestone completion.
- Commit messages:
  - `plan: add attractor 100% compliance milestone roadmap`
  - `dsl: add typed dot parser and validator`
  - `engine: add routing and condition evaluator`
  - `engine: add checkpoint resume and artifact contract`
  - `handlers: add registry built-ins and interviewer implementations`
  - `engine: add goal-gate retry and failure routing`
  - `transforms: add stylesheet and transform pipeline`
  - `api/tests: wire full engine and add parity smoke coverage`
