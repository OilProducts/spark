use attractor_core::FlowDefinition;
use serde_json::{json, Value};

fn exit(name: &str, outcome: &str) -> Value {
    json!({"kind": "exit", "config": {"kind": "exit", "result": {"name": name, "outcome": outcome}}})
}

fn route(to: &str) -> Value {
    json!({"to": to, "description": format!("Go to {to}.")})
}

fn agent(routes: Value) -> Value {
    json!({"kind": "agent_task", "contracts": {"response": "decision_envelope"}, "routes": routes})
}

fn start(to: &str) -> Value {
    json!({"kind": "start", "routes": {"next": route(to)}})
}

fn flow(nodes: Value) -> Value {
    json!({"schema_version": "2", "id": "checked", "nodes": nodes})
}

fn rules(value: Value) -> Vec<(String, String)> {
    serde_json::from_value::<FlowDefinition>(value)
        .expect("flow JSON")
        .diagnostics()
        .into_iter()
        .map(|diagnostic| (diagnostic.rule_id, diagnostic.message))
        .collect()
}

fn has(rules: &[(String, String)], rule: &str, text: &str) -> bool {
    rules
        .iter()
        .any(|(id, message)| id == rule && message.contains(text))
}

/// A valid two-agent loop: work may stay unfinished on itself, review may
/// send work back, and both may block to a failure exit.
fn review_loop() -> Value {
    flow(json!({
        "start": start("work"),
        "work": agent(json!({"ready_for_review": route("review"), "unfinished": route("work"), "blocked": route("failed")})),
        "review": agent(json!({"ready": route("done"), "fix": route("work"), "blocked": route("failed")})),
        "done": exit("completed", "success"),
        "failed": exit("blocked", "failure"),
    }))
}

#[test]
fn bounded_cycles_and_unfinished_self_loops_validate_and_round_trip() {
    let mut value = review_loop();
    value["nodes"]["work"]["runtime"] = json!({"max_visits": 5});
    assert_eq!(rules(value.clone()), Vec::new());
    let parsed: FlowDefinition = serde_json::from_value(value).expect("flow");
    let canonical = parsed.to_canonical_json_value();
    assert_eq!(
        canonical["nodes"]["work"]["routes"]["unfinished"]["to"],
        json!("work")
    );
    assert!(canonical.get("edges").is_none());
    let reparsed: FlowDefinition = serde_json::from_value(canonical).expect("canonical");
    assert_eq!(reparsed, parsed.normalize());
    assert_eq!(
        parsed.route("review", "fix").map(|r| r.to.as_str()),
        Some("work")
    );
    assert_eq!(parsed.route("review", "Fix"), None);
}

#[test]
fn version_and_legacy_fields_are_rejected_not_dropped() {
    let mut old = review_loop();
    old["schema_version"] = json!("1");
    assert!(has(&rules(old), "schema_version", "must be '2'"));

    let mut missing = review_loop();
    missing.as_object_mut().unwrap().remove("schema_version");
    assert!(serde_json::from_value::<FlowDefinition>(missing).is_err());

    for (path, field) in [
        ("", "edges"),
        ("/nodes/work", "condition"),
        ("/nodes/work/routes/unfinished", "weight"),
        ("/nodes/work/routes/unfinished", "condition"),
    ] {
        let mut value = review_loop();
        value.pointer_mut(path).unwrap()[field] = json!([]);
        assert!(
            serde_json::from_value::<FlowDefinition>(value).is_err(),
            "{path} {field}"
        );
    }
    for legacy in ["error_policy", "retry_target", "goal_gate", "allow_partial"] {
        let mut value = review_loop();
        value["nodes"]["work"]["runtime"] = json!({legacy: true});
        assert!(
            serde_json::from_value::<FlowDefinition>(value).is_err(),
            "{legacy}"
        );
    }
    // Legacy routing keys hidden in extension bags are rejected too.
    let mut value = review_loop();
    value["nodes"]["work"]["extensions"] = json!({"preferred_label": "Fix"});
    value["nodes"]["review"]["routes"]["fix"]["extensions"] = json!({"weight": 1});
    let found = rules(value);
    assert!(has(&found, "removed_field", "preferred_label"), "{found:?}");
    assert!(has(&found, "removed_field", "weight"), "{found:?}");
}

