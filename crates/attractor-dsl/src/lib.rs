#![forbid(unsafe_code)]

//! Flow source helpers for Spark flow directories and flow definitions.

mod flow_files;
mod flow_sources;
mod transforms;
mod validation;

pub use attractor_core::{
    DotAttribute, DotEdge, DotGraph, DotNode, DotScopeDefaults, DotSubgraphScope, DotValue,
    DotValueType, DurationLiteral, FlowDefinition, FlowDefinitionError, FlowDiagnostic,
};
pub use flow_files::{
    assemble_flow_files, read_flow_dir, render_flow_files, resolve_flow_roles, write_flow_dir,
    FLOW_FILE,
};
pub use flow_sources::{
    canonicalize_flow_yaml, ensure_flows_dir, flow_name_from_path, flow_yaml, inject_flow_goal,
    list_flow_names, load_flow_content, load_resolved_flow_content, normalize_flow_name,
    parse_flow_definition, read_named_flow_source, resolve_flow_path, resolve_flow_ref,
    validate_flow_definition, FlowSourceError, NamedFlowSource,
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
