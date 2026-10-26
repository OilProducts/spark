use std::sync::{Arc, Mutex};

use attractor_core::{DotAttribute, DotValue, DotValueType};
use serde_json::{json, Value};
use spark_agent_adapter::{
    CodergenBackend, CodergenBackendOutput, CodergenBackendRequest, CodergenError, CodergenHandler,
    CodergenRequest,
};

struct CapturePrompt(Arc<Mutex<String>>);

impl CodergenBackend for CapturePrompt {
    fn run(
        &mut self,
        request: CodergenBackendRequest,
    ) -> Result<CodergenBackendOutput, CodergenError> {
        *self.0.lock().unwrap() = request.prompt;
        Ok(CodergenBackendOutput::text(r#"{"outcome":"success"}"#))
    }
}

fn rendered_prompt(response_contract: Option<&str>, edges: Value) -> String {
    let flow = flow(edges);
    let mut graph = attractor_dsl::apply_graph_transforms(&flow.to_runtime_dot_graph());
    if let Some(contract) = response_contract {
        // The runtime maps a node's `contracts.response` onto this attribute.
        graph.nodes.get_mut("work").unwrap().attrs.insert(
            "codergen.response_contract".to_string(),
            DotAttribute {
                key: "codergen.response_contract".to_string(),
                value: DotValue::String(contract.to_string()),
                value_type: DotValueType::String,
                line: 0,
            },
        );
    }
    let node_id = "work";
    let request: CodergenRequest = serde_json::from_value(
        json!({"node_id": node_id, "node": graph.nodes[node_id], "graph": graph, "context": {}}),
    )
    .unwrap();
    let prompt = Arc::new(Mutex::new(String::new()));
    CodergenHandler::with_backend(CapturePrompt(prompt.clone()))
        .execute(request)
        .unwrap();
    let prompt = prompt.lock().unwrap().clone();
    prompt
}

fn flow(edges: Value) -> attractor_core::FlowDefinition {
    let source = json!({"schema_version":"1", "id":"flow",
        "nodes":{
            "start":{"kind":"start"},
            "work":{"kind":"agent_task", "config":{"kind":"agent_task", "prompt":"Do the work."}},
            "fix":{"kind":"agent_task", "label":"Fix It", "config":{"kind":"agent_task", "prompt":"fix"}},
            "retry":{"kind":"agent_task", "config":{"kind":"agent_task", "prompt":"retry"}},
            "review":{"kind":"agent_task", "label":"Human Review", "config":{"kind":"agent_task", "prompt":"review"}},
            "end":{"kind":"exit", "label":"Done"}
        },
        "edges": edges
    });
    attractor_dsl::validated_flow(serde_json::from_value(source).unwrap()).unwrap()
}

fn routed_edges() -> Value {
    json!([
        {"from":"start","to":"work"},
        {"from":"work","to":"end"},
        {"from":"work","to":"review","label":"Escalate"},
        {"from":"work","to":"retry","condition":"outcome=retry","weight":1},
        {"from":"work","to":"fix","condition":"outcome=fail","weight":5},
        {"from":"work","to":"review","condition":"outcome=fail && context.flag=1","weight":9},
        {"from":"fix","to":"work"},
        {"from":"retry","to":"work"},
        {"from":"review","to":"end"}
    ])
}

#[test]
fn status_envelope_appendix_lists_routes_in_routing_precedence_order() {
    let prompt = rendered_prompt(Some("status_envelope"), routed_edges());
    let routes = [
        "Routes from this node, checked in this order:",
        "- When the condition `outcome=fail` holds: go to \"Fix It\".",
        "- When the condition `outcome=fail && context.flag=1` holds: go to \"Human Review\".",
        "- When the condition `outcome=retry` holds: go to \"retry\".",
        "- With \"preferred_label\": \"Escalate\": go to \"Human Review\".",
        "- Default: go to \"Done\".",
    ]
    .join("\n");
    assert!(prompt.contains(&routes), "{prompt}");
}

#[test]
fn status_envelope_appendix_has_no_routes_section_without_outgoing_edges() {
    let edges = json!([
        {"from":"start","to":"work"},
        {"from":"start","to":"fix"},
        {"from":"start","to":"retry"},
        {"from":"start","to":"review"},
        {"from":"fix","to":"end"},
        {"from":"retry","to":"end"},
        {"from":"review","to":"end"}
    ]);
    let prompt = rendered_prompt(Some("status_envelope"), edges);
    assert!(prompt.contains("Structured response contract:"), "{prompt}");
    assert!(!prompt.contains("Routes from this node"), "{prompt}");
}

#[test]
fn non_envelope_node_prompt_is_unchanged_by_routes() {
    let prompt = rendered_prompt(None, routed_edges());
    assert_eq!(prompt, "Do the work.");
}