#[test]
fn typed_sections_reject_unknown_and_misplaced_fields_and_open_maps_stay_open() {
    let parse_error = |value: Value| {
        FlowDefinition::from_json_str(&value.to_string())
            .expect_err("unknown field")
            .detail
    };
    // The misspelled field and the legacy routing fields of a real authoring slip.
    let mut value = review_loop();
    value["nodes"]["start"]["contracts"] =
        json!({"writes_contex": ["context.required"], "retry_target": "done"});
    assert!(parse_error(value).contains("unknown field `"));
    let mut value = review_loop();
    value["nodes"]["start"]["contracts"] = json!({"writes_contex": ["context.required"]});
    assert!(parse_error(value).contains("unknown field `writes_contex`"));

    for section in ["contracts", "manager", "retry", "execution", "ui"] {
        for field in ["retry_target", "goal_gate", "preferred_label", "typo"] {
            let mut value = review_loop();
            value["nodes"]["work"][section] = json!({field: true});
            let detail = parse_error(value);
            assert!(
                detail.contains(&format!("unknown field `{field}`")),
                "{section}.{field}: {detail}"
            );
        }
    }
    let mut value = review_loop();
    value["inputs"] = json!([{"key": "context.request", "goal_gate": true}]);
    assert!(parse_error(value).contains("unknown field `goal_gate`"));
    // Nodes no longer author context of their own.
    let mut value = review_loop();
    value["nodes"]["work"]["context"] = json!({"context.any": 1});
    assert!(parse_error(value).contains("unknown field `context`"));
    let mut value = review_loop();
    value["execution_lock"] = json!({"scope": "project", "key": "main", "retry_target": "done"});
    assert!(parse_error(value).contains("unknown field `retry_target`"));

    let mut open = review_loop();
    open["metadata"] = json!({"owner": {"team": "any"}});
    open["extensions"] = json!({"vendor.flag": true});
    open["defaults"] = json!({"extensions": {"vendor.flag": true}});
    open["nodes"]["work"]["extensions"] = json!({"vendor.flag": true});
    open["nodes"]["work"]["routes"]["unfinished"]["extensions"] = json!({"vendor.flag": true});
    let parsed = FlowDefinition::from_json_str(&open.to_string()).expect("open maps");
    assert_eq!(parsed.metadata["owner"], json!({"team": "any"}));
    assert_eq!(parsed.diagnostics(), Vec::new());
}

#[test]
fn routes_need_valid_ids_descriptions_and_existing_targets() {
    let mut value = review_loop();
    value["nodes"]["review"]["routes"] = json!({
        "Ready": route("done"),
        "fix": {"to": "work", "description": "  "},
        "blocked": route("nowhere"),
    });
    let found = rules(value);
    assert!(has(&found, "decision_id", "'Ready'"), "{found:?}");
    assert!(
        has(&found, "route_description", "review.md: decision 'fix'"),
        "{found:?}"
    );
    assert!(
        has(&found, "route_target", "targets 'nowhere'"),
        "{found:?}"
    );
    let diagnostic = serde_json::from_value::<FlowDefinition>({
        let mut value = review_loop();
        value["nodes"]["review"]["routes"]["blocked"] = route("nowhere");
        value
    })
    .unwrap()
    .diagnostics()
    .into_iter()
    .find(|d| d.rule_id == "route_target")
    .expect("target diagnostic");
    assert_eq!(diagnostic.node_id.as_deref(), Some("review"));
    assert_eq!(diagnostic.decision.as_deref(), Some("blocked"));
}

