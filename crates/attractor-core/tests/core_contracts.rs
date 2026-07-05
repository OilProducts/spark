use std::collections::BTreeMap;

use attractor_core::{
    apply_launch_context, attr_bool, attr_i64, attr_text, best_edge_by_weight_then_lexical,
    evaluate_condition, is_exact_outcome_fail_condition, node_has_explicit_attr,
    normalize_condition_literal, normalize_context_read_key, normalize_context_update_key,
    normalize_context_updates, normalize_label, parse_context_read_contract,
    parse_context_write_contract, resolve_context_read_contract, resolve_context_write_contract,
    routing_edge_from_dot_edge, select_failure_route_edge_with_context, select_next_edge,
    select_next_edge_with_condition_results, select_next_edge_with_context,
    validate_context_updates_against_contract, AttractorContext, AttractorCoreError, ContextMap,
    DotAttribute, DotEdge, DotValue, DotValueType, EdgeId, FailureKind, FlowName, GraphId,
    LaunchContext, NodeId, Outcome, OutcomeStatus, RawRuntimeEvent, RoutingEdge, RunId, RunRecord,
};
use serde_json::json;

fn context_map(entries: impl IntoIterator<Item = (&'static str, serde_json::Value)>) -> ContextMap {
    entries
        .into_iter()
        .map(|(key, value)| (key.to_string(), value))
        .collect()
}

fn edge(source: &str, target: &str) -> RoutingEdge {
    RoutingEdge::new(
        NodeId::try_from(source).expect("source id"),
        NodeId::try_from(target).expect("target id"),
    )
}

fn dot_string_attr(key: &str, value: &str) -> DotAttribute {
    DotAttribute {
        key: key.to_string(),
        value: DotValue::String(value.to_string()),
        value_type: DotValueType::String,
        line: 1,
    }
}

fn dot_bool_attr(key: &str, value: bool) -> DotAttribute {
    DotAttribute {
        key: key.to_string(),
        value: DotValue::Boolean(value),
        value_type: DotValueType::Boolean,
        line: 1,
    }
}

fn dot_int_attr(key: &str, value: i64) -> DotAttribute {
    DotAttribute {
        key: key.to_string(),
        value: DotValue::Integer(value),
        value_type: DotValueType::Integer,
        line: 1,
    }
}

#[test]
fn graph_attr_helpers_match_python_runtime_coercions() {
    let attrs = BTreeMap::from([
        ("enabled".to_string(), dot_bool_attr("enabled", true)),
        ("disabled".to_string(), dot_string_attr("disabled", "false")),
        (
            "string_true".to_string(),
            dot_string_attr("string_true", " true "),
        ),
        ("string_one".to_string(), dot_string_attr("string_one", "1")),
        ("count".to_string(), dot_int_attr("count", 3)),
        ("label".to_string(), dot_string_attr("label", "  Ship it  ")),
    ]);

    assert!(attr_bool(&attrs, "enabled", false));
    assert!(attr_bool(&attrs, "string_true", false));
    assert!(!attr_bool(&attrs, "disabled", true));
    assert!(!attr_bool(&attrs, "string_one", false));
    assert!(attr_bool(&attrs, "missing", true));
    assert_eq!(attr_i64(&attrs, "count", 0), 3);
    assert_eq!(attr_text(&attrs, "label").as_deref(), Some("Ship it"));

    let mut node = attractor_core::DotNode {
        node_id: "task".to_string(),
        attrs,
        line: 0,
        declaration_order: 0,
        explicit_attr_keys: ["enabled".to_string()].into_iter().collect(),
    };
    assert!(node_has_explicit_attr(&node, "enabled"));
    assert!(node_has_explicit_attr(&node, "label"));
    node.attrs.get_mut("label").expect("label").line = 0;
    assert!(!node_has_explicit_attr(&node, "label"));
}

#[test]
fn context_set_get_string_delete_batch_validation_and_isolation() {
    let mut context = AttractorContext::new();
    context.set("name", json!("spark")).expect("set string");
    context.set("attempts", json!(3)).expect("set number");
    context.set("enabled", json!(true)).expect("set bool");
    context
        .set("empty", serde_json::Value::Null)
        .expect("delete");
    context.append_log("first");

    assert_eq!(context.get("name"), Some(&json!("spark")));
    assert_eq!(context.get_string("name"), "spark");
    assert_eq!(context.get_string("attempts"), "3");
    assert_eq!(context.get_string("enabled"), "true");
    assert_eq!(context.get_string_or("missing", "fallback"), "fallback");
    assert!(!context.snapshot().contains_key("empty"));

    let updates = context_map([
        ("context.item.id", serde_json::Value::Null),
        ("context.item.title", json!("Retitled")),
        ("context.item.summary", serde_json::Value::Null),
    ]);
    context
        .set("context.item.id", json!("ITEM-1"))
        .expect("seed item id");
    context.apply_updates(&updates).expect("apply updates");
    assert_eq!(context.get("context.item.id"), None);
    assert_eq!(context.get("context.item.title"), Some(&json!("Retitled")));

    let invalid = context_map([
        ("context.ok", json!("yes")),
        ("tool.output", json!("blocked")),
    ]);
    let error = context.apply_updates(&invalid).expect_err("invalid batch");
    assert!(matches!(
        error,
        AttractorCoreError::InvalidContextNamespace { key } if key == "tool.output"
    ));
    assert_eq!(context.get("context.ok"), None);

    let mut clone = context.clone_isolated();
    clone
        .set("context.item.title", json!("Clone"))
        .expect("mutate clone");
    clone.append_log("second");
    assert_eq!(context.get("context.item.title"), Some(&json!("Retitled")));
    assert_eq!(context.logs(), ["first"]);
    assert_eq!(clone.logs(), ["first", "second"]);
}

#[test]
fn context_namespaces_and_path_lookup_match_runtime_contracts() {
    let mut context = AttractorContext::new();
    for (key, value) in [
        ("plain_key", json!("ok")),
        ("context.plan", json!("ready")),
        ("graph.goal", json!("ship")),
        ("internal.retry_count.task", json!(1)),
        ("parallel.results", json!([])),
        ("stack.supervisor", json!("manager")),
        ("human.gate.selected", json!("A")),
        ("work.item.id", json!("42")),
        ("_attractor.runtime.fidelity", json!("compact")),
    ] {
        context.set(key, value).expect("allowed namespace");
    }
    assert!(context.set("unknown.value", json!("nope")).is_err());

    let values = context_map([
        ("context.item.title", json!("prefixed flat wins")),
        ("context.item.id", json!("ITEM-1")),
        ("context", json!({"item": {"title": "Nested title"}})),
        ("object", json!({"enabled": true})),
    ]);
    let context = AttractorContext::from_map(values).expect("context map");
    assert_eq!(context.get_context_path("object.enabled"), "true");
    assert_eq!(context.get_context_path("item.id"), "ITEM-1");
    assert_eq!(context.get_context_path("item.title"), "prefixed flat wins");
    assert_eq!(
        context.get_context_path("context.item.title"),
        "prefixed flat wins"
    );
    assert_eq!(context.get_context_path("missing.path"), "");
}

#[test]
fn launch_context_only_accepts_context_namespace_and_applies_after_graph_values() {
    let launch = LaunchContext::new(context_map([
        ("context.request.id", json!("REQ-1")),
        ("context.metadata", json!({"source": "test"})),
    ]))
    .expect("valid launch context");
    let mut context = AttractorContext::new();
    context
        .set("graph.goal", json!("ship"))
        .expect("graph seed");
    apply_launch_context(&mut context, &launch).expect("apply launch");

    assert_eq!(context.get("graph.goal"), Some(&json!("ship")));
    assert_eq!(context.get("context.request.id"), Some(&json!("REQ-1")));

    let error =
        LaunchContext::new(context_map([("graph.goal", json!("override"))])).expect_err("graph");
    assert!(matches!(
        error,
        AttractorCoreError::InvalidLaunchContextKey { key, .. } if key == "graph.goal"
    ));
}

#[test]
fn context_contracts_parse_normalize_sort_and_report_violations() {
    let write = parse_context_write_contract(Some(r#"["review.summary","missing_prerequisites"]"#));
    assert_eq!(write.parse_error, "");
    assert_eq!(
        write.allowed_keys,
        ["context.review.summary", "missing_prerequisites"]
    );
    assert_eq!(
        normalize_context_update_key("review.summary"),
        "context.review.summary"
    );
    assert_eq!(
        normalize_context_update_key("runtime/state.json"),
        "runtime/state.json"
    );

    let read = parse_context_read_contract(Some(
        r#"["review.summary","custom.live.binding","internal.run_id"]"#,
    ));
    assert_eq!(
        read.declared_keys,
        [
            "context.review.summary",
            "custom.live.binding",
            "internal.run_id"
        ]
    );
    assert_eq!(
        normalize_context_read_key("custom.live.binding"),
        "custom.live.binding"
    );

    let malformed = parse_context_write_contract(Some(r#"{"bad":true}"#));
    assert_eq!(malformed.allowed_keys, Vec::<String>::new());
    assert_eq!(malformed.parse_error, "expected a JSON array of strings");

    let invalid = parse_context_write_contract(Some(r#"["runtime/state.json"]"#));
    assert_eq!(
        invalid.parse_error,
        "invalid context update key 'runtime/state.json': path separators are not allowed"
    );

    let contract = parse_context_write_contract(Some(
        r#"["context.review.summary","context.review.required_changes","missing_prerequisites"]"#,
    ));
    let violation = validate_context_updates_against_contract(
        &context_map([
            ("review.summary", json!("ready")),
            ("context.review.extra", json!("nope")),
            ("runtime/state.json", json!("bad")),
            ("missing_prerequisites", json!([])),
        ]),
        &contract,
    )
    .expect("violation");
    assert_eq!(violation.offending_keys, ["context.review.extra"]);
    assert_eq!(violation.invalid_keys, ["runtime/state.json"]);
    assert_eq!(
        violation.allowed_keys,
        [
            "context.review.required_changes",
            "context.review.summary",
            "missing_prerequisites"
        ]
    );
}

#[test]
fn graph_attr_contract_resolvers_and_context_update_normalization_are_runtime_ready() {
    let attrs = BTreeMap::from([
        (
            "spark.writes_context".to_string(),
            dot_string_attr("spark.writes_context", r#"["review.summary","flat.key"]"#),
        ),
        (
            "spark.reads_context".to_string(),
            dot_string_attr(
                "spark.reads_context",
                r#"["request.id","custom.live.binding","internal.run_id"]"#,
            ),
        ),
    ]);

    let write = resolve_context_write_contract(&attrs);
    assert_eq!(
        write.allowed_keys,
        ["context.flat.key", "context.review.summary"]
    );
    let read = resolve_context_read_contract(&attrs);
    assert_eq!(
        read.declared_keys,
        [
            "context.request.id",
            "custom.live.binding",
            "internal.run_id"
        ]
    );

    let normalized = normalize_context_updates(&context_map([
        ("review.summary", json!("ready")),
        ("context.keep", json!(true)),
        ("artifact_path", json!("bare stays bare")),
    ]));
    assert_eq!(normalized["context.review.summary"], json!("ready"));
    assert_eq!(normalized["context.keep"], json!(true));
    assert_eq!(normalized["artifact_path"], json!("bare stays bare"));
}

#[test]
fn outcome_payload_round_trips_and_omits_runtime_only_fields() {
    let mut outcome = Outcome::new(OutcomeStatus::Fail);
    outcome.preferred_label = "Fix".to_string();
    outcome.suggested_next_ids = vec!["retry_stage".to_string(), "fallback_stage".to_string()];
    outcome.context_updates = context_map([
        ("context.retry_count", json!(2)),
        ("context.metadata", json!({"source": "validator"})),
    ]);
    outcome.notes = "validation failed".to_string();
    outcome.failure_reason = "lint failed".to_string();
    outcome.failure_kind = Some(FailureKind::Contract);
    outcome.retryable = Some(false);
    outcome.raw_response_text = r#"{"outcome":"fail"}"#.to_string();

    let payload = outcome.to_payload();
    let payload_json = serde_json::to_value(&payload).expect("payload json");
    assert_eq!(
        payload_json,
        json!({
            "status": "fail",
            "preferred_label": "Fix",
            "suggested_next_ids": ["retry_stage", "fallback_stage"],
            "context_updates": {
                "context.metadata": {"source": "validator"},
                "context.retry_count": 2
            },
            "notes": "validation failed",
            "failure_reason": "lint failed",
            "failure_kind": "contract"
        })
    );
    assert!(payload_json.get("retryable").is_none());
    assert!(payload_json.get("raw_response_text").is_none());

    let skipped = Outcome::new(OutcomeStatus::Skipped).to_payload();
    assert_eq!(
        serde_json::to_value(skipped).expect("skipped"),
        json!({
            "status": "skipped",
            "preferred_label": "",
            "suggested_next_ids": [],
            "context_updates": {},
            "notes": "",
            "failure_reason": ""
        })
    );
}

#[test]
fn identifier_newtypes_reject_empty_text_and_serialize_as_strings() {
    assert!(GraphId::try_from("").is_err());
    assert!(NodeId::try_from("  ").is_err());
    assert_eq!(
        serde_json::to_value(NodeId::try_from("plan").expect("node")).expect("node json"),
        json!("plan")
    );
    assert_eq!(
        serde_json::to_value(RunId::try_from("run-1").expect("run")).expect("run json"),
        json!("run-1")
    );
    assert_eq!(
        FlowName::try_from("starter").expect("flow").as_str(),
        "starter"
    );
    assert_eq!(
        EdgeId::try_from(" edge-1 ").expect("edge").as_str(),
        "edge-1"
    );
}

#[test]
fn routing_normalizes_labels_preserves_suggestion_order_and_tiebreaks() {
    assert_eq!(normalize_label("[Y] Approve"), "approve");
    assert_eq!(normalize_label("Y) Approve"), "approve");
    assert_eq!(normalize_label("Y - Approve"), "approve");

    let mut c2 = edge("a", "c2");
    c2.condition = "outcome=success".to_string();
    c2.weight = 4;
    let mut c1 = edge("a", "c1");
    c1.condition = "outcome=success".to_string();
    c1.weight = 4;
    let mut label_hit = edge("a", "label_hit");
    label_hit.label = "[Y] Approve".to_string();
    label_hit.weight = 99;
    let mut suggested_hit = edge("a", "suggested_hit");
    suggested_hit.weight = 50;

    let outcome = Outcome {
        preferred_label: "Approve".to_string(),
        suggested_next_ids: vec!["suggested_hit".to_string()],
        ..Outcome::new(OutcomeStatus::Success)
    };
    let mut conditions = BTreeMap::new();
    conditions.insert("outcome=success".to_string(), true);
    let selected = select_next_edge_with_condition_results(
        &[
            c2.clone(),
            c1.clone(),
            label_hit.clone(),
            suggested_hit.clone(),
        ],
        &outcome,
        &conditions,
    )
    .expect("condition selected");
    assert_eq!(selected.target.as_str(), "c1");

    let preferred = select_next_edge(&[label_hit.clone(), suggested_hit.clone()], &outcome)
        .expect("preferred selected");
    assert_eq!(preferred.target.as_str(), "label_hit");

    let suggested = Outcome {
        suggested_next_ids: vec!["missing".to_string(), "suggested_hit".to_string()],
        ..Outcome::new(OutcomeStatus::Fail)
    };
    let selected = select_next_edge(&[label_hit, suggested_hit.clone()], &suggested)
        .expect("suggested selected");
    assert_eq!(selected.target.as_str(), "suggested_hit");

    let mut beta = edge("a", "beta");
    beta.weight = 7;
    let mut alpha = edge("a", "alpha");
    alpha.weight = 7;
    let best = best_edge_by_weight_then_lexical(&[beta, alpha]).expect("best edge");
    assert_eq!(best.target.as_str(), "alpha");
}

#[test]
fn conditions_match_python_resolution_and_literal_rules() {
    let context: AttractorContext = serde_json::from_value(json!({
        "values": {
            "context.flag": true,
            "context.flat.key": "flat",
            "flat.direct": "direct",
            "context": {"nested": {"value": "nested"}},
            "number": 3,
            "enabled": true,
            "phrase": "a && b",
            "escaped": "say \"hi\" \\ end"
        },
        "logs": []
    }))
    .expect("context");
    let outcome = Outcome {
        status: OutcomeStatus::PartialSuccess,
        preferred_label: "[Y] Ship".to_string(),
        ..Outcome::new(OutcomeStatus::PartialSuccess)
    };

    assert!(evaluate_condition("", &outcome, &context));
    assert!(evaluate_condition(
        r#"outcome=partial_success && preferred_label="[Y] Ship""#,
        &outcome,
        &context
    ));
    assert!(evaluate_condition(
        r#"context.flag=true && number=3 && enabled"#,
        &outcome,
        &context
    ));
    assert!(evaluate_condition(
        "context.nested.value=nested",
        &outcome,
        &context
    ));
    assert!(evaluate_condition("flat.direct=direct", &outcome, &context));
    assert!(evaluate_condition(
        "context.flat.key=flat",
        &outcome,
        &context
    ));
    assert!(!evaluate_condition("flat.key=flat", &outcome, &context));
    assert!(evaluate_condition(r#"phrase="a && b""#, &outcome, &context));
    assert!(evaluate_condition(
        r#"escaped="say \"hi\" \\ end""#,
        &outcome,
        &context
    ));
    assert!(!evaluate_condition(
        "context.flag ~~ true",
        &outcome,
        &context
    ));
    assert_eq!(
        normalize_condition_literal(r#""a \"b\" \\ c""#),
        "a \"b\" \\ c"
    );
}

#[test]
fn context_aware_routing_preserves_condition_failure_and_dot_edge_priority() {
    let context = AttractorContext::from_map(context_map([
        ("context.ready", json!(true)),
        ("context.error", json!(true)),
    ]))
    .expect("context");
    let outcome = Outcome {
        preferred_label: "Approve".to_string(),
        suggested_next_ids: vec!["suggested".to_string()],
        ..Outcome::new(OutcomeStatus::Success)
    };

    let mut conditional = edge("a", "conditional");
    conditional.condition = "context.ready=true".to_string();
    conditional.weight = 1;
    let mut unconditional = edge("a", "unconditional");
    unconditional.label = "[Y] Approve".to_string();
    unconditional.weight = 99;
    let selected = select_next_edge_with_context(
        &[unconditional.clone(), conditional.clone()],
        &outcome,
        &context,
    )
    .expect("condition wins");
    assert_eq!(selected.target.as_str(), "conditional");

    conditional.condition = "context.missing=true".to_string();
    assert!(select_next_edge_with_context(&[conditional.clone()], &outcome, &context).is_none());

    let mut exact_fail = edge("a", "exact_fail");
    exact_fail.condition = " outcome = fail ".to_string();
    exact_fail.weight = 1;
    let mut general_fail = edge("a", "general_fail");
    general_fail.condition = "context.error=true".to_string();
    general_fail.weight = 100;
    let selected = select_failure_route_edge_with_context(
        &[general_fail, exact_fail],
        &Outcome::new(OutcomeStatus::Fail),
        &context,
    )
    .expect("failure route");
    assert_eq!(selected.target.as_str(), "exact_fail");
    assert!(is_exact_outcome_fail_condition(" outcome = fail "));
    assert!(is_exact_outcome_fail_condition("OUTCOME=FAIL"));

    let dot_edge = DotEdge {
        source: "source".to_string(),
        target: "target".to_string(),
        attrs: BTreeMap::from([
            ("label".to_string(), dot_string_attr("label", "ship")),
            (
                "condition".to_string(),
                dot_string_attr("condition", "outcome=success"),
            ),
            ("weight".to_string(), dot_string_attr("weight", "7")),
        ]),
        line: 1,
    };
    let routing_edge = routing_edge_from_dot_edge(&dot_edge).expect("routing edge");
    assert_eq!(routing_edge.source.as_str(), "source");
    assert_eq!(routing_edge.target.as_str(), "target");
    assert_eq!(routing_edge.label, "ship");
    assert_eq!(routing_edge.condition, "outcome=success");
    assert_eq!(routing_edge.weight, 7);
}

#[test]
fn run_record_serializes_api_observed_nulls_and_provider_aliases() {
    let mut record = RunRecord::new("run-1", "/tmp/project");
    record.flow_name = "flow-a".to_string();
    record.model = "gpt-compat".to_string();
    record.llm_provider = "openai".to_string();
    record.provider = "openai".to_string();

    let value = serde_json::to_value(&record).expect("record json");

    assert_eq!(value["run_id"], "run-1");
    assert_eq!(value["status"], "running");
    assert_eq!(value["provider"], "openai");
    assert_eq!(value["llm_provider"], "openai");
    assert!(value["outcome"].is_null());
    assert!(value["llm_profile"].is_null());
    assert!(value["continued_from_run_id"].is_null());
    assert!(value["token_usage_breakdown"].is_null());

    let legacy = json!({
        "run_id": "run-legacy",
        "status": "success",
        "provider": "codex",
        "working_directory": "/tmp/project"
    });
    let loaded: RunRecord = serde_json::from_value(legacy).expect("legacy record");
    assert_eq!(loaded.run_id, "run-legacy");
    assert_eq!(loaded.provider, "codex");
    assert_eq!(loaded.llm_provider, "");
    assert_eq!(loaded.execution_mode, "native");
}

#[test]
fn checkpoint_event_and_journal_dtos_round_trip_with_payloads() {
    let checkpoint = attractor_core::CheckpointState {
        timestamp: "2026-06-22T17:40:17Z".to_string(),
        current_node: "done".to_string(),
        completed_nodes: vec!["start".to_string()],
        context: context_map([
            ("context.topic", json!("compat")),
            ("_attractor.runtime.retry.attempt", json!(0)),
        ]),
        retry_counts: BTreeMap::from([("work".to_string(), 1)]),
        logs: vec!["first".to_string()],
    };
    let checkpoint_json = serde_json::to_value(&checkpoint).expect("checkpoint json");
    assert_eq!(checkpoint_json["current_node"], "done");
    assert_eq!(checkpoint_json["completed_nodes"], json!(["start"]));
    let loaded: attractor_core::CheckpointState =
        serde_json::from_value(checkpoint_json).expect("checkpoint round trip");
    assert_eq!(loaded.retry_counts["work"], 1);

    let mut event = RawRuntimeEvent::new("StageCompleted", "run-1");
    event.sequence = Some(42);
    event.emitted_at = "2026-06-22T17:40:17Z".to_string();
    event.payload.insert("node_id".to_string(), json!("work"));
    event
        .payload
        .insert("outcome".to_string(), json!("success"));
    let event_json = serde_json::to_value(&event).expect("event json");
    assert_eq!(event_json["type"], "StageCompleted");
    assert_eq!(event_json["node_id"], "work");

    let journal = attractor_core::JournalEntry {
        id: "journal-42".to_string(),
        sequence: 42,
        emitted_at: "2026-06-22T17:40:17Z".to_string(),
        kind: "stage".to_string(),
        raw_type: "StageCompleted".to_string(),
        severity: "info".to_string(),
        summary: "Stage work completed (success)".to_string(),
        node_id: Some("work".to_string()),
        stage_index: Some(1),
        source_scope: "root".to_string(),
        source_parent_node_id: None,
        source_flow_name: None,
        question_id: None,
        payload: event_json.clone(),
    };
    let journal_json = serde_json::to_value(&journal).expect("journal json");
    assert_eq!(journal_json["id"], "journal-42");
    assert!(journal_json["question_id"].is_null());
    assert_eq!(journal_json["payload"], event_json);
}
