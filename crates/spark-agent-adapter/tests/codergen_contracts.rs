use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

use attractor_core::{ContextMap, FailureKind, Outcome, OutcomeStatus};
use attractor_dsl::{parse_dot, AttributeDefaultsTransform, GraphTransform};
use serde_json::json;
use spark_agent_adapter::{
    build_status_envelope_prompt_appendix, CodergenBackend, CodergenBackendOutput,
    CodergenBackendRequest, CodergenError, CodergenHandler, CodergenRequest,
};
use tempfile::tempdir;
use unified_llm_adapter::{RUNTIME_LAUNCH_MODEL_KEY, RUNTIME_LAUNCH_REASONING_EFFORT_KEY};

#[derive(Clone)]
struct ScriptedBackend {
    calls: Rc<RefCell<Vec<CodergenBackendRequest>>>,
    outputs: Rc<RefCell<VecDeque<CodergenBackendOutput>>>,
}

impl ScriptedBackend {
    fn new(outputs: impl IntoIterator<Item = CodergenBackendOutput>) -> Self {
        Self {
            calls: Rc::new(RefCell::new(Vec::new())),
            outputs: Rc::new(RefCell::new(outputs.into_iter().collect())),
        }
    }
}

impl CodergenBackend for ScriptedBackend {
    fn run(
        &mut self,
        request: CodergenBackendRequest,
    ) -> Result<CodergenBackendOutput, CodergenError> {
        self.calls.borrow_mut().push(request);
        Ok(self
            .outputs
            .borrow_mut()
            .pop_front()
            .unwrap_or_else(|| CodergenBackendOutput::text("backend text response")))
    }
}

#[test]
fn codergen_selects_authored_prompt_expands_goal_and_writes_artifacts() {
    let graph = parse_dot(
        r#"
        digraph G {
          graph [goal="Ship docs"];
          task [shape=box, prompt="Plan for $goal", label="Label loses"];
        }
        "#,
    )
    .expect("dot parses");
    let backend = ScriptedBackend::new([CodergenBackendOutput::text("backend text response")]);
    let calls = backend.calls.clone();
    let logs_root = tempdir().unwrap();
    let mut handler = CodergenHandler::with_backend(backend);

    let execution = handler
        .execute(CodergenRequest {
            node_id: "task".to_string(),
            node: graph.nodes["task"].clone(),
            graph,
            context: ContextMap::from([("graph.goal".to_string(), json!("ship"))]),
            logs_root: Some(logs_root.path().to_path_buf()),
            fallback_model: None,
            fallback_provider: None,
            fallback_profile: None,
            fallback_reasoning_effort: None,
        })
        .unwrap();

    assert_eq!(execution.outcome.status, OutcomeStatus::Success);
    assert_eq!(calls.borrow()[0].prompt, "Plan for ship");
    assert_eq!(
        std::fs::read_to_string(logs_root.path().join("task/prompt.md"))
            .unwrap()
            .trim(),
        "Plan for ship"
    );
    assert_eq!(
        std::fs::read_to_string(logs_root.path().join("task/response.md"))
            .unwrap()
            .trim(),
        "backend text response"
    );
}