#[test]
fn kinds_declare_their_decision_domains() {
    // start and fan_in take exactly `next`; exits take none; agents need
    // the decision envelope and at least one decision.
    let found = rules(flow(json!({
        "start": {"kind": "start", "routes": {"go": route("work")}},
        "work": {"kind": "agent_task", "routes": {"done": route("done")}},
        "idle": agent(json!({})),
        "done": {"kind": "exit", "config": {"kind": "exit", "result": {"name": "completed", "outcome": "success"}},
            "routes": {"again": route("work")}},
    })));
    assert!(
        has(
            &found,
            "decision_domain",
            "start nodes declare exactly the decision 'next'"
        ),
        "{found:?}"
    );
    assert!(
        has(&found, "response_contract", "decision_envelope"),
        "{found:?}"
    );
    assert!(
        has(&found, "decision_domain", "exit nodes declare no routes"),
        "{found:?}"
    );
    assert!(
        has(
            &found,
            "decision_domain",
            "agent_task steps need at least one route"
        ),
        "{found:?}"
    );

    // Tools take exactly one adapter whose values are all declared routes.
    let tool = |config: Value, routes: Value| {
        let mut config = config;
        config["kind"] = json!("tool");
        config["command"] = json!("true");
        rules(flow(json!({
            "start": start("tool"),
            "tool": {"kind": "tool", "config": config, "routes": routes},
            "done": exit("completed", "success"),
        })))
    };
    assert_eq!(
        tool(
            json!({"exit_codes": {"0": "done"}}),
            json!({"done": route("done")})
        ),
        Vec::new()
    );
    assert_eq!(
        tool(
            json!({"decision_field": "decision"}),
            json!({"done": route("done")})
        ),
        Vec::new()
    );
    assert!(has(
        &tool(json!({}), json!({"done": route("done")})),
        "decision_domain",
        "exactly one adapter"
    ));
    assert!(has(
        &tool(
            json!({"exit_codes": {"0": "done"}, "decision_field": "decision"}),
            json!({"done": route("done")})
        ),
        "decision_domain",
        "exactly one adapter"
    ));
    let found = tool(
        json!({"exit_codes": {"0": "done", "x": "missing"}}),
        json!({"done": route("done"), "unused": route("done")}),
    );
    assert!(
        has(&found, "decision_domain", "undeclared decision 'missing'"),
        "{found:?}"
    );
    assert!(
        has(
            &found,
            "decision_domain",
            "exit code key 'x' must be an integer"
        ),
        "{found:?}"
    );
    assert!(
        has(&found, "decision_domain", "'unused' is not selected"),
        "{found:?}"
    );

    // Exits sharing a result name must agree on its outcome.
    let found = rules(flow(json!({
        "start": start("work"),
        "work": agent(json!({"a": route("one"), "b": route("two")})),
        "one": exit("blocked", "failure"),
        "two": exit("blocked", "success"),
    })));
    assert!(
        has(&found, "exit_result", "share result 'blocked'"),
        "{found:?}"
    );
}

#[test]
fn graphs_need_reachable_nodes_exits_and_no_ordinary_leaves() {
    let found = rules(flow(json!({
        "start": start("work"),
        "work": agent(json!({"loop": route("work")})),
        "orphan": agent(json!({"done": route("done")})),
        "done": exit("completed", "success"),
    })));
    assert!(has(&found, "reachability", "'orphan'"), "{found:?}");
    assert!(
        has(&found, "exit_path", "'work' has no path to an exit"),
        "{found:?}"
    );

    let found = rules(flow(json!({"start": start("start")})));
    assert!(has(&found, "exit_node", "at least one exit"), "{found:?}");

    let mut leaf = review_loop();
    leaf["nodes"]["gate"] = json!({"kind": "human_gate", "routes": {}});
    leaf["nodes"]["work"]["routes"]["ask"] = route("gate");
    let found = rules(leaf);
    // A gate without routes is one problem, reported once.
    assert!(
        has(
            &found,
            "decision_domain",
            "gate.md: human_gate steps need at least one route"
        ),
        "{found:?}"
    );
    assert!(!has(&found, "nonterminal_leaf", "'gate'"), "{found:?}");

    let mut leaf = review_loop();
    leaf["nodes"]["switch"] = json!({"kind": "switch", "config": {"kind": "switch", "on": "context.x", "cases": {}, "otherwise": "done"}, "routes": {}});
    leaf["nodes"]["work"]["routes"]["ask"] = route("switch");
    let found = rules(leaf);
    assert!(
        has(
            &found,
            "nonterminal_leaf",
            "'switch' has no routes, but only exit nodes"
        ),
        "{found:?}"
    );

    let mut budget = review_loop();
    budget["nodes"]["work"]["runtime"] = json!({"max_visits": 0});
    assert!(has(&rules(budget), "max_visits", "positive"));
}

