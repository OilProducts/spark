//! Conformance of the bundled flows to the normative conversion schedule in
//! CR-2026-0179 (design.md section 5). The tables below are transcribed from
//! that schedule, not from the flow files, and each flow's nodes are compared
//! whole: node set, kinds, complete route maps and adapter configuration.

use std::collections::BTreeMap;
use std::path::Path;
use std::process::{Command, Output};

use serde_json::{json, Value};
use spark_assets::flows;

const DESCRIPTIONS: &[(&str, &str)] = &[
    ("done", "The bounded work required by this node’s prompt is complete, including its required outputs; this does not assert that the whole workflow objective is achieved."),
    ("unfinished", "Required work for this bounded node invocation remains and can be continued in the same session; no completed judgment is being asserted."),
    ("blocked", "This node cannot complete its bounded task without unavailable input or authority and no honest local workaround or deferral remains; record the blocker and its evidence in notes and any required artifacts."),
    ("ready_for_review", "The requested implementation, investigation or planning pass and its required outputs are complete enough for independent review, including any honestly recorded caveats."),
    ("ready", "The review is complete and no blocking issue remains under this node’s stated acceptance criteria; record all required review evidence."),
    ("fix", "The completed review or validation assessment found concrete defects requiring another implementation pass on the same item; record actionable findings."),
    ("replan", "The current implementation plan or queue cannot honestly support the next work step; record the evidence and perform any ledger repairs required by this node’s prompt before replanning."),
    ("verified", "The completed verification supports the diagnosis, plan or active-item state under this node’s stated checks; write its required evidence and clear stale findings as instructed."),
    ("gaps", "The completed verification found unproven, contradicted or missing claims; record each gap and the evidence needed to resolve it."),
    ("experiment", "The findings review found an evidence or reproducibility defect owned by the experiment step; record the required corrections."),
    ("synthesize", "The findings review found a labeling, scope or report defect owned by synthesis; record the required corrections."),
    ("faithful", "The completed fidelity review accepts the formal statement or its explicitly declared deviations under the prompt’s fidelity criteria."),
    ("revise", "The completed review requires revision of its subject within the existing scope; record the specific corrections and their evidence."),
    ("findings", "The statement is settled by faithful evidence or further rounds are not worth their budget; append the required verdict evidence before synthesis."),
    ("repair_statement", "The round exposed a materially different or inconsistent statement that warrants a repaired statement; append the required evidence before repair."),
    ("continue_round", "The statement remains unsettled and another round is warranted; append the required exchange and next targets."),
    ("approve", "The completed review accepts its subject under every applicable acceptance criterion; write any required review outputs."),
    ("reject", "The organization change is unnecessary or violates the authority and safety rule; record the rejection and its evidence."),
    ("rereview", "Substantive design changes have been made and require independent review again; update the required design summary."),
    ("complete", "The current item or milestone satisfies this node’s completion criteria with all required evidence and ledger updates; no in-scope work remains for that unit."),
    ("validated", "The current queue or completion state passes this node’s identity, dependency, coverage and validation checks after any permitted repairs."),
    ("revalidate", "The active item lacks current passing validation evidence; preserve its incomplete state and record precisely what must be rerun."),
    ("work", "A ready pending item or milestone has been durably dispatched with the required identity, attempt and context invariants verified."),
    ("audit", "No pending or active work remains in this queue; clear the required live bindings and advance to its independent audit."),
    ("extend", "Remaining implementable work belongs in the current scope; durably record the additional work required by this node’s prompt before continuing."),
    ("rewrite", "The current item or milestone decomposition is unsuitable; write the required replacement-work ledger before rewriting it."),
    ("validate", "The item review found the implementation complete enough for the downstream repository validation gate; this does not assert validation has passed."),
    ("rework", "The architecture review found blocking design defects; write their evidence and required corrections before revision."),
    ("requirements_covered", "All in-scope requirements and bound contract decisions are covered; repository deliverability still requires its separate audit."),
    ("deliverable", "The repository passes the final deliverability and integrity criteria with no outstanding bound contract decision."),
    ("cleanup", "Repository integrity or deliverability gaps remain; write the cleanup ledger so bounded cleanup milestones can be planned."),
    ("existing", "context.request.artifact_path names an existing design, so no approaches are drafted."),
    ("new", "No existing design was given, so competing approaches are drafted."),
];