#[test]
fn codergen_uses_authored_label_but_not_generated_label() {
    let graph = parse_dot(
        r#"
        digraph G {
          labelled [shape=box, prompt="", label="Label Prompt"];
          generated [shape=box];
        }
        "#,
    )
    .expect("dot parses");
    let mut graph_with_defaults = graph.clone();
    AttributeDefaultsTransform::default().apply(&mut graph_with_defaults);

    let labelled_backend = ScriptedBackend::new([CodergenBackendOutput::text("ok")]);
    let labelled_calls = labelled_backend.calls.clone();
    CodergenHandler::with_backend(labelled_backend)
        .execute(CodergenRequest {
            node_id: "labelled".to_string(),
            node: graph.nodes["labelled"].clone(),
            graph: graph.clone(),
            context: ContextMap::new(),
            logs_root: None,
            fallback_model: None,
            fallback_provider: None,
            fallback_profile: None,
            fallback_reasoning_effort: None,
        })
        .unwrap();
    assert_eq!(labelled_calls.borrow()[0].prompt, "Label Prompt");

    let generated_backend = ScriptedBackend::new([CodergenBackendOutput::text("ok")]);
    let generated_calls = generated_backend.calls.clone();
    CodergenHandler::with_backend(generated_backend)
        .execute(CodergenRequest {
            node_id: "generated".to_string(),
            node: graph_with_defaults.nodes["generated"].clone(),
            graph: graph_with_defaults,
            context: ContextMap::new(),
            logs_root: None,
            fallback_model: None,
            fallback_provider: None,
            fallback_profile: None,
            fallback_reasoning_effort: None,
        })
        .unwrap();
    assert_eq!(generated_calls.borrow()[0].prompt, "");
}

#[test]
fn codergen_declared_reads_and_malformed_read_contract_are_observable() {
    let graph = parse_dot(
        r#"
        digraph G {
          task [
            shape=box,
            prompt="Plan",
            spark.reads_context="[\"context.request.summary\",\"context.missing\"]"
          ];
          bad [shape=box, prompt="Bad", spark.reads_context="{\"bad\":true}"];
        }
        "#,
    )
    .expect("dot parses");
    let backend = ScriptedBackend::new([CodergenBackendOutput::text("ok")]);
    let calls = backend.calls.clone();
    CodergenHandler::with_backend(backend)
        .execute(CodergenRequest {
            node_id: "task".to_string(),
            node: graph.nodes["task"].clone(),
            graph: graph.clone(),
            context: ContextMap::from([
                (
                    "context.request.summary".to_string(),
                    json!("Ship docs safely"),
                ),
                (
                    "_attractor.runtime.context_carryover".to_string(),
                    json!("carryover:summary:high"),
                ),
            ]),
            logs_root: None,
            fallback_model: None,
            fallback_provider: None,
            fallback_profile: None,
            fallback_reasoning_effort: None,
        })
        .unwrap();
    let prompt = &calls.borrow()[0].prompt;
    assert!(prompt.contains("Context carryover:\n\ncarryover:summary:high"));
    assert!(prompt.contains("context.request.summary=Ship docs safely"));
    assert!(prompt.contains("context.missing=<missing>"));

    let execution = CodergenHandler::with_backend(ScriptedBackend::new([]))
        .execute(CodergenRequest {
            node_id: "bad".to_string(),
            node: graph.nodes["bad"].clone(),
            graph,
            context: ContextMap::new(),
            logs_root: None,
            fallback_model: None,
            fallback_provider: None,
            fallback_profile: None,
            fallback_reasoning_effort: None,
        })
        .unwrap();
    assert_eq!(execution.outcome.status, OutcomeStatus::Fail);
    assert_eq!(execution.outcome.failure_kind, Some(FailureKind::Contract));
    assert!(execution
        .outcome
        .failure_reason
        .contains("expected a JSON array of strings"));
}