fn fork(branches: Value, extra: Value) -> Value {
    let mut nodes = json!({
        "start": start("fork"),
        "fork": {"kind": "parallel", "config": {"kind": "parallel", "branches": branches, "join": "join",
            "tolerate_branch_execution_errors": true}},
        "a": agent(json!({"done": route("join"), "unfinished": route("a"), "blocked": route("failed")})),
        "b": agent(json!({"done": route("join"), "blocked": route("failed")})),
        "join": {"kind": "fan_in", "routes": {"next": route("synth")}},
        "synth": agent(json!({"done": route("done")})),
        "done": exit("completed", "success"),
        "failed": exit("blocked", "failure"),
    });
    for (key, value) in extra.as_object().unwrap() {
        nodes[key] = value.clone();
    }
    flow(nodes)
}

#[test]
fn forks_are_structured_with_disjoint_branches_and_one_join() {
    // Branches may share the failure exit.
    assert_eq!(rules(fork(json!(["a", "b"]), json!({}))), Vec::new());
    let parsed: FlowDefinition =
        serde_json::from_value(fork(json!(["a", "b"]), json!({}))).unwrap();
    let structural = parsed
        .edges()
        .into_iter()
        .filter(|edge| edge.decision.is_none())
        .map(|edge| (edge.from, edge.to))
        .collect::<Vec<_>>();
    assert_eq!(
        structural,
        vec![
            ("fork".to_string(), "a".to_string()),
            ("fork".to_string(), "b".to_string()),
            ("fork".to_string(), "join".to_string()),
        ]
    );

    let found = rules(fork(json!(["a", "a"]), json!({})));
    assert!(
        has(&found, "decision_domain", "branch 'a' is listed twice"),
        "{found:?}"
    );

    // A branch reaching a success exit, or the join's continuation.
    let found = rules(fork(
        json!(["a", "b"]),
        json!({"b": agent(json!({"done": route("join"), "shortcut": route("done")}))}),
    ));
    assert!(
        has(&found, "fork_structure", "reaches success exit 'done'"),
        "{found:?}"
    );
    let found = rules(fork(
        json!(["a", "b"]),
        json!({"b": agent(json!({"done": route("join"), "skip": route("synth")}))}),
    ));
    assert!(has(&found, "fork_structure", "success exit"), "{found:?}");

    // Overlapping interiors and outside entries into a branch.
    let found = rules(fork(
        json!(["a", "b"]),
        json!({"b": agent(json!({"done": route("join"), "help": route("a")}))}),
    ));
    assert!(
        has(&found, "fork_structure", "belongs to both branch"),
        "{found:?}"
    );
    let found = rules(fork(
        json!(["a", "b"]),
        json!({"synth": agent(json!({"done": route("done"), "redo": route("a")}))}),
    ));
    assert!(
        has(&found, "fork_structure", "bypassing the fork"),
        "{found:?}"
    );

    // Nested forks and a join that is not a fan_in.
    let found = rules(fork(
        json!(["a", "b"]),
        json!({
            "b": agent(json!({"split": route("inner")})),
            "inner": {"kind": "parallel", "config": {"kind": "parallel", "branches": ["c"], "join": "inner_join"}},
            "c": agent(json!({"done": route("inner_join")})),
            "inner_join": {"kind": "fan_in", "routes": {"next": route("join")}},
        }),
    ));
    assert!(
        has(&found, "fork_structure", "nested fork 'inner'"),
        "{found:?}"
    );
    let mut bad_join = fork(json!(["a", "b"]), json!({}));
    bad_join["nodes"]["fork"]["config"]["join"] = json!("synth");
    assert!(has(
        &rules(bad_join),
        "fork_structure",
        "must be a fan_in node"
    ));

    // Old aggregation settings are rejected, and forks take no routes.
    let mut old = fork(json!(["a", "b"]), json!({}));
    old["nodes"]["fork"]["config"]["join_policy"] = json!("first_success");
    assert!(serde_json::from_value::<FlowDefinition>(old).is_err());
    let mut routed = fork(json!(["a", "b"]), json!({}));
    routed["nodes"]["fork"]["routes"] = json!({"next": route("join")});
    assert!(has(&rules(routed), "decision_domain", "not routes"));
}

