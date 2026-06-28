use std::path::PathBuf;

use attractor_core::{ContextMap, DotGraph, Outcome, RawRuntimeEvent};
use serde_json::json;
use spark_agent_adapter::{
    CodergenBackend, CodergenError, CodergenExecution, CodergenHandler, CodergenRequest,
};

use crate::events::codergen_adapter_event;

pub struct RuntimeCodergen {
    graph: DotGraph,
    logs_root: Option<PathBuf>,
    handler: CodergenHandler,
    fallback_model: Option<String>,
    fallback_provider: Option<String>,
    fallback_profile: Option<String>,
    fallback_reasoning_effort: Option<String>,
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

    pub fn execute(
        &mut self,
        node_id: &str,
        context: ContextMap,
    ) -> Result<CodergenExecution, CodergenError> {
        let node =
            self.graph.nodes.get(node_id).cloned().ok_or_else(|| {
                CodergenError::Backend(format!("unknown codergen node: {node_id}"))
            })?;
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
        })
    }
}

pub fn codergen_events_for_journal(
    run_id: &str,
    node_id: &str,
    execution: &CodergenExecution,
) -> Vec<RawRuntimeEvent> {
    execution
        .events
        .iter()
        .map(|event| {
            codergen_adapter_event(
                run_id,
                node_id,
                &event.event_type,
                serde_json::to_value(&event.payload).unwrap_or_else(|_| json!({})),
            )
        })
        .collect()
}

pub fn codergen_outcome(execution: CodergenExecution) -> Outcome {
    execution.outcome
}
