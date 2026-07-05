#![forbid(unsafe_code)]

//! Parser for the Spark-supported Attractor DOT subset.

mod error;
mod flow_sources;
mod formatter;
mod lexer;
mod parser;
mod preview;
mod transforms;
mod validation;

pub use attractor_core::{
    DotAttribute, DotEdge, DotGraph, DotNode, DotScopeDefaults, DotSubgraphScope, DotValue,
    DotValueType, DurationLiteral,
};
pub use error::DotParseError;
pub use flow_sources::{
    ensure_flows_dir, flow_name_from_path, inject_pipeline_goal, load_flow_content,
    normalize_flow_name, resolve_flow_path, semantic_equivalent_sources,
    semantic_signature_for_graph, semantic_signature_for_source, FlowSourceError,
};
pub use formatter::{
    canonicalize_dot, canonicalize_readable_dot, format_dot, format_readable_dot,
    semantic_equivalent, semantic_signature,
};
pub use parser::{normalize_graph, parse_dot};
pub use preview::{
    graph_payload, graph_payload_with_child_previews, parse_error_payload, preview_dot_source,
    preview_dot_source_with_extra, preview_response_payload, preview_response_payload_with_extra,
    preview_response_payload_with_options, DotPreview, PreviewOptions,
};
pub use transforms::{
    apply_graph_transforms, apply_graph_transforms_with_extra, build_transform_pipeline,
    build_transform_pipeline_with_extra, graph_attr_context_seed, AttributeDefaultsTransform,
    GoalVariableTransform, GraphTransform, ModelStylesheetTransform, RuntimePreambleTransform,
    TransformPipeline,
};
pub use validation::{
    diagnostic_payload, diagnostics_payload, preview_payload_for_graph, validate, validate_graph,
    validate_launch_contract_declarations, validate_or_raise, validate_or_raise_with_extra,
    validate_with_extra, Diagnostic, DiagnosticSeverity, LintRule, ValidationError,
};