fn switch_flow(cases: Value, otherwise: &str, reads: Value, writer: bool) -> Value {
    let mut prepare = json!({"kind": "tool", "config": {"kind": "tool", "command": "true",
        "exit_codes": {"0": "next"}}, "routes": {"next": route("check")}});
    if writer {
        prepare["config"]["output_map"] = json!({"context.count": "count"});
    }
    flow(json!({
        "start": start("prepare"),
        "prepare": prepare,
        "check": {"kind": "switch", "contracts": {"reads_context": reads},
            "config": {"kind": "switch", "on": "context.count", "cases": cases, "otherwise": otherwise},
            "routes": {"clean": route("done"), "conflicts": route("done")}},
        "done": exit("completed", "success"),
    }))
}

#[test]
fn switches_need_complete_scalar_cases_and_an_upstream_writer() {
    let reads = json!(["context.count"]);
    assert_eq!(
        rules(switch_flow(
            json!({"0": "clean"}),
            "conflicts",
            reads.clone(),
            true
        )),
        Vec::new()
    );
    // `on` is the switch's declared read; reads_context need not repeat it.
    assert_eq!(
        rules(switch_flow(
            json!({"0": "clean"}),
            "conflicts",
            json!([]),
            true
        )),
        Vec::new()
    );
    let found = rules(switch_flow(
        json!({"0": "clean"}),
        "conflicts",
        json!([]),
        false,
    ));
    assert!(
        has(
            &found,
            "context_source",
            "check.md: config.on reads 'context.count'"
        ),
        "{found:?}"
    );
    let found = rules(switch_flow(
        json!({"0": "clean", "0.0": "clean", "null": "clean", "ready": "clean"}),
        "conflicts",
        reads.clone(),
        true,
    ));
    assert!(
        has(&found, "decision_domain", "duplicates another case value"),
        "{found:?}"
    );
    assert!(has(&found, "decision_domain", "case null"), "{found:?}");
    assert!(has(&found, "decision_domain", "case ready"), "{found:?}");
    let found = rules(switch_flow(
        json!({"0": "clean"}),
        "missing",
        reads.clone(),
        true,
    ));
    assert!(
        has(&found, "decision_domain", "undeclared decision 'missing'"),
        "{found:?}"
    );
    assert!(
        has(&found, "decision_domain", "'conflicts' is not selected"),
        "{found:?}"
    );
    let mut no_otherwise = switch_flow(json!({"0": "clean"}), "conflicts", reads, true);
    no_otherwise["nodes"]["check"]["config"]
        .as_object_mut()
        .unwrap()
        .remove("otherwise");
    assert!(serde_json::from_value::<FlowDefinition>(no_otherwise).is_err());
}

