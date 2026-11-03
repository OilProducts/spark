use std::collections::BTreeMap;

use attractor_core::{
    apply_launch_context, attr_bool, attr_i64, attr_text, evaluate_condition,
    node_has_explicit_attr, normalize_condition_literal, normalize_context_read_key,
    normalize_context_update_key, normalize_context_updates, parse_context_read_contract,
    parse_context_write_contract, resolve_context_read_contract, resolve_context_write_contract,
    routing_edge_from_dot_edge, validate_context_updates_against_contract, AttractorContext,
    AttractorCoreError, ContextMap, DotAttribute, DotEdge, DotValue, DotValueType, EdgeId,
    FailureKind, FlowName, GraphId, LaunchContext, NodeId, Outcome, OutcomeStatus, RawRuntimeEvent,
    RunId, RunRecord,
};
use serde_json::json;

fn context_map(entries: impl IntoIterator<Item = (&'static str, serde_json::Value)>) -> ContextMap {
    entries
        .into_iter()
        .map(|(key, value)| (key.to_string(), value))
        .collect()
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
        ("_attractor.runtime.sessions", json!({})),
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
fn outcomes_separate_completed_decisions_from_classified_errors() {
    let mut completed = Outcome::completed("ready");
    completed.context_updates = context_map([("context.review.summary", json!("ok"))]);
    completed.notes = "reviewed".to_string();
    let value = serde_json::to_value(&completed).expect("completed json");
    assert_eq!(value["status"], json!("completed"));
    assert_eq!(value["decision"], json!("ready"));
    assert!(value.get("failure_kind").is_none());
    assert!(value.get("preferred_label").is_none());
    assert_eq!(
        serde_json::from_value::<Outcome>(value).expect("round trip"),
        completed
    );
    assert!(!completed.is_error());
    assert_eq!(completed.error_kind(), None);

    let error = Outcome::error(FailureKind::Contract, "bad envelope", Some(false));
    let value = serde_json::to_value(&error).expect("error json");
    assert_eq!(value["status"], json!("error"));
    assert_eq!(value["decision"], json!(""));
    assert_eq!(value["failure_kind"], json!("contract"));
    assert_eq!(value["retryable"], json!(false));
    assert_eq!(error.error_kind(), Some(FailureKind::Contract));
    // An unclassified error is a runtime error.
    assert_eq!(
        Outcome::new(OutcomeStatus::Error).error_kind(),
        Some(FailureKind::Runtime)
    );

    // Only routing errors are never tolerable inside a fork.
    assert!(FailureKind::Runtime.is_tolerable());
    assert!(FailureKind::Contract.is_tolerable());
    assert!(!FailureKind::Routing.is_tolerable());

    // The legacy statuses are not outcome statuses any more.
    for legacy in ["success", "fail", "partial_success", "retry", "skipped"] {
        assert!(legacy.parse::<OutcomeStatus>().is_err(), "{legacy}");
        assert!(serde_json::from_value::<Outcome>(json!({"status": legacy})).is_err());
    }
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
fn switch_cases_compare_exact_json_scalars_without_coercion() {
    use attractor_core::{decode_case_key, switch_decision, SwitchScalar};
    let cases = BTreeMap::from([
        ("0".to_string(), "clean".to_string()),
        ("true".to_string(), "flagged".to_string()),
        (r#""0""#.to_string(), "text_zero".to_string()),
        (r#""ready""#.to_string(), "ready".to_string()),
    ]);
    let pick = |value: serde_json::Value| switch_decision(&cases, "other", &value);
    assert_eq!(pick(json!(0)).as_deref(), Ok("clean"));
    // Numerically equal spellings are the same number.
    assert_eq!(pick(json!(0.0)).as_deref(), Ok("clean"));
    assert_eq!(pick(json!(true)).as_deref(), Ok("flagged"));
    assert_eq!(pick(json!("0")).as_deref(), Ok("text_zero"));
    assert_eq!(pick(json!("ready")).as_deref(), Ok("ready"));
    // No truthiness or string/number/boolean coercion: unmatched scalars take otherwise.
    assert_eq!(pick(json!(3)).as_deref(), Ok("other"));
    assert_eq!(pick(json!(false)).as_deref(), Ok("other"));
    assert_eq!(pick(json!("true")).as_deref(), Ok("other"));
    assert_eq!(pick(json!("")).as_deref(), Ok("other"));
    // Unsupported values are errors, never otherwise.
    for value in [json!(null), json!([0]), json!({"count": 0})] {
        assert!(pick(value.clone()).is_err(), "{value}");
    }

    assert_eq!(decode_case_key("0"), Ok(SwitchScalar::Number(0.0)));
    assert_eq!(
        decode_case_key(r#""0""#),
        Ok(SwitchScalar::String("0".to_string()))
    );
    for invalid in ["null", "[1]", "{}", "ready", "1e999"] {
        assert!(decode_case_key(invalid).is_err(), "{invalid}");
    }
    assert!(SwitchScalar::Number(0.0).same(&SwitchScalar::Number(-0.0)));
    assert!(!SwitchScalar::Number(1.0).same(&SwitchScalar::String("1".to_string())));
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

    assert!(evaluate_condition("", &context));
    // Conditions read context only; there is no inherited outcome or label.
    assert!(!evaluate_condition("outcome=success", &context));
    assert!(!evaluate_condition(r#"preferred_label="Ship""#, &context));
    assert!(evaluate_condition(
        r#"context.flag=true && number=3 && enabled"#,
        &context
    ));
    assert!(evaluate_condition("context.nested.value=nested", &context));
    assert!(evaluate_condition("flat.direct=direct", &context));
    assert!(evaluate_condition("context.flat.key=flat", &context));
    assert!(!evaluate_condition("flat.key=flat", &context));
    assert!(evaluate_condition(r#"phrase="a && b""#, &context));
    assert!(evaluate_condition(
        r#"escaped="say \"hi\" \\ end""#,
        &context
    ));
    assert!(!evaluate_condition("context.flag ~~ true", &context));
    assert_eq!(
        normalize_condition_literal(r#""a \"b\" \\ c""#),
        "a \"b\" \\ c"
    );
}

#[test]
fn dot_projection_carries_each_route_decision_description_and_label() {
    let dot_edge = DotEdge {
        source: "source".to_string(),
        target: "target".to_string(),
        attrs: BTreeMap::from([
            ("label".to_string(), dot_string_attr("label", "Ship")),
            ("decision".to_string(), dot_string_attr("decision", "ship")),
            (
                "description".to_string(),
                dot_string_attr("description", "Ready to ship."),
            ),
        ]),
        line: 1,
    };
    let routing_edge = routing_edge_from_dot_edge(&dot_edge).expect("routing edge");
    assert_eq!(routing_edge.source.as_str(), "source");
    assert_eq!(routing_edge.target.as_str(), "target");
    assert_eq!(routing_edge.decision, "ship");
    assert_eq!(routing_edge.description, "Ready to ship.");
    assert_eq!(routing_edge.display_label(), "Ship");

    let flow: attractor_core::FlowDefinition = serde_json::from_value(json!({
        "schema_version": "2", "id": "dot",
        "nodes": {
            "start": {"kind": "start", "routes": {"next": {"to": "gate", "description": "Enter."}}},
            "gate": {"kind": "human_gate", "routes": {
                "approve": {"to": "done", "label": "Approve", "description": "Approved."},
                "stop": {"to": "done", "description": "Stopped."}
            }},
            "done": {"kind": "exit", "config": {"kind": "exit", "result": {"name": "completed", "outcome": "success"}}}
        }
    }))
    .expect("flow");
    let graph = flow.to_runtime_dot_graph();
    let edges = graph
        .edges
        .iter()
        .map(|edge| {
            let edge = routing_edge_from_dot_edge(edge).expect("edge");
            (
                edge.source.to_string(),
                edge.decision.clone(),
                edge.target.to_string(),
                edge.display_label().to_string(),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        edges,
        vec![
            (
                "gate".into(),
                "approve".into(),
                "done".into(),
                "Approve".into()
            ),
            ("gate".into(), "stop".into(), "done".into(), "stop".into()),
            ("start".into(), "next".into(), "gate".into(), "next".into()),
        ]
    );
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
        visit_counts: Default::default(),
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

#[test]
fn node_runtime_timeout_accepts_durations_and_rejects_invalid_values() {
    let timeout = |raw: Option<&str>| {
        attractor_core::NodeRuntimeConfig {
            timeout: raw.map(str::to_string),
            ..Default::default()
        }
        .timeout_seconds()
    };
    assert_eq!(timeout(None), Ok(None));
    for (raw, seconds) in [
        ("30", 30.0),
        ("1.5", 1.5),
        ("250ms", 0.25),
        ("90s", 90.0),
        (" 15m ", 900.0),
        ("2h", 7200.0),
        ("1d", 86400.0),
        ("365d", 365.0 * 86400.0),
    ] {
        assert_eq!(timeout(Some(raw)), Ok(Some(seconds)), "{raw}");
    }
    for raw in [
        "", "abc", "10x", "s", "0", "0s", "-5", "-1m", "nan", "inf", "infinity", "1e100s", "366d",
    ] {
        let error = timeout(Some(raw)).expect_err(raw);
        assert!(error.contains("runtime.timeout"), "{error}");
    }

    let flow = serde_json::from_value::<attractor_core::FlowDefinition>(json!({
        "schema_version": "2",
        "id": "timeouts",
        "nodes": {
            "start": { "kind": "start", "routes": { "next": { "to": "work", "description": "Enter." } } },
            "work": { "kind": "agent_task", "runtime": { "timeout": "1e100s" },
                "contracts": { "response": "decision_envelope" },
                "routes": { "done": { "to": "done", "description": "Done." } } },
            "done": { "kind": "exit", "config": { "kind": "exit", "result": { "name": "completed", "outcome": "success" } } }
        }
    }))
    .unwrap();
    let diagnostics = flow.diagnostics();
    assert!(
        diagnostics.iter().any(|d| d.rule_id == "node_timeout"
            && d.node_id.as_deref() == Some("work")
            && d.message.contains("1e100s")),
        "{diagnostics:?}"
    );
}

#[test]
fn execution_locks_require_project_scope_a_key_and_queue_policy() {
    let rules = |lock: serde_json::Value| {
        serde_json::from_value::<attractor_core::FlowDefinition>(json!({
            "schema_version": "2",
            "id": "locked",
            "execution_lock": lock,
            "nodes": {
                "start": { "kind": "start", "routes": { "next": { "to": "done", "description": "Enter." } } },
                "done": { "kind": "exit", "config": { "kind": "exit", "result": { "name": "completed", "outcome": "success" } } }
            }
        }))
        .unwrap()
        .diagnostics()
        .into_iter()
        .filter(|d| d.rule_id == "execution_lock")
        .map(|d| d.message)
        .collect::<Vec<_>>()
    };
    assert_eq!(
        rules(json!({ "scope": "project", "key": "integration" })),
        Vec::<String>::new()
    );
    assert_eq!(
        rules(json!({ "scope": "Project", "key": "integration", "conflict_policy": "QUEUE" })),
        Vec::<String>::new()
    );
    assert_eq!(
        rules(json!({ "scope": "global", "key": "integration" })).len(),
        1
    );
    assert_eq!(rules(json!({ "scope": "project", "key": " " })).len(), 1);
    assert_eq!(
        rules(json!({ "scope": "project", "key": "integration", "conflict_policy": "reject" }))
            .len(),
        1
    );
}

#[test]
fn sessions_output_maps_and_removed_fidelity_fields_are_validated() {
    let route = |to: &str| json!({"done": {"to": to, "description": "Done."}});
    let exit = json!({"kind": "exit", "config": {"kind": "exit", "result": {"name": "completed", "outcome": "success"}}});
    let start = json!({"kind": "start", "routes": {"next": {"to": "a", "description": "Enter."}}});
    // start -> a -> b -> done, with `a` and `b` given by the caller.
    let linear = |a: serde_json::Value, b: serde_json::Value| {
        let mut a = a;
        let mut b = b;
        a["routes"] = route("b");
        if b.get("routes").is_none() {
            b["routes"] = route("done");
        }
        json!({"start": start, "done": exit, "a": a, "b": b})
    };
    let rules = |nodes: serde_json::Value| {
        serde_json::from_value::<attractor_core::FlowDefinition>(json!({
            "schema_version": "2", "id": "sessions", "nodes": nodes
        }))
        .expect("flow JSON")
        .diagnostics()
        .into_iter()
        .map(|d| (d.rule_id, d.message))
        .collect::<Vec<_>>()
    };
    let agent = |runtime: serde_json::Value, execution: serde_json::Value| {
        json!({"kind": "agent_task", "runtime": runtime, "execution": execution,
            "contracts": {"response": "decision_envelope"}})
    };
    let tool = |runtime: serde_json::Value| {
        json!({"kind": "tool", "runtime": runtime,
            "config": {"kind": "tool", "command": "true", "exit_codes": {"0": "done"}}})
    };

    // Shared sessions with matching (or unset) providers and models are valid.
    assert_eq!(
        rules(linear(
            agent(
                json!({"session": "author"}),
                json!({"llm_provider": "codex"})
            ),
            agent(json!({"session": "author"}), json!({})),
        )),
        Vec::new()
    );

    // Conflicting providers or models in one session are rejected.
    let diagnostics = rules(linear(
        agent(
            json!({"session": "author"}),
            json!({"llm_provider": "codex", "llm_model": "m1"}),
        ),
        agent(
            json!({"session": "author"}),
            json!({"llm_provider": "claude-code", "llm_model": "m2"}),
        ),
    ));
    assert_eq!(diagnostics.len(), 2, "{diagnostics:?}");
    assert!(diagnostics
        .iter()
        .all(|(rule, _)| rule == "session_conflict"));
    assert!(diagnostics[0].1.contains("llm_provider") && diagnostics[1].1.contains("llm_model"));

    // `session` is only for agent tasks, and its name must be a valid id.
    let diagnostics = rules(linear(
        tool(json!({"session": "author"})),
        agent(json!({"session": "bad name"}), json!({})),
    ));
    assert!(
        diagnostics
            .iter()
            .any(|(rule, message)| rule == "session"
                && message.contains("only allowed on agent_task"))
    );
    assert!(diagnostics
        .iter()
        .any(|(rule, message)| rule == "session" && message.contains("invalid session name")));

    // `workdir_from` is only for agent tasks, and must be a valid context key.
    assert_eq!(
        rules(linear(
            agent(json!({"workdir_from": "context.workspace.path"}), json!({})),
            agent(json!({}), json!({}))
        )),
        Vec::new()
    );
    let diagnostics = rules(linear(
        tool(json!({"workdir_from": "context.workspace.path"})),
        agent(json!({"workdir_from": "invalid.namespace"}), json!({})),
    ));
    assert_eq!(
        diagnostics
            .iter()
            .filter(|(rule, _)| rule == "workdir_from")
            .count(),
        2,
        "{diagnostics:?}"
    );
    assert!(diagnostics
        .iter()
        .any(|(rule, message)| rule == "workdir_from"
            && message.contains("only allowed on agent_task")));

    // Every output_map parent key must be declared in writes_context.
    let subflow = |writes: serde_json::Value| {
        linear(
            json!({"kind": "subflow", "config": {"kind": "subflow", "flow_ref": "child",
                "output_map": {"context.child.result": "context.result"},
                "results": {"completed": "done"}},
                "contracts": {"writes_context": writes}}),
            agent(json!({}), json!({})),
        )
    };
    assert_eq!(rules(subflow(json!(["context.result"]))), Vec::new());
    assert!(rules(subflow(json!([])))
        .iter()
        .any(|(rule, message)| rule == "subflow_output_map" && message.contains("context.result")));

    // The removed fidelity and thread_id settings fail and are named.
    for (nodes, defaults) in [
        (
            json!({"a": agent(json!({"fidelity": "full"}), json!({}))}),
            json!({}),
        ),
        (
            json!({"a": agent(json!({"thread_id": "t"}), json!({}))}),
            json!({}),
        ),
        (json!({}), json!({"fidelity": "summary:high"})),
    ] {
        let error = serde_json::from_value::<attractor_core::FlowDefinition>(
            json!({"schema_version": "2", "id": "old", "nodes": nodes, "defaults": defaults}),
        )
        .expect_err("removed field");
        assert!(
            error.to_string().contains("unknown field `fidelity`")
                || error.to_string().contains("unknown field `thread_id`"),
            "{error}"
        );
    }
    let mut nodes = linear(agent(json!({}), json!({})), agent(json!({}), json!({})));
    nodes["a"]["routes"]["done"]["extensions"] = json!({"fidelity": "full"});
    nodes["b"]["extensions"] = json!({"thread_id": "t"});
    let diagnostics = rules(nodes);
    assert!(diagnostics
        .iter()
        .any(|(rule, message)| rule == "removed_field" && message.contains("thread_id")));
}
