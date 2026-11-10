use std::collections::BTreeMap;

use attractor_core::{ContextMap, DotAttribute, DotGraph, DotValue, DotValueType};
use serde_json::{Number, Value};

const DEFAULT_MAX_RETRIES_KEY: &str = "default_max_retries";
const REASONING_DEFAULT_PROPERTIES: [&str; 4] = [
    "thinking",
    "thinking_budget_tokens",
    "reasoning_mode",
    "reasoning_summary",
];

pub trait GraphTransform: std::fmt::Debug + Send + Sync {
    fn apply(&self, graph: &mut DotGraph);
}

#[derive(Debug, Default)]
pub struct TransformPipeline {
    transforms: Vec<Box<dyn GraphTransform>>,
}

impl TransformPipeline {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register<T>(&mut self, transform: T)
    where
        T: GraphTransform + 'static,
    {
        self.transforms.push(Box::new(transform));
    }

    pub fn register_boxed(&mut self, transform: Box<dyn GraphTransform>) {
        self.transforms.push(transform);
    }

    pub fn apply(&self, graph: &DotGraph) -> DotGraph {
        let mut transformed = graph.clone();
        for transform in &self.transforms {
            transform.apply(&mut transformed);
        }
        transformed
    }
}

pub fn build_transform_pipeline() -> TransformPipeline {
    let mut pipeline = TransformPipeline::new();
    pipeline.register(GoalVariableTransform);
    pipeline.register(ReasoningDefaultsTransform);
    pipeline
}

pub fn build_transform_pipeline_with_extra(
    extra_transforms: impl IntoIterator<Item = Box<dyn GraphTransform>>,
) -> TransformPipeline {
    let mut pipeline = build_transform_pipeline();
    for transform in extra_transforms {
        pipeline.register_boxed(transform);
    }
    pipeline
}

pub fn apply_graph_transforms(graph: &DotGraph) -> DotGraph {
    build_transform_pipeline().apply(graph)
}

