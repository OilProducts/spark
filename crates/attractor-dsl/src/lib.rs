#![forbid(unsafe_code)]

//! Flow source helpers for Spark-authored YAML flow definitions.

mod flow_sources;
mod transforms;
mod validation;

pub use attractor_core::{
    DotAttribute, DotEdge, DotGraph, DotNode, DotScopeDefaults, DotSubgraphScope, DotValue,
    DotValueType, DurationLiteral, FlowDefinition, FlowDefinitionError, FlowDiagnostic,
};
pub use flow_sources::{
    canonicalize_flow_yaml, ensure_flows_dir, flow_name_from_path, inject_flow_goal,
    load_flow_content, normalize_flow_name, parse_flow_definition, read_named_flow_source,
    resolve_flow_path, FlowSourceError, NamedFlowSource,
};
pub use transforms::{
    apply_graph_transforms, apply_graph_transforms_with_extra, build_transform_pipeline,
    build_transform_pipeline_with_extra, graph_attr_context_seed, AttributeDefaultsTransform,
    GoalVariableTransform, GraphTransform, ModelStylesheetTransform, RuntimePreambleTransform,
    TransformPipeline,
};
pub use validation::{
    diagnostic_payload, diagnostics_payload, validate, validate_graph,
    validate_launch_contract_declarations, validate_or_raise, validate_or_raise_with_extra,
    validate_with_extra, Diagnostic, DiagnosticSeverity, LintRule, ValidationError,
};