const AGENTS: &[(&str, &[(&str, &str)])] = &[
    (
        "examples/human-review-loop/implement",
        &[("done", "review_gate")],
    ),
    (
        "examples/human-review-loop/revise",
        &[("done", "review_gate")],
    ),
    ("examples/human-review-loop/ship", &[("done", "exit")]),
    (
        "examples/implement-review-loop/implement",
        &[("done", "review"), ("replan", "plan")],
    ),
    (
        "examples/implement-review-loop/plan",
        &[("done", "implement")],
    ),
    (
        "examples/implement-review-loop/review",
        &[("ready", "done"), ("fix", "implement")],
    ),
    (
        "examples/parallel-review/branch_docs",
        &[("done", "fan_in")],
    ),
    (
        "examples/parallel-review/branch_tests",
        &[("done", "fan_in")],
    ),
    ("examples/parallel-review/final_review", &[("done", "exit")]),
    (
        "examples/parallel-review/implement",
        &[("done", "approval")],
    ),
    ("examples/parallel-review/plan", &[("done", "implement")]),
    ("examples/simple-linear/implement", &[("done", "summarize")]),
    ("examples/simple-linear/plan", &[("done", "implement")]),
    ("examples/simple-linear/summarize", &[("done", "done")]),
    (
        "examples/supervision/implementation-worker/implement",
        &[("done", "review")],
    ),
    (
        "examples/supervision/implementation-worker/review",
        &[("ready", "done"), ("fix", "implement")],
    ),
    (
        "examples/supervision/supervised-implementation/summarize_failure",
        &[("done", "failed")],
    ),
    (
        "examples/supervision/supervised-implementation/summarize_success",
        &[("done", "done")],
    ),
    (
        "math-research/explore-conjecture/experiment",
        &[("done", "verify_evidence")],
    ),
    (
        "math-research/explore-conjecture/review_findings",
        &[
            ("ready", "commit_findings"),
            ("experiment", "experiment"),
            ("synthesize", "synthesize"),
        ],
    ),
    (
        "math-research/explore-conjecture/survey",
        &[("done", "experiment")],
    ),
    (
        "math-research/explore-conjecture/synthesize",
        &[("done", "review_findings")],
    ),
    (
        "math-research/formalize-result/formalize",
        &[("done", "lean_gate")],
    ),
    (
        "math-research/formalize-result/review_fidelity",
        &[("faithful", "commit_result"), ("revise", "formalize")],
    ),
    (
        "math-research/prove-refute/arbiter",
        &[
            ("findings", "synthesize"),
            ("repair_statement", "repair_statement"),
            ("continue_round", "round_gate"),
        ],
    ),
    (
        "math-research/prove-refute/frame_problem",
        &[("done", "fan_out")],
    ),
    (
        "math-research/prove-refute/prove",
        &[("done", "prover_gate")],
    ),
    (
        "math-research/prove-refute/refute",
        &[("done", "refuter_gate")],
    ),
    (
        "math-research/prove-refute/repair_statement",
        &[("done", "round_gate")],
    ),
    (
        "math-research/prove-refute/review_findings",
        &[("ready", "commit_findings"), ("revise", "synthesize")],
    ),
    (
        "math-research/prove-refute/synthesize",
        &[("done", "review_findings")],
    ),
    (
        "math-research/research-program/assess",
        &[("done", "record")],
    ),
    (
        "math-research/research-program/record",
        &[("done", "commit")],
    ),
    (
        "math-research/research-program/repair",
        &[("done", "verify")],
    ),
    (
        "math-research/research-program/research",
        &[("done", "verify")],
    ),
    (
        "organization/review-change/review",
        &[("approve", "done"), ("revise", "done"), ("reject", "done")],
    ),
    (
        "software-development/audit-codebase/lens_correctness",
        &[("done", "fan_in")],
    ),
    (
        "software-development/audit-codebase/lens_minimality",
        &[("done", "fan_in")],
    ),
    (
        "software-development/audit-codebase/lens_safety",
        &[("done", "fan_in")],
    ),
    (
        "software-development/audit-codebase/lens_test_gaps",
        &[("done", "fan_in")],
    ),
    (
        "software-development/audit-codebase/report",
        &[("done", "done")],
    ),
    (
        "software-development/audit-codebase/verify_findings",
        &[("done", "report")],
    ),
    (
        "software-development/design-change/choose",
        &[("done", "draft")],
    ),
    (
        "software-development/design-change/cleanest_opus",
        &[("done", "explore_join")],
    ),
    (
        "software-development/design-change/cleanest_sol",
        &[("done", "explore_join")],
    ),
    (
        "software-development/design-change/codebase_check",
        &[("done", "fan_in")],
    ),
    (
        "software-development/design-change/critic",
        &[("done", "fan_in")],
    ),
    (
        "software-development/design-change/draft",
        &[("done", "review")],
    ),
    (
        "software-development/design-change/naive_reader",
        &[("done", "fan_in")],
    ),
    (
        "software-development/design-change/readiness",
        &[("done", "fan_in")],
    ),
    (
        "software-development/design-change/resolve",
        &[("done", "done"), ("rereview", "review")],
    ),
    (
        "software-development/design-change/route_request",
        &[("existing", "draft"), ("new", "explore")],
    ),
    (
        "software-development/design-change/smallest_opus",
        &[("done", "explore_join")],
    ),
    (
        "software-development/design-change/smallest_sol",
        &[("done", "explore_join")],
    ),
    (
        "software-development/design-change/synthesize",
        &[("done", "resolve")],
    ),
    (
        "software-development/design-change/zoom_out",
        &[("done", "fan_in")],
    ),
    (
        "software-development/implement-change/diagnose_validation",
        &[("done", "implement")],
    ),
    (
        "software-development/implement-change/evaluate",
        &[("ready", "validate"), ("fix", "implement")],
    ),
    (
        "software-development/implement-change/implement",
        &[("ready_for_review", "evaluate")],
    ),
    (
        "software-development/implement-change/normalize_task",
        &[("done", "prepare_workspace")],
    ),
    (
        "software-development/investigate-bug/investigate",
        &[("ready_for_review", "verify_investigation")],
    ),
    (
        "software-development/investigate-bug/propose_plan",
        &[("ready_for_review", "verify_plan")],
    ),
    (
        "software-development/investigate-bug/verify_investigation",
        &[("verified", "propose_plan"), ("gaps", "investigate")],
    ),
    (
        "software-development/investigate-bug/verify_plan",
        &[("verified", "done"), ("gaps", "propose_plan")],
    ),
    (
        "software-development/merge-change/normalize_task",
        &[("done", "prepare_integration")],
    ),
    (
        "software-development/merge-change/resolve_conflicts",
        &[("done", "validate")],
    ),
    (
        "software-development/review-change/find_issues",
        &[("done", "verify_findings")],
    ),
    (
        "software-development/review-change/report",
        &[("done", "done")],
    ),
    (
        "software-development/review-change/verify_findings",
        &[("done", "report")],
    ),
    (
        "software-development/run-retrospective/diagnose",
        &[("ready_for_review", "verify_diagnosis")],
    ),
    (
        "software-development/run-retrospective/recommend",
        &[("done", "done")],
    ),
    (
        "software-development/run-retrospective/verify_diagnosis",
        &[("verified", "recommend"), ("gaps", "diagnose")],
    ),
    (
        "software-development/spec-implementation/implement-milestone/assess_validation",
        &[
            ("complete", "gate_item_completion"),
            ("fix", "implement_current"),
            ("rewrite", "rewrite_current"),
            ("blocked", "mark_current_blocked"),
        ],
    ),
    (
        "software-development/spec-implementation/implement-milestone/blocked_exit",
        &[("done", "failed")],
    ),
    (
        "software-development/spec-implementation/implement-milestone/extract_items",
        &[("done", "validate_item_plan")],
    ),
    (
        "software-development/spec-implementation/implement-milestone/final_milestone_audit",
        &[
            ("complete", "record_milestone_success"),
            ("extend", "validate_item_plan"),
            ("blocked", "blocked_exit"),
        ],
    ),
    (
        "software-development/spec-implementation/implement-milestone/gate_item_completion",
        &[
            ("validated", "mark_current_done"),
            ("revalidate", "prepare_validation"),
            ("blocked", "blocked_exit"),
        ],
    ),
    (
        "software-development/spec-implementation/implement-milestone/implement_current",
        &[
            ("done", "validate_active_item_state"),
            ("replan", "plan_current"),
        ],
    ),
    (
        "software-development/spec-implementation/implement-milestone/mark_current_blocked",
        &[("done", "next_item"), ("blocked", "blocked_exit")],
    ),
    (
        "software-development/spec-implementation/implement-milestone/mark_current_done",
        &[
            ("done", "commit_current"),
            ("revalidate", "prepare_validation"),
            ("blocked", "blocked_exit"),
        ],
    ),
    (
        "software-development/spec-implementation/implement-milestone/next_item",
        &[
            ("work", "plan_current"),
            ("audit", "final_milestone_audit"),
            ("replan", "validate_item_plan"),
            ("blocked", "blocked_exit"),
        ],
    ),
    (
        "software-development/spec-implementation/implement-milestone/plan_current",
        &[("done", "implement_current")],
    ),
    (
        "software-development/spec-implementation/implement-milestone/prepare_milestone_state",
        &[("done", "extract_items"), ("blocked", "blocked_exit")],
    ),
    (
        "software-development/spec-implementation/implement-milestone/prepare_validation",
        &[("done", "validate_repo")],
    ),
    (
        "software-development/spec-implementation/implement-milestone/record_milestone_success",
        &[("done", "done")],
    ),
    (
        "software-development/spec-implementation/implement-milestone/review_current",
        &[
            ("validate", "prepare_validation"),
            ("fix", "implement_current"),
            ("rewrite", "rewrite_current"),
            ("blocked", "mark_current_blocked"),
        ],
    ),
    (
        "software-development/spec-implementation/implement-milestone/rewrite_current",
        &[("done", "validate_item_plan"), ("blocked", "blocked_exit")],
    ),
    (
        "software-development/spec-implementation/implement-milestone/validate_active_item_state",
        &[
            ("verified", "review_current"),
            ("replan", "validate_item_plan"),
            ("blocked", "blocked_exit"),
        ],
    ),
    (
        "software-development/spec-implementation/implement-milestone/validate_item_plan",
        &[("validated", "next_item"), ("blocked", "blocked_exit")],
    ),
    (
        "software-development/spec-implementation/implement-spec/audit_milestone",
        &[
            ("complete", "commit_milestone"),
            ("extend", "run_milestone"),
            ("rewrite", "rewrite_milestones"),
            ("blocked", "blocked_exit"),
        ],
    ),
    (
        "software-development/spec-implementation/implement-spec/blocked_exit",
        &[("done", "failed")],
    ),
    (
        "software-development/spec-implementation/implement-spec/design_architecture",
        &[("done", "evaluate_architecture")],
    ),
    (
        "software-development/spec-implementation/implement-spec/evaluate_architecture",
        &[
            ("approve", "plan_milestones"),
            ("rework", "revise_architecture"),
            ("blocked", "blocked_exit"),
        ],
    ),
    (
        "software-development/spec-implementation/implement-spec/extract_requirements",
        &[("done", "design_architecture")],
    ),
    (
        "software-development/spec-implementation/implement-spec/final_conformance_audit",
        &[
            ("requirements_covered", "repository_integrity_audit"),
            ("extend", "plan_milestones"),
            ("blocked", "blocked_exit"),
        ],
    ),
    (
        "software-development/spec-implementation/implement-spec/next_milestone",
        &[
            ("work", "run_milestone"),
            ("audit", "final_conformance_audit"),
            ("replan", "validate_milestone_plan"),
            ("blocked", "blocked_exit"),
        ],
    ),
    (
        "software-development/spec-implementation/implement-spec/plan_cleanup_milestone",
        &[("done", "validate_milestone_plan")],
    ),
    (
        "software-development/spec-implementation/implement-spec/plan_milestones",
        &[("done", "validate_milestone_plan")],
    ),
    (
        "software-development/spec-implementation/implement-spec/prepare_workspace",
        &[("done", "extract_requirements")],
    ),
    (
        "software-development/spec-implementation/implement-spec/repository_integrity_audit",
        &[
            ("deliverable", "done"),
            ("cleanup", "plan_cleanup_milestone"),
            ("blocked", "blocked_exit"),
        ],
    ),
    (
        "software-development/spec-implementation/implement-spec/revise_architecture",
        &[("done", "evaluate_architecture")],
    ),
    (
        "software-development/spec-implementation/implement-spec/rewrite_milestones",
        &[("done", "validate_milestone_plan")],
    ),
    (
        "software-development/spec-implementation/implement-spec/validate_milestone_plan",
        &[
            ("validated", "next_milestone"),
            ("replan", "plan_milestones"),
        ],
    ),
];