/// One agent step `work` between start and exit, with `extra` merged into it.
fn one_step(extra: Value) -> Value {
    let mut value = flow(json!({
        "start": start("work"),
        "work": agent(json!({"done": route("done")})),
        "done": exit("completed", "success"),
    }));
    for (key, field) in extra.as_object().unwrap() {
        value["nodes"]["work"][key] = field.clone();
    }
    value
}

#[test]
fn context_keys_are_literal_and_flow_writes_stay_in_context() {
    let reads = |keys: Value| {
        rules(one_step(
            json!({"contracts": {"response": "decision_envelope", "reads_context": keys}}),
        ))
    };
    let writes = |keys: Value| {
        rules(one_step(
            json!({"contracts": {"response": "decision_envelope", "writes_context": keys}}),
        ))
    };
    assert_eq!(
        reads(json!([
            "internal.run_id",
            "tool.output",
            "stack.child.status",
            "parallel.results",
            "human.gate.note"
        ])),
        Vec::new()
    );
    assert_eq!(writes(json!(["context.review.summary"])), Vec::new());

    // Shorthand and padded keys are never expanded or trimmed.
    let found = reads(json!(["review.summary"]));
    assert!(
        has(
            &found,
            "context_key",
            "work.md: contracts.reads_context key 'review.summary' is not a context.* key"
        ),
        "{found:?}"
    );
    let found = writes(json!(["summary"]));
    assert!(
        has(
            &found,
            "context_key",
            "key 'summary' is not a context.* key"
        ),
        "{found:?}"
    );
    let found = reads(json!([" context.review.summary"]));
    assert!(
        has(
            &found,
            "context_key",
            "key ' context.review.summary' is invalid: keys must not contain whitespace"
        ),
        "{found:?}"
    );
    let found = writes(json!(["context.review.summary "]));
    assert!(
        has(&found, "context_key", "must not contain whitespace"),
        "{found:?}"
    );

    // A flow writes only context.* keys, wherever the write comes from.
    let found = writes(json!(["tool.output"]));
    assert!(
        has(
            &found,
            "context_write",
            "work.md: contracts.writes_context writes 'tool.output', which Spark owns"
        ),
        "{found:?}"
    );
    let mut input = one_step(json!({}));
    input["inputs"] = json!([{"key": "internal.run_id", "label": "Run"}]);
    let found = rules(input);
    assert!(
        has(&found, "context_write", "inputs writes 'internal.run_id'"),
        "{found:?}"
    );
    let tool = rules(flow(json!({
        "start": start("probe"),
        "probe": {"kind": "tool", "config": {"kind": "tool", "command": "true", "exit_codes": {"0": "next"},
            "output_map": {"stack.child.status": "status"}}, "routes": {"next": route("done")}},
        "done": exit("completed", "success"),
    })));
    assert!(
        has(
            &tool,
            "context_write",
            "probe.md: config.output_map writes 'stack.child.status'"
        ),
        "{tool:?}"
    );

    // Spark keys outside the readable list are private.
    for key in [
        "graph.goal",
        "internal.llm_profiles_snapshot",
        "_attractor.runtime.sessions",
        "stack.child_flow_ref",
    ] {
        let found = reads(json!([key]));
        assert!(
            has(
                &found,
                "private_context_read",
                &format!("contracts.reads_context reads '{key}', which is private to Spark")
            ),
            "{key}: {found:?}"
        );
    }
}

