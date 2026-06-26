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
}

impl RuntimeCodergen {
    pub fn simulation(graph: DotGraph, logs_root: Option<PathBuf>) -> Self {
        Self {
            graph,
            logs_root,
            handler: CodergenHandler::simulation(),
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
        }
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
            fallback_model: None,
            fallback_provider: None,
            fallback_profile: None,
            fallback_reasoning_effort: None,
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
