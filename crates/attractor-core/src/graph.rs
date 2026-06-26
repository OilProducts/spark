use std::collections::{BTreeMap, BTreeSet};
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::{AttractorCoreError, Result};

macro_rules! string_id {
    ($name:ident, $kind:literal) => {
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(try_from = "String", into = "String")]
        pub struct $name(String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Result<Self> {
                let value = value.into();
                let normalized = value.trim();
                if normalized.is_empty() {
                    return Err(AttractorCoreError::InvalidIdentifier {
                        kind: $kind,
                        value,
                        reason: "identifier must be non-empty".to_string(),
                    });
                }
                Ok(Self(normalized.to_string()))
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                self.as_str()
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str(self.as_str())
            }
        }

        impl From<$name> for String {
            fn from(value: $name) -> Self {
                value.0
            }
        }

        impl TryFrom<String> for $name {
            type Error = AttractorCoreError;

            fn try_from(value: String) -> Result<Self> {
                Self::new(value)
            }
        }

        impl TryFrom<&str> for $name {
            type Error = AttractorCoreError;

            fn try_from(value: &str) -> Result<Self> {
                Self::new(value)
            }
        }

        impl FromStr for $name {
            type Err = AttractorCoreError;

            fn from_str(value: &str) -> Result<Self> {
                Self::new(value)
            }
        }
    };
}

string_id!(GraphId, "graph");
string_id!(NodeId, "node");
string_id!(EdgeId, "edge");
string_id!(FlowId, "flow");
string_id!(FlowName, "flow name");

