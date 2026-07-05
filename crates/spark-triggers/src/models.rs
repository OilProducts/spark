use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

pub const SOURCE_SCHEDULE: &str = "schedule";
pub const SOURCE_POLL: &str = "poll";
pub const SOURCE_WEBHOOK: &str = "webhook";
pub const SOURCE_FLOW_EVENT: &str = "flow_event";

pub const TERMINAL_PIPELINE_STATUSES: &[&str] = &[
    "completed",
    "failed",
    "validation_error",
    "canceled",
    "cancelled",
];

pub use spark_storage::{TriggerAction, TriggerDefinition, TriggerState, TriggerStateHistoryEntry};

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TriggerCreateRequest {
    pub name: String,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    pub source_type: String,
    pub action: Map<String, Value>,
    pub source: Map<String, Value>,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TriggerUpdateRequest {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub action: Option<Map<String, Value>>,
    #[serde(default)]
    pub source: Option<Map<String, Value>>,
    #[serde(default)]
    pub regenerate_webhook_secret: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SerializedTrigger {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    pub protected: bool,
    pub source_type: String,
    pub created_at: String,
    pub updated_at: String,
    pub action: TriggerAction,
    pub source: Value,
    pub state: TriggerState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub webhook_secret: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TriggerActivationRequest {
    pub trigger_id: String,
    pub trigger_name: String,
    pub source_type: String,
    pub action: TriggerAction,
    pub source_payload: Value,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TriggerActivationSinkOutcome {
    pub run_id: Option<String>,
    pub message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TriggerActivationOutcome {
    pub trigger_id: String,
    pub source_type: String,
    pub status: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    pub source_payload: Value,
    pub trigger: SerializedTrigger,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WebhookDispatchOutcome {
    pub response: WebhookHandleResponse,
    pub activation: TriggerActivationOutcome,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TriggerDeleteResponse {
    pub status: String,
    pub id: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WebhookHandleRequest {
    pub webhook_key: String,
    pub webhook_secret: String,
    pub request_id: Option<String>,
    pub payload: Map<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WebhookHandleResponse {
    pub ok: bool,
    pub trigger_id: String,
}

fn default_enabled() -> bool {
    true
}
