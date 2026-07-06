use std::collections::BTreeMap;
use std::path::PathBuf;

use attractor_core::{ContextMap, DotGraph, Outcome, RawRuntimeEvent};
use serde_json::{json, Value};
use spark_agent_adapter::{
    codergen::CodergenEventSink, CodergenBackend, CodergenError, CodergenExecution,
    CodergenHandler, CodergenRequest,
};

use crate::events::{
    llm_request_completed_event, llm_request_started_event, llm_token_usage_event,
};

pub struct RuntimeCodergen {
    graph: DotGraph,
    logs_root: Option<PathBuf>,
    handler: CodergenHandler,
    fallback_model: Option<String>,
    fallback_provider: Option<String>,
    fallback_profile: Option<String>,
    fallback_reasoning_effort: Option<String>,
    project_path: Option<PathBuf>,
    metadata: BTreeMap<String, Value>,
    event_sink: Option<CodergenEventSink>,
}

impl RuntimeCodergen {
    pub fn simulation(graph: DotGraph, logs_root: Option<PathBuf>) -> Self {
        Self {
            graph,
            logs_root,
            handler: CodergenHandler::simulation(),
            fallback_model: None,
            fallback_provider: None,
            fallback_profile: None,
            fallback_reasoning_effort: None,
            project_path: None,
            metadata: BTreeMap::new(),
            event_sink: None,
        }
    }

    pub fn with_backend(
        graph: DotGraph,
        logs_root: Option<PathBuf>,
        backend: impl CodergenBackend + 'static,
    ) -> Self {
        Self {
            graph,
            logs_root,
            handler: CodergenHandler::with_backend(backend),
            fallback_model: None,
            fallback_provider: None,
            fallback_profile: None,
            fallback_reasoning_effort: None,
            project_path: None,
            metadata: BTreeMap::new(),
            event_sink: None,
        }
    }

    pub fn with_boxed_backend(
        graph: DotGraph,
        logs_root: Option<PathBuf>,
        backend: Box<dyn CodergenBackend>,
    ) -> Self {
        Self {
            graph,
            logs_root,
            handler: CodergenHandler::with_boxed_backend(backend),
            fallback_model: None,
            fallback_provider: None,
            fallback_profile: None,
            fallback_reasoning_effort: None,
            project_path: None,
            metadata: BTreeMap::new(),
            event_sink: None,
        }
    }

    pub fn with_llm_fallbacks(
        mut self,
        model: Option<String>,
        provider: Option<String>,
        profile: Option<String>,
        reasoning_effort: Option<String>,
    ) -> Self {
        self.fallback_model = model;
        self.fallback_provider = provider;
        self.fallback_profile = profile;
        self.fallback_reasoning_effort = reasoning_effort;
        self
    }

    pub fn with_runtime_context(
        mut self,
        project_path: Option<PathBuf>,
        metadata: BTreeMap<String, Value>,
    ) -> Self {
        self.project_path = project_path;
        self.metadata = metadata;
        self
    }

    pub fn with_event_sink(mut self, event_sink: Option<CodergenEventSink>) -> Self {
        self.event_sink = event_sink;
        self
    }

    pub fn execute(
        &mut self,
        node_id: &str,
        context: ContextMap,
    ) -> Result<CodergenExecution, CodergenError> {
        let node =
            self.graph.nodes.get(node_id).cloned().ok_or_else(|| {
                CodergenError::Backend(format!("unknown codergen node: {node_id}"))
            })?;
        spark_agent_adapter::codergen::with_codergen_event_sink(self.event_sink.clone(), || {
            self.handler.execute(CodergenRequest {
                node_id: node_id.to_string(),
                node,
                graph: self.graph.clone(),
                context,
                logs_root: self.logs_root.clone(),
                fallback_model: self.fallback_model.clone(),
                fallback_provider: self.fallback_provider.clone(),
                fallback_profile: self.fallback_profile.clone(),
                fallback_reasoning_effort: self.fallback_reasoning_effort.clone(),
                project_path: self.project_path.clone(),
                metadata: self.metadata.clone(),
            })
        })
    }
}

pub fn codergen_events_for_journal(
    run_id: &str,
    node_id: &str,
    execution: &CodergenExecution,
) -> Vec<RawRuntimeEvent> {
    let mut events = Vec::new();
    for event in &execution.events {
        match event.event_type.as_str() {
            "codergen_backend_request_started" => {
                events.push(llm_request_started_event(
                    run_id,
                    node_id,
                    low_volume_llm_payload(&event.payload),
                ));
            }
            "rust_llm_adapter_request_completed"
            | "rust_agent_adapter_request_completed"
            | "codex_app_server_request_completed" => {
                events.push(llm_request_completed_event(
                    run_id,
                    node_id,
                    low_volume_llm_payload(&event.payload),
                ));
            }
            "rust_agent_session_event" | "codex_app_server_session_event" => {
                if let Some(usage) = event
                    .payload
                    .get("token_usage")
                    .cloned()
                    .or_else(|| {
                        event
                            .payload
                            .get("turn_stream_event")?
                            .get("token_usage")
                            .cloned()
                    })
                    .or_else(|| {
                        event
                            .payload
                            .get("session_event")?
                            .get("data")?
                            .get("usage")
                            .cloned()
                    })
                {
                    events.push(llm_token_usage_event(run_id, node_id, usage));
                }
            }
            _ => {}
        }
    }
    events
}

fn low_volume_llm_payload(payload: &BTreeMap<String, Value>) -> Value {
    let mut output = serde_json::Map::new();
    for key in [
        "node_id",
        "provider",
        "provider_selector",
        "model",
        "model_selector",
        "llm_profile",
        "reasoning_effort",
        "response_contract",
        "runtime_mode",
        "repair_attempts",
        "token_usage",
    ] {
        if let Some(value) = payload.get(key) {
            output.insert(key.to_string(), value.clone());
        }
    }
    json!(output)
}

pub fn codergen_outcome(execution: CodergenExecution) -> Outcome {
    execution.outcome
}