pub type AttributeValue = Value;
pub type AttributeMap = BTreeMap<String, AttributeValue>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GraphAttribute {
    pub key: String,
    pub value: AttributeValue,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NodeAttribute {
    pub key: String,
    pub value: AttributeValue,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EdgeAttribute {
    pub key: String,
    pub value: AttributeValue,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GraphRef {
    pub id: GraphId,
    #[serde(default)]
    pub attributes: AttributeMap,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NodeRef {
    pub id: NodeId,
    #[serde(default)]
    pub attributes: AttributeMap,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EdgeRef {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<EdgeId>,
    pub source: NodeId,
    pub target: NodeId,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub condition: String,
    #[serde(default)]
    pub weight: i64,
    #[serde(default)]
    pub attributes: AttributeMap,
}

impl EdgeRef {
    pub fn new(source: NodeId, target: NodeId) -> Self {
        Self {
            id: None,
            source,
            target,
            label: String::new(),
            condition: String::new(),
            weight: 0,
            attributes: AttributeMap::new(),
        }
    }

    pub fn condition_text(&self) -> &str {
        self.condition.trim()
    }
}

pub type RoutingEdge = EdgeRef;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DotValueType {
    String,
    Integer,
    Float,
    Boolean,
    Duration,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DurationLiteral {
    pub raw: String,
    pub value: i64,
    pub unit: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum DotValue {
    Null,
    String(String),
    Integer(i64),
    Float(f64),
    Boolean(bool),
    Duration(DurationLiteral),
}

impl std::fmt::Display for DotValue {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Null => Ok(()),
            Self::String(value) => formatter.write_str(value),
            Self::Integer(value) => write!(formatter, "{value}"),
            Self::Float(value) => write!(formatter, "{value}"),
            Self::Boolean(value) => write!(formatter, "{value}"),
            Self::Duration(value) => formatter.write_str(&value.raw),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DotAttribute {
    pub key: String,
    pub value: DotValue,
    pub value_type: DotValueType,
    #[serde(default)]
    pub line: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DotNode {
    pub node_id: String,
    #[serde(default)]
    pub attrs: BTreeMap<String, DotAttribute>,
    #[serde(default)]
    pub line: usize,
    #[serde(skip)]
    pub declaration_order: usize,
    #[serde(default)]
    pub explicit_attr_keys: BTreeSet<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DotEdge {
    pub source: String,
    pub target: String,
    #[serde(default)]
    pub attrs: BTreeMap<String, DotAttribute>,
    #[serde(default)]
    pub line: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct DotScopeDefaults {
    #[serde(default)]
    pub node: BTreeMap<String, DotAttribute>,
    #[serde(default)]
    pub edge: BTreeMap<String, DotAttribute>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DotSubgraphScope {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default)]
    pub attrs: BTreeMap<String, DotAttribute>,
    #[serde(default)]
    pub node_ids: Vec<String>,
    #[serde(default)]
    pub defaults: DotScopeDefaults,
    #[serde(default)]
    pub subgraphs: Vec<DotSubgraphScope>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DotGraph {
    pub graph_id: String,
    #[serde(default)]
    pub graph_attrs: BTreeMap<String, DotAttribute>,
    #[serde(default)]
    pub nodes: BTreeMap<String, DotNode>,
    #[serde(default)]
    pub edges: Vec<DotEdge>,
    #[serde(default)]
    pub defaults: DotScopeDefaults,
    #[serde(default)]
    pub subgraphs: Vec<DotSubgraphScope>,
}

impl DotGraph {
    pub fn goal(&self) -> String {
        self.graph_attrs
            .get("goal")
            .map(|attribute| attribute.value.to_string())
            .unwrap_or_default()
    }
}

pub fn attr_string(attrs: &BTreeMap<String, DotAttribute>, key: &str) -> String {
    attrs
        .get(key)
        .map(|attribute| dot_value_text(&attribute.value))
        .unwrap_or_default()
}

pub fn attr_text(attrs: &BTreeMap<String, DotAttribute>, key: &str) -> Option<String> {
    attrs
        .get(key)
        .map(|attribute| dot_value_text(&attribute.value).trim().to_string())
        .filter(|value| !value.is_empty())
}

pub fn attr_bool(attrs: &BTreeMap<String, DotAttribute>, key: &str, default: bool) -> bool {
    let Some(attribute) = attrs.get(key) else {
        return default;
    };
    match &attribute.value {
        DotValue::Boolean(value) => *value,
        DotValue::Null => false,
        _ => dot_value_text(&attribute.value)
            .trim()
            .eq_ignore_ascii_case("true"),
    }
}

pub fn attr_i64(attrs: &BTreeMap<String, DotAttribute>, key: &str, default: i64) -> i64 {
    let Some(attribute) = attrs.get(key) else {
        return default;
    };
    match &attribute.value {
        DotValue::Integer(value) => *value,
        DotValue::Boolean(value) => i64::from(*value),
        DotValue::String(value) => value.trim().parse().unwrap_or(default),
        DotValue::Float(value) => value.to_string().parse().unwrap_or(default),
        DotValue::Duration(value) => value.raw.trim().parse().unwrap_or(default),
        DotValue::Null => default,
    }
}

pub fn node_shape(node: &DotNode) -> String {
    attr_string(&node.attrs, "shape")
}

pub fn node_has_explicit_attr(node: &DotNode, key: &str) -> bool {
    if node.explicit_attr_keys.contains(key) {
        return true;
    }
    node.attrs
        .get(key)
        .is_some_and(|attribute| attribute.line > 0)
}

pub fn routing_edge_from_dot_edge(edge: &DotEdge) -> Result<RoutingEdge> {
    let mut routing_edge = RoutingEdge::new(
        NodeId::try_from(edge.source.as_str())?,
        NodeId::try_from(edge.target.as_str())?,
    );
    routing_edge.label = attr_string(&edge.attrs, "label");
    routing_edge.condition = attr_string(&edge.attrs, "condition");
    routing_edge.weight = attr_i64(&edge.attrs, "weight", 0);
    routing_edge.attributes = edge
        .attrs
        .iter()
        .map(|(key, attribute)| (key.clone(), dot_value_to_json(&attribute.value)))
        .collect();
    Ok(routing_edge)
}

pub fn dot_value_to_json(value: &DotValue) -> Value {
    match value {
        DotValue::Null => Value::Null,
        DotValue::String(value) => Value::String(value.clone()),
        DotValue::Integer(value) => Value::Number(serde_json::Number::from(*value)),
        DotValue::Float(value) => serde_json::Number::from_f64(*value)
            .map(Value::Number)
            .unwrap_or(Value::Null),
        DotValue::Boolean(value) => Value::Bool(*value),
        DotValue::Duration(value) => Value::String(value.raw.clone()),
    }
}

pub fn dot_value_text(value: &DotValue) -> String {
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