pub fn apply_graph_transforms_with_extra(
    graph: &DotGraph,
    extra_transforms: impl IntoIterator<Item = Box<dyn GraphTransform>>,
) -> DotGraph {
    build_transform_pipeline_with_extra(extra_transforms).apply(graph)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttributeDefaultsTransform {
    child_workdir: String,
}

impl AttributeDefaultsTransform {
    pub fn new() -> Self {
        let child_workdir = std::env::current_dir()
            .map(|path| path.to_string_lossy().to_string())
            .unwrap_or_default();
        Self { child_workdir }
    }

    pub fn with_child_workdir(child_workdir: impl Into<String>) -> Self {
        Self {
            child_workdir: child_workdir.into(),
        }
    }
}

impl Default for AttributeDefaultsTransform {
    fn default() -> Self {
        Self::new()
    }
}

impl GraphTransform for AttributeDefaultsTransform {
    fn apply(&self, graph: &mut DotGraph) {
        for (key, value_type, value) in [
            (
                "goal",
                DotValueType::String,
                DotValue::String(String::new()),
            ),
            (
                "label",
                DotValueType::String,
                DotValue::String(String::new()),
            ),
            (
                DEFAULT_MAX_RETRIES_KEY,
                DotValueType::Integer,
                DotValue::Integer(0),
            ),
            (
                "stack.child_dotfile",
                DotValueType::String,
                DotValue::String(String::new()),
            ),
            (
                "stack.child_workdir",
                DotValueType::String,
                DotValue::String(self.child_workdir.clone()),
            ),
            (
                "tool.hooks.pre",
                DotValueType::String,
                DotValue::String(String::new()),
            ),
            (
                "tool.hooks.post",
                DotValueType::String,
                DotValue::String(String::new()),
            ),
        ] {
            set_default(&mut graph.graph_attrs, key, value_type, value);
        }

        for node in graph.nodes.values_mut() {
            set_default(
                &mut node.attrs,
                "label",
                DotValueType::String,
                DotValue::String(node.node_id.clone()),
            );
            for (key, value_type, value) in [
                (
                    "shape",
                    DotValueType::String,
                    DotValue::String("box".to_string()),
                ),
                (
                    "type",
                    DotValueType::String,
                    DotValue::String(String::new()),
                ),
                (
                    "prompt",
                    DotValueType::String,
                    DotValue::String(String::new()),
                ),
                ("max_retries", DotValueType::Integer, DotValue::Integer(0)),
                (
                    "class",
                    DotValueType::String,
                    DotValue::String(String::new()),
                ),
                ("timeout", DotValueType::Duration, DotValue::Null),
                (
                    "llm_model",
                    DotValueType::String,
                    DotValue::String(String::new()),
                ),
                (
                    "llm_provider",
                    DotValueType::String,
                    DotValue::String(String::new()),
                ),
                (
                    "reasoning_effort",
                    DotValueType::String,
                    DotValue::String("high".to_string()),
                ),
            ] {
                set_default(&mut node.attrs, key, value_type, value);
            }
        }

        for edge in &mut graph.edges {
            for (key, value_type, value) in [(
                "label",
                DotValueType::String,
                DotValue::String(String::new()),
            )] {
                set_default(&mut edge.attrs, key, value_type, value);
            }
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct GoalVariableTransform;

impl GraphTransform for GoalVariableTransform {
    fn apply(&self, graph: &mut DotGraph) {
        let goal = graph.goal();
        for node in graph.nodes.values_mut() {
            let Some(prompt_attr) = node.attrs.get("prompt") else {
                continue;
            };
            let DotValue::String(prompt) = &prompt_attr.value else {
                continue;
            };
            let expanded = prompt.replace("$goal", &goal);
            if expanded == *prompt {
                continue;
            }
            let line = prompt_attr.line;
            node.attrs.insert(
                "prompt".to_string(),
                DotAttribute {
                    key: "prompt".to_string(),
                    value: DotValue::String(expanded),
                    value_type: DotValueType::String,
                    line,
                },
            );
        }
    }
}

/// Copies the flow's default reasoning controls onto nodes that do not set them.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ReasoningDefaultsTransform;

impl GraphTransform for ReasoningDefaultsTransform {
    fn apply(&self, graph: &mut DotGraph) {
        for node in graph.nodes.values_mut() {
            for property in REASONING_DEFAULT_PROPERTIES {
                if node.explicit_attr_keys.contains(property) {
                    continue;
                }
                // A node's own non-budget thinking choice drops an inherited budget.
                if property == "thinking_budget_tokens"
                    && node.explicit_attr_keys.contains("thinking")
                    && node
                        .attrs
                        .get("thinking")
                        .is_some_and(|v| dot_value_text(&v.value) != "budget")
                {
                    node.attrs.remove(property);
                    continue;
                }
                let Some(attr) = graph.graph_attrs.get(property) else {
                    continue;
                };
                let value = dot_value_text(&attr.value).trim().to_string();
                if value.is_empty() {
                    continue;
                }
                node.attrs.insert(
                    property.to_string(),
                    DotAttribute {
                        key: property.to_string(),
                        value: DotValue::String(value),
                        value_type: DotValueType::String,
                        line: attr.line,
                    },
                );
            }
        }
    }
}

pub fn graph_attr_context_seed(graph: &DotGraph) -> ContextMap {
    let mut seeded = ContextMap::new();
    for (key, attr) in &graph.graph_attrs {
        seeded.insert(format!("graph.{key}"), dot_value_to_json(&attr.value));
    }
    seeded
        .entry("graph.goal".to_string())
        .or_insert_with(|| Value::String(String::new()));
    seeded.insert(
        format!("graph.{DEFAULT_MAX_RETRIES_KEY}"),
        Value::Number(Number::from(resolve_default_max_retries_value(
            &graph.graph_attrs,
            0,
        ))),
    );
    seeded
}

fn set_default(
    attrs: &mut BTreeMap<String, DotAttribute>,
    key: &str,
    value_type: DotValueType,
    value: DotValue,
) {
    attrs
        .entry(key.to_string())
        .or_insert_with(|| DotAttribute {
            key: key.to_string(),
            value,
            value_type,
            line: 0,
        });
}

fn resolve_default_max_retries_value(attrs: &BTreeMap<String, DotAttribute>, default: i64) -> i64 {
    let Some(attr) = attrs.get(DEFAULT_MAX_RETRIES_KEY) else {
        return default;
    };
    match &attr.value {
        DotValue::Integer(value) => *value,
        DotValue::Boolean(value) => i64::from(*value),
        DotValue::String(value) => value.trim().parse().unwrap_or(default),
        _ => default,
    }
}

fn dot_value_to_json(value: &DotValue) -> Value {
    match value {
        DotValue::Null => Value::Null,
        DotValue::String(value) => Value::String(value.clone()),
        DotValue::Integer(value) => Value::Number(Number::from(*value)),
        DotValue::Float(value) => Number::from_f64(*value)
            .map(Value::Number)
            .unwrap_or(Value::Null),
        DotValue::Boolean(value) => Value::Bool(*value),
        DotValue::Duration(value) => Value::String(value.raw.clone()),
    }
}

fn dot_value_text(value: &DotValue) -> String {
    match value {
        DotValue::Null => String::new(),
        DotValue::String(value) => value.clone(),
        DotValue::Integer(value) => value.to_string(),
        DotValue::Float(value) => value.to_string(),
        DotValue::Boolean(true) => "true".to_string(),
        DotValue::Boolean(false) => "false".to_string(),
        DotValue::Duration(value) => value.raw.clone(),
    }
}
