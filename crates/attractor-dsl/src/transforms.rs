use std::collections::BTreeMap;

use attractor_core::{
    AttractorContext, ContextMap, DotAttribute, DotGraph, DotNode, DotValue, DotValueType,
};
use serde_json::{Number, Value};

const DEFAULT_MAX_RETRIES_KEY: &str = "default_max_retries";
const ALLOWED_STYLE_PROPERTIES: [&str; 4] = [
    "llm_model",
    "llm_provider",
    "llm_profile",
    "reasoning_effort",
];
const ALLOWED_REASONING_EFFORTS: [&str; 4] = ["low", "medium", "high", "xhigh"];
const NODE_OUTCOMES_KEY: &str = "_attractor.node_outcomes";
const RUNTIME_RETRY_NODE_ID_KEY: &str = "_attractor.runtime.retry.node_id";
const RUNTIME_RETRY_ATTEMPT_KEY: &str = "_attractor.runtime.retry.attempt";
const RUNTIME_RETRY_MAX_ATTEMPTS_KEY: &str = "_attractor.runtime.retry.max_attempts";
const RUNTIME_RETRY_FAILURE_REASON_KEY: &str = "_attractor.runtime.retry.failure_reason";

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
    pipeline.register(ModelStylesheetTransform);
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
                "model_stylesheet",
                DotValueType::String,
                DotValue::String(String::new()),
            ),
            (
                DEFAULT_MAX_RETRIES_KEY,
                DotValueType::Integer,
                DotValue::Integer(0),
            ),
            (
                "retry_target",
                DotValueType::String,
                DotValue::String(String::new()),
            ),
            (
                "fallback_retry_target",
                DotValueType::String,
                DotValue::String(String::new()),
            ),
            (
                "default_fidelity",
                DotValueType::String,
                DotValue::String(String::new()),
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
                ("goal_gate", DotValueType::Boolean, DotValue::Boolean(false)),
                (
                    "retry_target",
                    DotValueType::String,
                    DotValue::String(String::new()),
                ),
                (
                    "fallback_retry_target",
                    DotValueType::String,
                    DotValue::String(String::new()),
                ),
                (
                    "fidelity",
                    DotValueType::String,
                    DotValue::String(String::new()),
                ),
                (
                    "thread_id",
                    DotValueType::String,
                    DotValue::String(String::new()),
                ),
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
                (
                    "auto_status",
                    DotValueType::Boolean,
                    DotValue::Boolean(false),
                ),
                (
                    "allow_partial",
                    DotValueType::Boolean,
                    DotValue::Boolean(false),
                ),
            ] {
                set_default(&mut node.attrs, key, value_type, value);
            }
        }

        for edge in &mut graph.edges {
            for (key, value_type, value) in [
                (
                    "label",
                    DotValueType::String,
                    DotValue::String(String::new()),
                ),
                (
                    "condition",
                    DotValueType::String,
                    DotValue::String(String::new()),
                ),
                ("weight", DotValueType::Integer, DotValue::Integer(0)),
                (
                    "loop_restart",
                    DotValueType::Boolean,
                    DotValue::Boolean(false),
                ),
            ] {
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

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ModelStylesheetTransform;

impl GraphTransform for ModelStylesheetTransform {
    fn apply(&self, graph: &mut DotGraph) {
        let (rules, stylesheet_line) = match graph.graph_attrs.get("model_stylesheet") {
            Some(attr) => match &attr.value {
                DotValue::String(value) if !value.trim().is_empty() => {
                    (parse_style_rules(value), attr.line)
                }
                _ => (Vec::new(), 0),
            },
            None => (Vec::new(), 0),
        };
        let graph_defaults = graph_default_model_attrs(graph);

        for node in graph.nodes.values_mut() {
            apply_style_rules_to_node(node, &rules, stylesheet_line, &graph_defaults);
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimePreambleTransform {
    pub node_outcomes_key: String,
}

impl RuntimePreambleTransform {
    pub fn new() -> Self {
        Self {
            node_outcomes_key: NODE_OUTCOMES_KEY.to_string(),
        }
    }

    pub fn apply(
        &self,
        fidelity: &str,
        context: &AttractorContext,
        completed_nodes: &[impl AsRef<str>],
    ) -> String {
        self.apply_with_options(fidelity, context, completed_nodes, true)
    }

    pub fn apply_with_options(
        &self,
        fidelity: &str,
        context: &AttractorContext,
        completed_nodes: &[impl AsRef<str>],
        include_context_items: bool,
    ) -> String {
        let mode = fidelity.trim().to_ascii_lowercase();
        if mode == "full" {
            return String::new();
        }

        let snapshot = context.snapshot();
        let goal = snapshot
            .get("graph.goal")
            .map(json_value_to_text)
            .unwrap_or_default()
            .trim()
            .to_string();
        let run_id = snapshot
            .get("internal.run_id")
            .map(json_value_to_text)
            .unwrap_or_default()
            .trim()
            .to_string();
        let statuses = snapshot
            .get(&self.node_outcomes_key)
            .and_then(Value::as_object);
        let retry_lines = retry_context_lines(&snapshot);

        if mode == "truncate" {
            let mut lines = vec![
                "carryover:truncate".to_string(),
                format!("goal={goal}"),
                format!("run_id={run_id}"),
            ];
            lines.extend(retry_lines);
            return lines.join("\n");
        }

        let context_items = if include_context_items {
            carryover_context_items(&snapshot)
        } else {
            Vec::new()
        };

        if mode == "compact" {
            let mut lines = vec![
                "carryover:compact".to_string(),
                format!("goal={goal}"),
                format!("run_id={run_id}"),
            ];
            if !completed_nodes.is_empty() {
                lines.push(format!(
                    "completed={}",
                    completed_summary(completed_nodes, statuses)
                ));
            }
            lines.extend(retry_lines);
            lines.extend(
                context_items
                    .into_iter()
                    .take(8)
                    .map(|(key, value)| format!("- {key}={value}")),
            );
            return lines.join("\n");
        }

        let Some((max_stages, max_context_items)) = summary_limits(&mode) else {
            return self.apply_with_options(
                "compact",
                context,
                completed_nodes,
                include_context_items,
            );
        };

        let recent_start = completed_nodes.len().saturating_sub(max_stages);
        let recent_nodes = &completed_nodes[recent_start..];
        let mut lines = vec![
            format!("carryover:{mode}"),
            format!("goal={goal}"),
            format!("run_id={run_id}"),
            format!(
                "recent_stages={}",
                completed_summary(recent_nodes, statuses)
            ),
            format!("log_entries={}", context.logs().len()),
        ];
        lines.extend(retry_lines);
        lines.extend(
            context_items
                .into_iter()
                .take(max_context_items)
                .map(|(key, value)| format!("{key}={value}")),
        );
        lines.join("\n")
    }
}

impl Default for RuntimePreambleTransform {
    fn default() -> Self {
        Self::new()
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

#[derive(Debug, Clone, PartialEq, Eq)]
struct StyleRule {
    selector: String,
    properties: BTreeMap<String, String>,
    order: usize,
}

fn apply_style_rules_to_node(
    node: &mut DotNode,
    rules: &[StyleRule],
    stylesheet_line: usize,
    graph_defaults: &BTreeMap<String, (String, usize)>,
) {
    let mut candidates: BTreeMap<String, (i32, usize, String)> = BTreeMap::new();
    for rule in rules {
        if !selector_matches(&rule.selector, node) {
            continue;
        }
        let specificity = selector_specificity(&rule.selector);
        for (property, value) in &rule.properties {
            let entry = (specificity, rule.order, value.clone());
            if candidates
                .get(property)
                .map(|current| entry > *current)
                .unwrap_or(true)
            {
                candidates.insert(property.clone(), entry);
            }
        }
    }

    for property in ALLOWED_STYLE_PROPERTIES {
        if node.explicit_attr_keys.contains(property) {
            continue;
        }

        let Some((value, line)) = candidates
            .get(property)
            .map(|(_, _, value)| (value.clone(), stylesheet_line))
            .or_else(|| graph_defaults.get(property).cloned())
            .or_else(|| {
                (!node.attrs.contains_key(property))
                    .then(|| (system_model_default(property).to_string(), 0))
            })
        else {
            continue;
        };

        node.attrs.insert(
            property.to_string(),
            DotAttribute {
                key: property.to_string(),
                value: DotValue::String(value),
                value_type: DotValueType::String,
                line,
            },
        );
    }
}

fn parse_style_rules(stylesheet: &str) -> Vec<StyleRule> {
    let text = stylesheet.trim();
    let mut rules = Vec::new();
    let mut index = 0;
    let mut order = 0;

    while index < text.len() {
        index = skip_ascii_whitespace(text, index);
        if index >= text.len() {
            break;
        }

        let Some(brace) = find_unquoted(text, '{', index) else {
            break;
        };
        let selector = text[index..brace].trim();
        let Some(close) = find_unquoted(text, '}', brace + 1) else {
            break;
        };
        let body = text[brace + 1..close].trim();

        let mut properties = BTreeMap::new();
        let mut rule_is_valid = selector_is_valid(selector);
        for statement in split_unquoted(body, ';') {
            let statement = statement.trim();
            if statement.is_empty() {
                continue;
            }
            if count_unquoted(statement, ':') != 1 {
                rule_is_valid = false;
                break;
            }
            let Some(colon) = find_unquoted(statement, ':', 0) else {
                rule_is_valid = false;
                break;
            };
            let key = statement[..colon].trim();
            let value = parse_style_value(statement[colon + 1..].trim());
            let Some(value) = value else {
                rule_is_valid = false;
                break;
            };
            if !ALLOWED_STYLE_PROPERTIES.contains(&key)
                || value.is_empty()
                || (key == "reasoning_effort"
                    && !ALLOWED_REASONING_EFFORTS.contains(&value.as_str()))
            {
                rule_is_valid = false;
                break;
            }
            properties.insert(key.to_string(), value);
        }

        if !selector.is_empty() && !properties.is_empty() && rule_is_valid {
            rules.push(StyleRule {
                selector: selector.to_string(),
                properties,
                order,
            });
        }
        order += 1;
        index = close + 1;
    }

    rules
}

fn selector_is_valid(selector: &str) -> bool {
    if selector == "*" {
        return true;
    }
    if let Some(class_name) = selector.strip_prefix('.') {
        return !class_name.is_empty()
            && class_name
                .chars()
                .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-');
    }
    if let Some(node_id) = selector.strip_prefix('#') {
        return is_node_id(node_id);
    }
    is_shape_selector(selector)
}

fn selector_matches(selector: &str, node: &DotNode) -> bool {
    let selector = selector.trim();
    if selector == "*" {
        return true;
    }
    if let Some(node_id) = selector.strip_prefix('#') {
        return node.node_id == node_id;
    }
    if let Some(class_name) = selector.strip_prefix('.') {
        let Some(class_attr) = node.attrs.get("class") else {
            return false;
        };
        return dot_value_text(&class_attr.value)
            .split(',')
            .map(str::trim)
            .any(|candidate| candidate == class_name);
    }

    let Some(shape_attr) = node.attrs.get("shape") else {
        return false;
    };
    dot_value_text(&shape_attr.value)
        .trim()
        .to_ascii_lowercase()
        == selector.trim().to_ascii_lowercase()
}

fn selector_specificity(selector: &str) -> i32 {
    let selector = selector.trim();
    if selector.starts_with('#') {
        3
    } else if selector.starts_with('.') {
        2
    } else if is_shape_selector(selector) {
        1
    } else if selector == "*" {
        0
    } else {
        -1
    }
}

fn parse_style_value(value: &str) -> Option<String> {
    if value.starts_with('"') || value.ends_with('"') {
        if !(value.starts_with('"') && value.ends_with('"')) || value.len() <= 2 {
            return None;
        }
        return unescape_quoted_style_value(&value[1..value.len() - 1]);
    }
    (!value.contains('"')).then(|| value.to_string())
}

fn unescape_quoted_style_value(value: &str) -> Option<String> {
    let mut output = String::new();
    let mut chars = value.chars();
    while let Some(ch) = chars.next() {
        if ch == '"' {
            return None;
        }
        if ch != '\\' {
            output.push(ch);
            continue;
        }
        let escaped = chars.next()?;
        match escaped {
            '"' => output.push('"'),
            '\\' => output.push('\\'),
            'n' => output.push('\n'),
            't' => output.push('\t'),
            other => {
                output.push('\\');
                output.push(other);
            }
        }
    }
    Some(output)
}

fn graph_default_model_attrs(graph: &DotGraph) -> BTreeMap<String, (String, usize)> {
    let mut defaults = BTreeMap::new();
    for property in ALLOWED_STYLE_PROPERTIES {
        let Some(attr) = graph.graph_attrs.get(property) else {
            continue;
        };
        if attr.line == 0 {
            continue;
        }
        let value = dot_value_text(&attr.value).trim().to_string();
        if value.is_empty() {
            continue;
        }
        defaults.insert(property.to_string(), (value, attr.line));
    }
    defaults
}

fn system_model_default(property: &str) -> &'static str {
    match property {
        "reasoning_effort" => "high",
        _ => "",
    }
}

fn is_shape_selector(value: &str) -> bool {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    first.is_ascii_alphabetic() && chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
}

fn is_node_id(value: &str) -> bool {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first.is_ascii_alphabetic() || first == '_')
        && chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
}

fn skip_ascii_whitespace(text: &str, start: usize) -> usize {
    let mut index = start;
    while index < text.len() {
        let Some(ch) = text[index..].chars().next() else {
            break;
        };
        if !ch.is_ascii_whitespace() {
            break;
        }
        index += ch.len_utf8();
    }
    index
}

fn find_unquoted(text: &str, token: char, start: usize) -> Option<usize> {
    let mut in_quotes = false;
    let mut escaped = false;
    for (offset, ch) in text[start..].char_indices() {
        if ch == '\\' && in_quotes && !escaped {
            escaped = true;
            continue;
        }
        if ch == '"' && !escaped {
            in_quotes = !in_quotes;
        } else if ch == token && !in_quotes {
            return Some(start + offset);
        }
        escaped = false;
    }
    None
}

fn count_unquoted(text: &str, token: char) -> usize {
    let mut count = 0;
    let mut in_quotes = false;
    let mut escaped = false;
    for ch in text.chars() {
        if ch == '\\' && in_quotes && !escaped {
            escaped = true;
            continue;
        }
        if ch == '"' && !escaped {
            in_quotes = !in_quotes;
        } else if ch == token && !in_quotes {
            count += 1;
        }
        escaped = false;
    }
    count
}

fn split_unquoted(text: &str, token: char) -> Vec<String> {
    let mut parts = Vec::new();
    let mut start = 0;
    let mut in_quotes = false;
    let mut escaped = false;
    for (index, ch) in text.char_indices() {
        if ch == '\\' && in_quotes && !escaped {
            escaped = true;
            continue;
        }
        if ch == '"' && !escaped {
            in_quotes = !in_quotes;
        } else if ch == token && !in_quotes {
            parts.push(text[start..index].to_string());
            start = index + ch.len_utf8();
        }
        escaped = false;
    }
    parts.push(text[start..].to_string());
    parts
}

fn summary_limits(mode: &str) -> Option<(usize, usize)> {
    match mode {
        "summary:low" => Some((3, 4)),
        "summary:medium" => Some((6, 8)),
        "summary:high" => Some((12, 16)),
        _ => None,
    }
}

fn completed_summary(
    completed_nodes: &[impl AsRef<str>],
    statuses: Option<&serde_json::Map<String, Value>>,
) -> String {
    completed_nodes
        .iter()
        .map(|node_id| {
            let node_id = node_id.as_ref();
            let status = statuses
                .and_then(|items| items.get(node_id))
                .map(json_value_to_text)
                .unwrap_or_default();
            format!("{node_id}:{status}")
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn carryover_context_items(snapshot: &ContextMap) -> Vec<(String, String)> {
    snapshot
        .iter()
        .filter(|(key, _)| {
            key.as_str() != "graph.goal"
                && key.as_str() != "internal.run_id"
                && !key.starts_with("_attractor.")
                && !key.starts_with("internal.")
        })
        .map(|(key, value)| (key.clone(), json_value_to_text(value)))
        .collect()
}

fn retry_context_lines(snapshot: &ContextMap) -> Vec<String> {
    let attempt = snapshot
        .get(RUNTIME_RETRY_ATTEMPT_KEY)
        .map(json_value_to_int)
        .unwrap_or_default();
    let max_attempts = snapshot
        .get(RUNTIME_RETRY_MAX_ATTEMPTS_KEY)
        .map(json_value_to_int)
        .unwrap_or_default();
    if attempt <= 0 && max_attempts <= 0 {
        return Vec::new();
    }

    let mut lines = vec![
        format!(
            "retry.node_id={}",
            snapshot
                .get(RUNTIME_RETRY_NODE_ID_KEY)
                .map(json_value_to_text)
                .unwrap_or_default()
        ),
        format!("retry.attempt={attempt}"),
        format!("retry.max_attempts={max_attempts}"),
    ];
    let failure_reason = snapshot
        .get(RUNTIME_RETRY_FAILURE_REASON_KEY)
        .map(json_value_to_text)
        .unwrap_or_default();
    if !failure_reason.is_empty() {
        lines.push(format!("retry.failure_reason={failure_reason}"));
    }
    lines
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

fn json_value_to_text(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::Bool(true) => "true".to_string(),
        Value::Bool(false) => "false".to_string(),
        Value::String(text) => text.clone(),
        Value::Number(number) => number.to_string(),
        Value::Array(_) | Value::Object(_) => {
            serde_json::to_string(value).unwrap_or_else(|_| value.to_string())
        }
    }
}

fn json_value_to_int(value: &Value) -> i64 {
    match value {
        Value::Bool(value) => i64::from(*value),
        Value::Number(number) => number
            .as_i64()
            .or_else(|| number.as_u64().and_then(|value| i64::try_from(value).ok()))
            .unwrap_or_else(|| number.to_string().trim().parse().unwrap_or_default()),
        Value::String(value) => value.trim().parse().unwrap_or_default(),
        Value::Null | Value::Array(_) | Value::Object(_) => {
            json_value_to_text(value).trim().parse().unwrap_or_default()
        }
    }
}