const EXIT_CODE_TOOLS: &[(&str, &str)] = &[
    (
        "math-research/explore-conjecture/prepare_workspace",
        "survey",
    ),
    ("math-research/explore-conjecture/commit_findings", "done"),
    (
        "math-research/formalize-result/prepare_workspace",
        "formalize",
    ),
    ("math-research/formalize-result/commit_result", "done"),
    (
        "math-research/prove-refute/prepare_workspace",
        "frame_problem",
    ),
    ("math-research/prove-refute/commit_findings", "done"),
    ("math-research/research-program/prepare", "research"),
    ("math-research/research-program/commit", "resolution"),
    (
        "software-development/implement-change/prepare_workspace",
        "implement",
    ),
    (
        "software-development/implement-change/finalize_commit",
        "cleanup_workspace",
    ),
    (
        "software-development/implement-change/cleanup_workspace",
        "done",
    ),
    ("software-development/merge-change/finish_merge", "done"),
    ("software-development/merge-change/validate", "finish_merge"),
    (
        "software-development/run-retrospective/locate_run",
        "diagnose",
    ),
    (
        "software-development/spec-implementation/implement-milestone/commit_current",
        "next_item",
    ),
    (
        "software-development/spec-implementation/implement-spec/commit_milestone",
        "next_milestone",
    ),
];