#[test]
fn every_context_read_needs_a_writer_that_can_run_before_it() {
    // Each read field is checked and reported with its node and field.
    let unsourced = rules(flow(json!({
        "start": start("gate"),
        "gate": {"kind": "tool", "config": {"kind": "tool", "command": "true", "exit_codes": {"0": "next"},
            "env_map": {"PLAN": "context.plan"}}, "routes": {"next": route("work")}},
        "work": {"kind": "agent_task", "runtime": {"workdir_from": "context.workspace.path"},
            "contracts": {"response": "decision_envelope", "reads_context": ["context.review.summary"]},
            "routes": {"done": route("child")}},
        "child": {"kind": "subflow", "config": {"kind": "subflow", "flow_ref": "child",
            "input_map": {"context.task": "context.task.objective"}, "results": {"completed": "done"}},
            "manager": {"child_workdir_from": "context.child.path", "stop_condition": "stack.child.status=done && context.halt=true"},
            "routes": {"done": route("done")}},
        "done": exit("completed", "success"),
    })));
    for (node, field, key) in [
        ("gate", "config.env_map", "context.plan"),
        ("work", "runtime.workdir_from", "context.workspace.path"),
        ("work", "contracts.reads_context", "context.review.summary"),
        ("child", "config.input_map", "context.task.objective"),
        ("child", "manager.child_workdir_from", "context.child.path"),
        ("child", "manager.stop_condition", "context.halt"),
    ] {
        assert!(
            has(
                &unsourced,
                "context_source",
                &format!("{node}.md: {field} reads '{key}'")
            ),
            "{node} {field} {key}: {unsourced:?}"
        );
    }
    assert_eq!(unsourced.len(), 6, "{unsourced:?}");

    // A flow input or an earlier writer is a source.
    let mut sourced = one_step(
        json!({"contracts": {"response": "decision_envelope", "reads_context": ["context.request.objective"]}}),
    );
    sourced["inputs"] = json!([{"key": "context.request.objective", "label": "Objective"}]);
    assert_eq!(rules(sourced), Vec::new());

    // A node's own write counts only when a loop can bring it back.
    let own = |self_route: bool| {
        let mut routes = json!({"done": route("done")});
        if self_route {
            routes["again"] = route("work");
        }
        rules(one_step(json!({
            "contracts": {"response": "decision_envelope", "reads_context": ["context.notes"], "writes_context": ["context.notes"]},
            "routes": routes,
        })))
    };
    assert!(has(&own(false), "context_source", "reads 'context.notes'"));
    assert_eq!(own(true), Vec::new());

    // Branch writes are visible later in the same branch only: never to a
    // sibling, the join or anything after it.
    let writer = json!({"kind": "agent_task", "contracts": {"response": "decision_envelope",
        "writes_context": ["context.review.lens_findings"]}, "routes": {"done": route("a_check")}});
    let reader = |next: &str| {
        json!({"kind": "agent_task", "contracts": {"response": "decision_envelope",
        "reads_context": ["context.review.lens_findings"]}, "routes": {"done": route(next)}})
    };
    let found = rules(flow(json!({
        "start": start("fork"),
        "fork": {"kind": "parallel", "config": {"kind": "parallel", "branches": ["a", "b"], "join": "join"}},
        "a": writer,
        "a_check": reader("join"),
        "b": reader("join"),
        "join": {"kind": "fan_in", "routes": {"next": route("after")}},
        "after": reader("done"),
        "done": exit("completed", "success"),
    })));
    assert!(!has(&found, "context_source", "a_check.md"), "{found:?}");
    assert!(
        has(
            &found,
            "context_source",
            "b.md: contracts.reads_context reads 'context.review.lens_findings'"
        ),
        "{found:?}"
    );
    assert!(
        has(
            &found,
            "context_source",
            "after.md: contracts.reads_context reads 'context.review.lens_findings'"
        ),
        "{found:?}"
    );
    assert_eq!(found.len(), 2, "{found:?}");
}

#[test]
fn subflows_map_every_local_decision_from_child_results() {
    let subflow = |results: Value| {
        rules(flow(json!({
            "start": start("child"),
            "child": {"kind": "subflow", "config": {"kind": "subflow", "flow_ref": "child", "results": results},
                "routes": {"child_completed": route("done"), "child_blocked": route("failed")}},
            "done": exit("completed", "success"),
            "failed": exit("blocked", "failure"),
        })))
    };
    assert_eq!(
        subflow(json!({"completed": "child_completed", "blocked": "child_blocked"})),
        Vec::new()
    );
    let found = subflow(json!({"completed": "child_completed"}));
    assert!(
        has(&found, "decision_domain", "'child_blocked' is not mapped"),
        "{found:?}"
    );
    let found = subflow(json!({}));
    assert!(
        has(&found, "decision_domain", "map every child result"),
        "{found:?}"
    );
}