#[test]
fn codergen_status_envelope_repair_is_separate_from_node_retry() {
    let graph = parse_dot(
        r#"
        digraph G {
          task [
            shape=box,
            prompt="Review",
            codergen.response_contract="status_envelope",
            codergen.contract_repair_attempts=1,
            spark.writes_context="[\"context.review.summary\"]"
          ];
        }
        "#,
    )
    .expect("dot parses");
    let backend = ScriptedBackend::new([
        CodergenBackendOutput::text(
            r#"{"outcome":"success","context_updates":{"context.review.extra":"nope"}}"#,
        ),
        CodergenBackendOutput::text(
            r#"{"outcome":"success","context_updates":{"context.review.summary":"ready"}}"#,
        ),
    ]);
    let calls = backend.calls.clone();
    let mut handler = CodergenHandler::with_backend(backend);
    let execution = handler
        .execute(CodergenRequest {
            node_id: "task".to_string(),
            node: graph.nodes["task"].clone(),
            graph,
            context: ContextMap::new(),
            logs_root: None,
            fallback_model: None,
            fallback_provider: None,
            fallback_profile: None,
            fallback_reasoning_effort: None,
        })
        .unwrap();

    assert_eq!(execution.outcome.status, OutcomeStatus::Success);
    assert_eq!(
        execution.outcome.context_updates["context.review.summary"],
        json!("ready")
    );
    assert_eq!(execution.repair_attempts, 1);
    assert_eq!(calls.borrow().len(), 2);
    assert_eq!(calls.borrow()[1].repair_attempt, Some(1));
    assert!(calls.borrow()[0]
        .prompt
        .contains(&build_status_envelope_prompt_appendix(Some(
            &calls.borrow()[0].write_contract
        ))));
    assert!(calls.borrow()[1]
        .prompt
        .contains("Do not do new repository work."));
}

#[test]
fn codergen_contract_failure_when_repair_budget_exhausts() {
    let graph = parse_dot(
        r#"
        digraph G {
          task [
            shape=box,
            prompt="Review",
            codergen.response_contract="status_envelope",
            codergen.contract_repair_attempts=0
          ];
        }
        "#,
    )
    .expect("dot parses");
    let backend = ScriptedBackend::new([CodergenBackendOutput::text(
        r#"{"outcome":"success","notes":["bad"]}"#,
    )]);
    let execution = CodergenHandler::with_backend(backend)
        .execute(CodergenRequest {
            node_id: "task".to_string(),
            node: graph.nodes["task"].clone(),
            graph,
            context: ContextMap::new(),
            logs_root: None,
            fallback_model: None,
            fallback_provider: None,
            fallback_profile: None,
            fallback_reasoning_effort: None,
        })
        .unwrap();
    assert_eq!(execution.outcome.status, OutcomeStatus::Fail);
    assert_eq!(execution.outcome.failure_kind, Some(FailureKind::Contract));
    assert!(execution
        .outcome
        .failure_reason
        .contains("notes must be a string"));
}

#[test]
fn codergen_resolves_model_provider_profile_and_reasoning() {
    let graph = parse_dot(
        r#"
        digraph G {
          task [shape=box, prompt="Plan", llm_provider="Anthropic"];
        }
        "#,
    )
    .expect("dot parses");
    let backend = ScriptedBackend::new([CodergenBackendOutput::outcome(Outcome {
        status: OutcomeStatus::Retry,
        notes: "please retry".to_string(),
        suggested_next_ids: vec!["fallback_stage".to_string()],
        ..Outcome::new(OutcomeStatus::Retry)
    })]);
    let calls = backend.calls.clone();
    let execution = CodergenHandler::with_backend(backend)
        .execute(CodergenRequest {
            node_id: "task".to_string(),
            node: graph.nodes["task"].clone(),
            graph,
            context: ContextMap::from([
                (RUNTIME_LAUNCH_MODEL_KEY.to_string(), json!("gpt-launch")),
                (
                    RUNTIME_LAUNCH_REASONING_EFFORT_KEY.to_string(),
                    json!("medium"),
                ),
            ]),
            logs_root: None,
            fallback_model: None,
            fallback_provider: None,
            fallback_profile: Some("backend-profile".to_string()),
            fallback_reasoning_effort: None,
        })
        .unwrap();

    assert_eq!(calls.borrow()[0].provider, "anthropic");
    assert_eq!(calls.borrow()[0].model.as_deref(), Some("gpt-launch"));
    assert_eq!(
        calls.borrow()[0].llm_profile.as_deref(),
        Some("backend-profile")
    );
    assert_eq!(
        calls.borrow()[0].reasoning_effort.as_deref(),
        Some("medium")
    );
    assert_eq!(execution.outcome.status, OutcomeStatus::Retry);
    assert_eq!(execution.outcome.suggested_next_ids, vec!["fallback_stage"]);
}