const JSON_TOOLS: &[(&str, &[(&str, &str, &str)])] = &[
    ("math-research/explore-conjecture/verify_evidence", &[("passed", "synthesize", "The evidence verifier completed successfully."), ("rejected", "experiment", "Required verifier evidence is missing or the verifier did not pass.")]),
    ("math-research/formalize-result/lean_gate", &[("passed", "review_fidelity", "The Lean build and proof-integrity scans passed."), ("rejected", "formalize", "Deliverable proof sources are missing, the build did not pass, or a forbidden proof construct was found.")]),
    ("math-research/prove-refute/prover_gate", &[("passed", "join", "No complete proof is claimed, or the claimed proof passed the build and integrity scans."), ("rejected", "prove", "A claimed proof lacks required sources or failed those checks.")]),
    ("math-research/prove-refute/refuter_gate", &[("passed", "join", "The refuter verifier passed and any REFUTED marker has its required witness report."), ("rejected", "refute", "Required verifier or witness evidence is missing, or the verifier did not pass.")]),
    ("math-research/prove-refute/round_gate", &[("more", "fan_out", "The completed round was recorded and the explicit domain round budget permits another round."), ("exhausted", "synthesize", "The completed round was recorded and the domain round budget is exhausted; synthesize the honest current findings.")]),
    ("math-research/research-program/repair_budget", &[("allowed", "repair", "The current action was read and the one-repair marker was successfully written; one focused repair is allowed."), ("exhausted", "assess", "The marker already exists, so assess the existing evidence without another repair.")]),
    ("math-research/research-program/resolution", &[("resolved", "done", "A valid explicit human acceptance or mechanically verified faithful resolution satisfies this node’s resolution predicate."), ("open", "research", "No mechanically accepted resolution is currently established; continue research without claiming resolution.")]),
    ("math-research/research-program/verify", &[("passed", "assess", "The action has no proposed machine-checkable verifier or its verifier passed, and the corresponding marker was written."), ("rejected", "repair_budget", "Required claim evidence is missing or malformed, or the action verifier did not pass; record the evidence for repair.")]),
    ("software-development/implement-change/validate", &[("passed", "finalize_commit", "The configured repository validation command returned zero and its evidence was captured."), ("rejected", "diagnose_validation", "That command returned nonzero; diagnose the captured output before further implementation.")]),
];

const STARTS: &[(&str, &str, &str)] = &[
    ("examples/human-review-loop", "implement", "exit"),
    ("examples/implement-review-loop", "plan", "done"),
    ("examples/parallel-review", "plan", "exit"),
    ("examples/simple-linear", "plan", "done"),
    (
        "examples/supervision/implementation-worker",
        "implement",
        "done",
    ),
    (
        "examples/supervision/supervised-implementation",
        "manager",
        "done",
    ),
    (
        "math-research/explore-conjecture",
        "prepare_workspace",
        "done",
    ),
    (
        "math-research/formalize-result",
        "prepare_workspace",
        "done",
    ),
    ("math-research/prove-refute", "prepare_workspace", "done"),
    ("math-research/research-program", "prepare", "done"),
    ("organization/review-change", "review", "done"),
    ("software-development/audit-codebase", "sweep", "done"),
    (
        "software-development/design-change",
        "route_request",
        "done",
    ),
    (
        "software-development/implement-change",
        "normalize_task",
        "done",
    ),
    (
        "software-development/investigate-bug",
        "investigate",
        "done",
    ),
    (
        "software-development/merge-change",
        "normalize_task",
        "done",
    ),
    ("software-development/review-change", "find_issues", "done"),
    (
        "software-development/run-retrospective",
        "locate_run",
        "done",
    ),
    (
        "software-development/spec-implementation/implement-milestone",
        "prepare_milestone_state",
        "done",
    ),
    (
        "software-development/spec-implementation/implement-spec",
        "prepare_workspace",
        "done",
    ),
];

const FORKS: &[(&str, &str, &[&str], &str, &str, bool)] = &[
    (
        "examples/parallel-review",
        "parallel_work",
        &["branch_docs", "branch_tests"],
        "fan_in",
        "final_review",
        true,
    ),
    (
        "software-development/audit-codebase",
        "sweep",
        &[
            "lens_correctness",
            "lens_minimality",
            "lens_safety",
            "lens_test_gaps",
        ],
        "fan_in",
        "verify_findings",
        true,
    ),
    (
        "software-development/design-change",
        "explore",
        &[
            "cleanest_opus",
            "cleanest_sol",
            "smallest_opus",
            "smallest_sol",
        ],
        "explore_join",
        "choose",
        true,
    ),
    (
        "software-development/design-change",
        "review",
        &[
            "codebase_check",
            "critic",
            "naive_reader",
            "readiness",
            "zoom_out",
        ],
        "fan_in",
        "synthesize",
        true,
    ),
    (
        "math-research/prove-refute",
        "fan_out",
        &["prove", "refute"],
        "join",
        "arbiter",
        false,
    ),
];

