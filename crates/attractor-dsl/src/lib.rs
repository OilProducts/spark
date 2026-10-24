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
    assemble_flow_files, load_flow, read_flow_dir, read_flow_dir_texts, render_flow_files,
    resolve_flow_roles, write_flow_dir, LoadedFlow, FLOW_FILE,
};
pub use flow_sources::{
    canonicalize_flow_yaml, flow_ref_name, flow_yaml, normalize_flow_name, parse_flow_definition,
    validate_flow_definition, FlowManifest, FlowRoots, FlowSourceError, NamedFlowSource, OrgState,
    ResolvedFlowDir, BUNDLED_LAYER, ORG_LAYER,
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