#[test]
fn switch_duplicate_cases_compare_exact_numbers() {
    let reads = json!(["context.count"]);
    // Distinct large integers are distinct cases, even where f64 rounds them together.
    assert_eq!(
        rules(switch_flow(
            json!({"9007199254740992": "clean", "9007199254740993": "conflicts"}),
            "conflicts",
            reads.clone(),
            true,
        )),
        Vec::new()
    );
    for duplicate in [
        json!({"1": "clean", "1.0": "clean"}),
        json!({"9007199254740992": "clean", "9.007199254740992e15": "clean"}),
    ] {
        let found = rules(switch_flow(
            duplicate.clone(),
            "conflicts",
            reads.clone(),
            true,
        ));
        assert!(
            has(&found, "decision_domain", "duplicates another case value"),
            "{duplicate}: {found:?}"
        );
    }
}

#[test]
fn a_success_exit_is_never_a_branch_entry() {
    let found = rules(flow(json!({
        "start": start("fork"),
        "fork": {"kind": "parallel", "config": {"kind": "parallel", "branches": ["done"], "join": "join"}},
        "join": {"kind": "fan_in", "routes": {"next": route("done")}},
        "done": exit("completed", "success"),
    })));
    assert!(
        has(
            &found,
            "fork_structure",
            "branch entry 'done' is a success exit"
        ),
        "{found:?}"
    );
}

/// Raw JSON text: `json!` and `Value` would drop the duplicate before parsing.
const DUPLICATE_ROUTES_JSON: &str = r#"{"schema_version":"2","id":"dup","nodes":{
  "start":{"kind":"start","routes":{"next":{"to":"work","description":"Enter."}}},
  "work":{"kind":"agent_task","contracts":{"response":"decision_envelope"},
    "routes":{"done":{"to":"failed","description":"First."},"done":{"to":"done","description":"Second."}}},
  "done":{"kind":"exit","config":{"kind":"exit","result":{"name":"completed","outcome":"success"}}},
  "failed":{"kind":"exit","config":{"kind":"exit","result":{"name":"blocked","outcome":"failure"}}}}}"#;

#[test]
fn canonical_json_rejects_duplicate_keys_before_insertion() {
    let error = FlowDefinition::from_json_str(DUPLICATE_ROUTES_JSON).unwrap_err();
    assert!(
        error.detail.contains("duplicate key \"done\""),
        "{}",
        error.detail
    );
    assert!(serde_json::from_str::<FlowDefinition>(DUPLICATE_ROUTES_JSON).is_err());

    for raw in [
        // Switch cases, tool exit codes, subflow results and node IDs.
        r#"{"kind":"switch","on":"context.x","cases":{"0":"a","0":"b"},"otherwise":"a"}"#,
        r#"{"kind":"tool","command":"true","exit_codes":{"0":"a","0":"b"}}"#,
        r#"{"kind":"subflow","flow_ref":"child","results":{"completed":"a","completed":"b"}}"#,
    ] {
        let error = serde_json::from_str::<attractor_core::NodeConfig>(raw).unwrap_err();
        assert!(
            error.to_string().contains("duplicate key"),
            "{raw}: {error}"
        );
    }
    let nodes =
        r#"{"schema_version":"2","id":"dup","nodes":{"a":{"kind":"start"},"a":{"kind":"start"}}}"#;
    assert!(serde_json::from_str::<FlowDefinition>(nodes)
        .unwrap_err()
        .to_string()
        .contains("duplicate key \"a\""));
}