const HUMAN_GATES: &[(&str, &[(&str, &str, &str, &str)])] = &[
    (
        "examples/human-review-loop/review_gate",
        &[
            (
                "request_fixes",
                "revise",
                "The proposal needs revision; record the reviewer’s feedback.",
                "Request Fixes",
            ),
            (
                "approve",
                "ship",
                "The proposal is approved for shipping.",
                "Approve",
            ),
            (
                "stop",
                "failed",
                "Stop this workflow without approving the proposal.",
                "Stop Run",
            ),
        ],
    ),
    (
        "examples/parallel-review/approval",
        &[
            (
                "cancel",
                "failed",
                "Decline further work and end without approval.",
                "Cancel",
            ),
            (
                "proceed",
                "parallel_work",
                "Approve running the declared parallel review branches.",
                "Proceed",
            ),
        ],
    ),
];

const SUBFLOWS: &[(&str, &[(&str, &str)], &[(&str, &str, &str)])] = &[
    ("examples/supervision/supervised-implementation/manager", &[("completed", "child_completed"), ("blocked", "child_blocked")], &[("child_completed", "summarize_success", "The child reached its declared completed success result; summarize that result."), ("child_blocked", "summarize_failure", "The child reached its declared blocked failure result; summarize its preserved blocker evidence.")]),
    ("software-development/spec-implementation/implement-spec/run_milestone", &[("completed", "child_completed"), ("blocked", "child_blocked")], &[("child_completed", "audit_milestone", "The milestone child reached completed; independently audit its artifacts and deliverability."), ("child_blocked", "audit_milestone", "The milestone child reached blocked; audit its evidence to choose continuation, rewriting or an honest external block.")]),
];

const START_DESCRIPTION: &str = "Enter the first workflow step.";
const JOIN_DESCRIPTION: &str = "All branches settled under the fork policy; continue with completed results and any recorded execution errors.";

const VALIDATE_REPO: &str =
    "software-development/spec-implementation/implement-milestone/validate_repo";
const VALIDATE_REPO_ROUTES: &[(&str, &str, &str)] = &[
    ("passed", "assess_validation", "The generated repository validation entrypoint returned pass; assess its captured evidence."),
    ("rejected", "assess_validation", "The generated validation entrypoint reported non-pass or missing repository support; assess the evidence before repair."),
    ("blocked", "assess_validation", "The generated entrypoint identified a genuinely external missing prerequisite; assess its evidence before deciding whether the item is blocked."),
];
const PREPARE_INTEGRATION: &str = "software-development/merge-change/prepare_integration";
const PREPARE_INTEGRATION_DESCRIPTION: &str =
    "Integration preparation completed; inspect its conflict count.";
const CHECK_CONFLICTS: &str = "software-development/merge-change/check_conflicts";
const CHECK_CONFLICTS_ROUTES: &[(&str, &str, &str)] = &[
    (
        "clean",
        "check_target_moved",
        "Integration preparation completed and the staged merge has zero unresolved conflicts.",
    ),
    (
        "conflicts",
        "resolve_conflicts",
        "Integration preparation completed and the staged merge contains unresolved conflicts.",
    ),
];

const CHECK_TARGET_MOVED: &str = "software-development/merge-change/check_target_moved";
const CHECK_TARGET_MOVED_ROUTES: &[(&str, &str, &str)] = &[
    (
        "tested",
        "finish_merge",
        "The target commit is the merge base, so the merged tree is the source tree already tested.",
    ),
    (
        "untested",
        "validate",
        "The target moved past the merge base, so the merged tree has never been tested.",
    ),
];

/// One node as the schedule requires it: its kind, complete route map
/// (decision -> to, description, label) and exact values at JSON pointers
/// into the node (absent is `null`).
struct Expected {
    kind: &'static str,
    routes: BTreeMap<String, (String, String, String)>,
    fields: Vec<(&'static str, Value)>,
}

fn description(decision: &str) -> &'static str {
    DESCRIPTIONS
        .iter()
        .find(|(id, _)| *id == decision)
        .unwrap_or_else(|| panic!("no dictionary description for {decision}"))
        .1
}

fn split(path: &str) -> (&str, &str) {
    path.rsplit_once('/').expect("flow/node")
}

fn routes(entries: &[(&str, &str, &str, &str)]) -> BTreeMap<String, (String, String, String)> {
    let mut map = BTreeMap::new();
    for (decision, to, description, label) in entries {
        let previous = map.insert(
            decision.to_string(),
            (to.to_string(), description.to_string(), label.to_string()),
        );
        assert!(previous.is_none(), "duplicate schedule decision {decision}");
    }
    map
}

fn unlabeled(entries: &[(&str, &str, &str)]) -> BTreeMap<String, (String, String, String)> {
    routes(
        &entries
            .iter()
            .map(|(d, t, s)| (*d, *t, *s, ""))
            .collect::<Vec<_>>(),
    )
}

/// The whole schedule, flow -> node -> expectation.
fn schedule() -> BTreeMap<&'static str, BTreeMap<&'static str, Expected>> {
    let mut flows: BTreeMap<&str, BTreeMap<&str, Expected>> = BTreeMap::new();
    let mut add = |path: &'static str, expected: Expected| {
        let (flow, node) = split(path);
        let previous = flows.entry(flow).or_default().insert(node, expected);
        assert!(previous.is_none(), "{path} scheduled twice");
    };
    for (flow, first, exit) in STARTS {
        let start = format!("{flow}/start").leak();
        add(
            start,
            Expected {
                kind: "start",
                routes: unlabeled(&[("next", first, START_DESCRIPTION)]),
                fields: vec![],
            },
        );
        add(
            format!("{flow}/{exit}").leak(),
            Expected {
                kind: "exit",
                routes: BTreeMap::new(),
                fields: vec![(
                    "/config/result",
                    json!({"name": "completed", "outcome": "success"}),
                )],
            },
        );
        add(
            format!("{flow}/failed").leak(),
            Expected {
                kind: "exit",
                routes: BTreeMap::new(),
                fields: vec![(
                    "/config/result",
                    json!({"name": "blocked", "outcome": "failure"}),
                )],
            },
        );
    }
    for (path, explicit) in AGENTS {
        let (_, node) = split(path);
        let mut entries = vec![("unfinished", node, description("unfinished"))];
        if !explicit.iter().any(|(decision, _)| *decision == "blocked") {
            entries.push(("blocked", "failed", description("blocked")));
        }
        entries.extend(explicit.iter().map(|(d, t)| (*d, *t, description(d))));
        add(
            path,
            Expected {
                kind: "agent_task",
                routes: unlabeled(&entries),
                fields: vec![],
            },
        );
    }
    for (path, target) in EXIT_CODE_TOOLS {
        add(
            path,
            Expected {
                kind: "tool",
                routes: unlabeled(&[("done", target, description("done"))]),
                fields: vec![
                    ("/config/exit_codes", json!({"0": "done"})),
                    ("/config/decision_field", Value::Null),
                ],
            },
        );
    }
    for (path, entries) in JSON_TOOLS {
        add(
            path,
            Expected {
                kind: "tool",
                routes: unlabeled(entries),
                fields: vec![
                    ("/config/exit_codes", Value::Null),
                    ("/config/decision_field", json!("decision")),
                ],
            },
        );
    }
    add(
        VALIDATE_REPO,
        Expected {
            kind: "tool",
            routes: unlabeled(VALIDATE_REPO_ROUTES),
            fields: vec![
                (
                    "/config/exit_codes",
                    json!({"0": "passed", "1": "rejected", "2": "blocked"}),
                ),
                ("/config/decision_field", Value::Null),
            ],
        },
    );
    add(
        PREPARE_INTEGRATION,
        Expected {
            kind: "tool",
            routes: unlabeled(&[("next", "check_conflicts", PREPARE_INTEGRATION_DESCRIPTION)]),
            fields: vec![
                ("/config/exit_codes", json!({"0": "next"})),
                ("/config/decision_field", Value::Null),
            ],
        },
    );
    add(
        CHECK_CONFLICTS,
        Expected {
            kind: "switch",
            routes: unlabeled(CHECK_CONFLICTS_ROUTES),
            fields: vec![
                (
                    "/contracts/reads_context",
                    json!(["context.integration.conflict_count"]),
                ),
                ("/contracts/writes_context", Value::Null),
                ("/config/on", json!("context.integration.conflict_count")),
                ("/config/cases", json!({"0": "clean"})),
                ("/config/otherwise", json!("conflicts")),
            ],
        },
    );
    add(
        CHECK_TARGET_MOVED,
        Expected {
            kind: "switch",
            routes: unlabeled(CHECK_TARGET_MOVED_ROUTES),
            fields: vec![
                (
                    "/contracts/reads_context",
                    json!(["context.integration.target_moved"]),
                ),
                ("/contracts/writes_context", Value::Null),
                ("/config/on", json!("context.integration.target_moved")),
                ("/config/cases", json!({"false": "tested"})),
                ("/config/otherwise", json!("untested")),
            ],
        },
    );
    for (flow, fork, branches, join, next, tolerate) in FORKS {
        let mut fields = vec![
            ("/config/branches", json!(branches)),
            ("/config/join", json!(join)),
            // Serialization omits the default `false`.
            (
                "/config/tolerate_branch_execution_errors",
                if *tolerate { json!(true) } else { Value::Null },
            ),
        ];
        if *fork == "review" {
            fields.push(("/runtime/max_visits", json!(3)));
        }
        add(
            format!("{flow}/{fork}").leak(),
            Expected {
                kind: "parallel",
                routes: BTreeMap::new(),
                fields,
            },
        );
        add(
            format!("{flow}/{join}").leak(),
            Expected {
                kind: "fan_in",
                routes: unlabeled(&[("next", next, JOIN_DESCRIPTION)]),
                fields: vec![],
            },
        );
    }
    for (path, entries) in HUMAN_GATES {
        add(
            path,
            Expected {
                kind: "human_gate",
                routes: routes(entries),
                fields: vec![],
            },
        );
    }
    for (path, results, entries) in SUBFLOWS {
        let results = results
            .iter()
            .map(|(r, d)| (r.to_string(), json!(d)))
            .collect::<serde_json::Map<_, _>>();
        add(
            path,
            Expected {
                kind: "subflow",
                routes: unlabeled(entries),
                fields: vec![("/config/results", Value::Object(results))],
            },
        );
    }
    flows
}

/// A bundled flow's canonical JSON form, assembled like the loader does.
fn bundled_flow(name: &str) -> Value {
    let files = flows::starter_flow_files(name).expect("bundled flow files");
    attractor_dsl::assemble_flow_files(&files)
        .unwrap_or_else(|error| panic!("{name} does not assemble: {}", error.detail()))
        .to_canonical_json_value()
}

fn actual_routes(node: &Value) -> BTreeMap<String, (String, String, String)> {
    let text = |route: &Value, key: &str| route[key].as_str().unwrap_or_default().to_string();
    node["routes"]
        .as_object()
        .map(|routes| {
            routes
                .iter()
                .map(|(decision, route)| {
                    (
                        decision.clone(),
                        (
                            text(route, "to"),
                            text(route, "description"),
                            text(route, "label"),
                        ),
                    )
                })
                .collect()
        })
        .unwrap_or_default()
}

#[test]
fn schedule_inventory_matches_the_design_counts() {
    assert_eq!(AGENTS.len(), 103);
    assert_eq!(EXIT_CODE_TOOLS.len() + JSON_TOOLS.len() + 2, 27);
    assert_eq!((EXIT_CODE_TOOLS.len(), JSON_TOOLS.len()), (16, 9));
    assert_eq!(STARTS.len(), 20);
    let schedule = schedule();
    assert_eq!(
        schedule.keys().copied().collect::<Vec<_>>(),
        flows::starter_flow_names().expect("flow names"),
    );
    let mut kinds = BTreeMap::<String, usize>::new();
    let mut failed_exits = 0;
    for name in schedule.keys() {
        let flow = bundled_flow(name);
        for (id, node) in flow["nodes"].as_object().unwrap() {
            *kinds
                .entry(node["kind"].as_str().unwrap().to_string())
                .or_default() += 1;
            if id == "failed"
                && node["config"]["result"] == json!({"name": "blocked", "outcome": "failure"})
            {
                failed_exits += 1;
            }
        }
    }
    assert_eq!(
        kinds
            .iter()
            .map(|(k, v)| (k.as_str(), *v))
            .collect::<BTreeMap<_, _>>(),
        BTreeMap::from([
            ("agent_task", 103),
            ("exit", 40),
            ("fan_in", 5),
            ("human_gate", 2),
            ("parallel", 5),
            ("start", 20),
            ("subflow", 2),
            ("switch", 2),
            ("tool", 27),
        ])
    );
    assert_eq!(failed_exits, 20);
}

#[test]
fn every_bundled_node_matches_the_conversion_schedule() {
    let mut violations = Vec::new();
    for (name, expected_nodes) in schedule() {
        let flow = bundled_flow(name);
        let nodes = flow["nodes"].as_object().expect("nodes");
        let actual_ids = nodes.keys().map(String::as_str).collect::<Vec<_>>();
        let expected_ids = expected_nodes.keys().copied().collect::<Vec<_>>();
        if actual_ids != expected_ids {
            violations.push(format!(
                "{name}: nodes {actual_ids:?}, schedule {expected_ids:?}"
            ));
        }
        for (id, expected) in &expected_nodes {
            let Some(node) = nodes.get(*id) else { continue };
            if node["kind"] != json!(expected.kind) {
                violations.push(format!(
                    "{name}/{id}: kind {}, expected {}",
                    node["kind"], expected.kind
                ));
            }
            let actual = actual_routes(node);
            for decision in expected
                .routes
                .keys()
                .chain(actual.keys())
                .collect::<std::collections::BTreeSet<_>>()
            {
                let (want, got) = (expected.routes.get(decision), actual.get(decision));
                if want != got {
                    violations.push(format!(
                        "{name}/{id}.{decision}: expected {want:?}, actual {got:?}"
                    ));
                }
            }
            for (pointer, want) in &expected.fields {
                let got = node.pointer(pointer).unwrap_or(&Value::Null);
                if got != want {
                    violations.push(format!(
                        "{name}/{id}{pointer}: expected {want}, actual {got}"
                    ));
                }
            }
        }
    }
    assert!(
        violations.is_empty(),
        "schedule violations:\n{}",
        violations.join("\n")
    );
}

// Sandbox runs of the bundled JSON-mode tool scripts.

/// The tool's command run with `sh -c` in `dir`, each env_map variable set
/// from `context` (an unmapped variable is a test bug).
fn run_tool(path: &str, dir: &Path, context: &[(&str, &str)]) -> Output {
    let (flow, node) = split(path);
    let config = &bundled_flow(flow)["nodes"][node]["config"];
    let mut command = Command::new("sh");
    command
        .arg("-c")
        .arg(config["command"].as_str().expect("tool command"))
        .current_dir(dir);
    for (variable, key) in config["env_map"].as_object().into_iter().flatten() {
        let value = context
            .iter()
            .find(|(k, _)| *k == key.as_str().unwrap())
            .unwrap_or_else(|| panic!("{path}: no context value for {key}"))
            .1;
        command.env(variable, value);
    }
    command.output().expect("spawn tool")
}

/// The single JSON object a JSON-mode tool printed at exit 0; its decision
/// is one of the node's declared routes.
fn decision_output(path: &str, output: &Output) -> Value {
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "{path}: {:?} {stderr}",
        output.status
    );
    let value: Value = serde_json::from_slice(&output.stdout)
        .unwrap_or_else(|error| panic!("{path}: stdout is not one JSON value: {error}"));
    let (flow, node) = split(path);
    let routes = &bundled_flow(flow)["nodes"][node]["routes"];
    let decision = value["decision"].as_str().expect("decision field");
    assert!(
        routes.get(decision).is_some(),
        "{path}: undeclared {decision}"
    );
    value
}

/// A nonzero exit with no domain decision on stdout: an execution error.
fn assert_execution_error(path: &str, output: &Output) {
    assert!(!output.status.success(), "{path}: expected a nonzero exit");
    assert!(
        !String::from_utf8_lossy(&output.stdout).contains("decision"),
        "{path}: an execution error printed a decision"
    );
}

fn have(tools: &[&str], test: &str) -> bool {
    for tool in tools {
        let found = Command::new("sh")
            .arg("-c")
            .arg(format!("command -v {tool}"))
            .output()
            .is_ok_and(|output| output.status.success());
        if !found {
            eprintln!("skipping {test}: {tool} is not installed");
            return false;
        }
    }
    true
}

#[test]
fn repair_budget_allows_one_repair_then_exhausts() {
    const TOOL: &str = "math-research/research-program/repair_budget";
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path();
    std::fs::create_dir(dir.join(".math-research")).unwrap();
    std::fs::write(dir.join(".math-research/current-action"), "A0001\n").unwrap();
    let first = decision_output(TOOL, &run_tool(TOOL, dir, &[]));
    assert_eq!(first, json!({"decision": "allowed"}));
    assert_eq!(
        std::fs::read_to_string(dir.join(".math-research/repair-used")).unwrap(),
        "A0001\n"
    );
    let second = decision_output(TOOL, &run_tool(TOOL, dir, &[]));
    assert_eq!(second, json!({"decision": "exhausted"}));

    // A marker that is not a readable file is an operating failure, not exhaustion.
    std::fs::remove_file(dir.join(".math-research/repair-used")).unwrap();
    std::fs::create_dir(dir.join(".math-research/repair-used")).unwrap();
    assert_execution_error(TOOL, &run_tool(TOOL, dir, &[]));

    for action in ["bogus\n", ""] {
        let temp = tempfile::tempdir().unwrap();
        std::fs::create_dir(temp.path().join(".math-research")).unwrap();
        std::fs::write(temp.path().join(".math-research/current-action"), action).unwrap();
        assert_execution_error(TOOL, &run_tool(TOOL, temp.path(), &[]));
        assert!(!temp.path().join(".math-research/repair-used").exists());
    }
}

#[test]
fn round_gate_records_rounds_until_the_budget_is_exhausted() {
    const TOOL: &str = "math-research/prove-refute/round_gate";
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path();
    // Defaults: round 0 of 3.
    for (round, decision) in [(1, "more"), (2, "more"), (3, "exhausted")] {
        let output = decision_output(TOOL, &run_tool(TOOL, dir, &[]));
        assert_eq!(output, json!({"decision": decision}), "round {round}");
        assert_eq!(
            std::fs::read_to_string(dir.join(".mathlab/round")).unwrap(),
            format!("{round}\n")
        );
    }
    std::fs::write(dir.join(".mathlab/round"), "0\n").unwrap();
    std::fs::write(dir.join(".mathlab/max-rounds"), "1\n").unwrap();
    assert_eq!(
        decision_output(TOOL, &run_tool(TOOL, dir, &[]))["decision"],
        "exhausted"
    );

    for (round, max) in [("x\n", "3\n"), ("1\n", "0\n"), ("-1\n", "3\n")] {
        std::fs::write(dir.join(".mathlab/round"), round).unwrap();
        std::fs::write(dir.join(".mathlab/max-rounds"), max).unwrap();
        assert_execution_error(TOOL, &run_tool(TOOL, dir, &[]));
        assert_eq!(
            std::fs::read_to_string(dir.join(".mathlab/round")).unwrap(),
            round
        );
    }
}

#[test]
fn research_verify_rejects_missing_evidence_and_passes_unverifiable_claims() {
    const TOOL: &str = "math-research/research-program/verify";
    if !have(&["bash", "jq"], "research_verify") {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path();
    let evidence = dir.join("evidence/A0007");
    std::fs::create_dir_all(dir.join(".math-research")).unwrap();
    std::fs::create_dir_all(&evidence).unwrap();
    std::fs::write(dir.join(".math-research/current-action"), "A0007\n").unwrap();
    let verify = || decision_output(TOOL, &run_tool(TOOL, dir, &[]))["decision"].clone();

    assert_eq!(verify(), "rejected");
    assert!(std::fs::read_to_string(evidence.join("verification.log"))
        .unwrap()
        .contains("claim.json is missing"));
    std::fs::write(evidence.join("claim.json"), "[]").unwrap();
    assert_eq!(verify(), "rejected");

    std::fs::write(evidence.join("claim.json"), "{}").unwrap();
    assert_eq!(verify(), "passed");
    assert!(evidence.join(".not-machine-checkable").exists());

    std::fs::write(evidence.join("verify.sh"), "echo checking; exit 1\n").unwrap();
    assert_eq!(verify(), "rejected");
    assert!(!evidence.join(".not-machine-checkable").exists());
    assert!(std::fs::read_to_string(evidence.join("verification.log"))
        .unwrap()
        .contains("checking"));
    std::fs::write(evidence.join("verify.sh"), "exit 0\n").unwrap();
    assert_eq!(verify(), "passed");
    assert!(evidence.join(".verified").exists());

    std::fs::write(dir.join(".math-research/current-action"), "nope\n").unwrap();
    assert_execution_error(TOOL, &run_tool(TOOL, dir, &[]));
}

#[test]
fn implement_change_validate_reports_the_wrapped_command_status_as_data() {
    const TOOL: &str = "software-development/implement-change/validate";
    if !have(&["jq"], "implement_change_validate") {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let workspace = temp.path().to_str().unwrap();
    let run = |command: &str| {
        run_tool(
            TOOL,
            temp.path(),
            &[
                ("context.task.validation_command", command),
                ("context.workspace.path", workspace),
            ],
        )
    };
    let passed = decision_output(TOOL, &run("true"));
    assert_eq!(
        passed,
        json!({"decision": "passed", "command_exit_code": 0, "command_output": ""})
    );

    let rejected = decision_output(TOOL, &run("echo 'quote \" and\ttab'; exit 3"));
    assert_eq!(rejected["decision"], "rejected");
    assert_eq!(rejected["command_exit_code"], 3);
    assert_eq!(rejected["command_output"], "quote \" and\ttab\n");

    // Every output_map source is present in the decision object.
    let config =
        &bundled_flow("software-development/implement-change")["nodes"]["validate"]["config"];
    for source in config["output_map"].as_object().unwrap().values() {
        assert!(rejected.get(source.as_str().unwrap()).is_some(), "{source}");
    }

    // A missing workspace is the wrapper's own failure, not a rejection.
    let missing = temp.path().join("missing");
    let output = run_tool(
        TOOL,
        temp.path(),
        &[
            ("context.task.validation_command", "true"),
            ("context.workspace.path", missing.to_str().unwrap()),
        ],
    );
    assert_execution_error(TOOL, &output);
}
