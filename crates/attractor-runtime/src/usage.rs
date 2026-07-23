//! Run token-usage aggregation and cost estimation.
//!
//! A read-time projection of the journal, like transcripts: every codergen
//! backend journals a `*_request_completed` adapter event carrying the
//! resolved model and that request's token usage, so replaying those events
//! reproduces the run's usage deterministically. The executor refreshes the
//! run record from this projection at stage boundaries, which is what the
//! runs list, header strip, and Details tab render.

use std::collections::BTreeMap;

use attractor_core::JournalEntry;
use serde_json::{json, Value};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TokenUsageBucket {
    pub input_tokens: u64,
    pub cached_input_tokens: u64,
    pub output_tokens: u64,
    pub total_tokens: u64,
}

impl TokenUsageBucket {
    fn add(&mut self, other: &TokenUsageBucket) {
        self.input_tokens += other.input_tokens;
        self.cached_input_tokens += other.cached_input_tokens;
        self.output_tokens += other.output_tokens;
        self.total_tokens += other.total_tokens;
    }

    fn has_any_usage(&self) -> bool {
        self.input_tokens > 0
            || self.cached_input_tokens > 0
            || self.output_tokens > 0
            || self.total_tokens > 0
    }

    /// Lenient parse across the payload dialects backends emit: normalized
    /// `Usage` snake_case, codex camelCase, and codex `{total: {...}}`
    /// wrappers.
    pub fn from_value(value: &Value) -> Option<TokenUsageBucket> {
        let payload = value
            .get("total")
            .filter(|total| total.is_object())
            .unwrap_or(value);
        let read = |keys: &[&str]| -> u64 {
            for key in keys {
                if let Some(number) = payload.get(*key).and_then(value_as_u64) {
                    return number;
                }
            }
            0
        };
        let input_tokens = read(&["input_tokens", "inputTokens"]);
        let cached_input_tokens = read(&[
            "cached_input_tokens",
            "cachedInputTokens",
            "cache_read_tokens",
        ])
        .min(input_tokens);
        let output_tokens = read(&["output_tokens", "outputTokens"]);
        let reported_total = read(&["total_tokens", "totalTokens"]);
        let baseline_total = input_tokens + output_tokens;
        let total_tokens = reported_total.max(baseline_total);
        let bucket = TokenUsageBucket {
            input_tokens,
            cached_input_tokens,
            output_tokens,
            total_tokens,
        };
        bucket.has_any_usage().then_some(bucket)
    }

    fn to_value(self) -> Value {
        json!({
            "input_tokens": self.input_tokens,
            "cached_input_tokens": self.cached_input_tokens,
            "output_tokens": self.output_tokens,
            "total_tokens": self.total_tokens,
        })
    }
}

fn value_as_u64(value: &Value) -> Option<u64> {
    if let Some(number) = value.as_u64() {
        return Some(number);
    }
    if let Some(number) = value.as_f64() {
        if number >= 0.0 && number.fract() == 0.0 {
            return Some(number as u64);
        }
    }
    value.as_str().and_then(|text| text.trim().parse().ok())
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TokenUsageBreakdown {
    pub totals: TokenUsageBucket,
    pub by_model: BTreeMap<String, TokenUsageBucket>,
}

/// Incremental run-usage projection. Completed requests are durable totals;
/// session events are cumulative snapshots replaced per node.
#[derive(Debug, Clone, Default)]
pub struct RunUsageAccumulator {
    fallback_model: String,
    completed: TokenUsageBreakdown,
    legacy: TokenUsageBreakdown,
    in_flight: BTreeMap<String, (String, TokenUsageBucket)>,
    completed_has_usage: bool,
}

impl RunUsageAccumulator {
    pub fn new(fallback_model: impl Into<String>) -> Self {
        Self {
            fallback_model: fallback_model.into(),
            ..Self::default()
        }
    }

    pub fn from_entries(entries: &[JournalEntry], fallback_model: &str) -> Self {
        let mut accumulator = Self::new(fallback_model);
        accumulator.apply(entries);
        accumulator
    }

    pub fn apply(&mut self, entries: &[JournalEntry]) {
        for entry in entries {
            self.apply_entry(entry);
        }
    }

    pub fn breakdown(&self) -> Option<TokenUsageBreakdown> {
        let mut breakdown = if self.completed_has_usage {
            self.completed.clone()
        } else {
            self.legacy.clone()
        };
        for (model, bucket) in self.in_flight.values() {
            breakdown.add_for_model(model, bucket);
        }
        breakdown.has_any_usage().then_some(breakdown)
    }

    fn apply_entry(&mut self, entry: &JournalEntry) {
        if entry.raw_type == "LLMTokenUsage" {
            if let Some(bucket) = entry
                .payload
                .get("token_usage")
                .and_then(TokenUsageBucket::from_value)
            {
                self.legacy.add_for_model(&self.fallback_model, &bucket);
            }
            return;
        }
        let (payload, node_id, completed) = match entry.raw_type.as_str() {
            "CodergenAdapter" => {
                let event_type = entry
                    .payload
                    .get("adapter_event_type")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let node_id = entry
                    .payload
                    .get("node_id")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                if SESSION_EVENT_TYPES.contains(&event_type) {
                    if let Some((model, bucket)) =
                        usage_from_payload(entry.payload.get("payload"), &self.fallback_model)
                    {
                        self.in_flight.insert(node_id.to_string(), (model, bucket));
                    }
                    return;
                }
                if !REQUEST_COMPLETED_EVENT_TYPES.contains(&event_type) {
                    return;
                }
                (entry.payload.get("payload"), node_id, true)
            }
            "LLMRequestCompleted" => (
                entry.payload.get("payload"),
                entry
                    .payload
                    .get("node_id")
                    .and_then(Value::as_str)
                    .unwrap_or_default(),
                true,
            ),
            _ => return,
        };
        if completed {
            self.in_flight.remove(node_id);
        }
        if let Some((model, bucket)) = usage_from_payload(payload, &self.fallback_model) {
            self.completed.add_for_model(&model, &bucket);
            self.completed_has_usage = true;
        }
    }
}

fn usage_from_payload(
    payload: Option<&Value>,
    fallback_model: &str,
) -> Option<(String, TokenUsageBucket)> {
    let payload = payload?;
    let bucket = payload
        .get("token_usage")
        .and_then(TokenUsageBucket::from_value)?;
    let model = payload
        .get("model")
        .and_then(Value::as_str)
        .filter(|model| !model.trim().is_empty())
        .unwrap_or(fallback_model);
    Some((model.to_string(), bucket))
}

impl TokenUsageBreakdown {
    pub fn add_for_model(&mut self, model: &str, delta: &TokenUsageBucket) {
        let model = model.trim();
        let model = if model.is_empty() { "unknown" } else { model };
        self.totals.add(delta);
        self.by_model
            .entry(model.to_string())
            .or_default()
            .add(delta);
    }

    pub fn has_any_usage(&self) -> bool {
        self.totals.has_any_usage() || !self.by_model.is_empty()
    }

    /// The wire shape the frontend renders (`token_usage_breakdown`).
    pub fn to_value(&self) -> Value {
        let mut payload = self.totals.to_value();
        payload["by_model"] = Value::Object(
            self.by_model
                .iter()
                .map(|(model, bucket)| (model.clone(), bucket.to_value()))
                .collect(),
        );
        payload
    }
}

/// USD per million tokens: (input, cached input, output).
/// Rates checked against the official OpenAI pricing page on 2026-07-09.
const MODEL_PRICING_CATALOG: &[(&str, (f64, f64, f64))] = &[
    ("codex-mini-latest", (1.50, 0.375, 6.00)),
    ("gpt-4.1", (2.00, 0.50, 8.00)),
    ("gpt-4.1-mini", (0.40, 0.10, 1.60)),
    ("gpt-4.1-nano", (0.10, 0.025, 0.40)),
    ("gpt-5", (1.25, 0.125, 10.00)),
    ("gpt-5-codex", (1.25, 0.125, 10.00)),
    ("gpt-5-mini", (0.25, 0.025, 2.00)),
    ("gpt-5-nano", (0.05, 0.005, 0.40)),
    ("gpt-5.1", (1.25, 0.125, 10.00)),
    ("gpt-5.1-codex", (1.25, 0.125, 10.00)),
    ("gpt-5.2", (1.75, 0.175, 14.00)),
    ("gpt-5.2-codex", (1.75, 0.175, 14.00)),
    ("gpt-5.3-codex", (1.75, 0.175, 14.00)),
    ("gpt-5.4", (2.50, 0.25, 15.00)),
    ("gpt-5.4-mini", (0.75, 0.075, 4.50)),
    ("gpt-5.4-nano", (0.20, 0.02, 1.25)),
    ("gpt-5.5", (5.00, 0.50, 30.00)),
    // No dedicated 5.5 codex model id is published (Codex uses gpt-5.5);
    // the alias covers clients that report one anyway.
    ("gpt-5.5-codex", (5.00, 0.50, 30.00)),
    ("gpt-5.6-sol", (5.00, 0.50, 30.00)),
    ("gpt-5.6-terra", (2.50, 0.25, 15.00)),
    ("gpt-5.6-luna", (1.00, 0.10, 6.00)),
];

fn pricing_for_model(model: &str) -> Option<(f64, f64, f64)> {
    MODEL_PRICING_CATALOG
        .iter()
        .find(|(candidate, _)| *candidate == model)
        .map(|(_, pricing)| *pricing)
}

fn round_usd(amount: f64) -> f64 {
    (amount * 1_000_000.0).round() / 1_000_000.0
}

/// The wire shape the frontend renders (`estimated_model_cost`).
pub fn estimate_model_cost(breakdown: &TokenUsageBreakdown) -> Option<Value> {
    if !breakdown.has_any_usage() {
        return None;
    }
    let mut subtotal = 0.0f64;
    let mut by_model = serde_json::Map::new();
    let mut unpriced_models: Vec<String> = Vec::new();
    for (model, usage) in &breakdown.by_model {
        let Some((input_rate, cached_rate, output_rate)) = pricing_for_model(model) else {
            unpriced_models.push(model.clone());
            by_model.insert(
                model.clone(),
                json!({"currency": "USD", "amount": null, "status": "unpriced"}),
            );
            continue;
        };
        let uncached_input = usage.input_tokens.saturating_sub(usage.cached_input_tokens);
        let amount = (uncached_input as f64 * input_rate
            + usage.cached_input_tokens as f64 * cached_rate
            + usage.output_tokens as f64 * output_rate)
            / 1_000_000.0;
        subtotal += amount;
        by_model.insert(
            model.clone(),
            json!({"currency": "USD", "amount": round_usd(amount), "status": "estimated"}),
        );
    }
    let status = if unpriced_models.len() == breakdown.by_model.len() {
        "unpriced"
    } else if !unpriced_models.is_empty() {
        "partial_unpriced"
    } else {
        "estimated"
    };
    Some(json!({
        "currency": "USD",
        "amount": round_usd(subtotal),
        "status": status,
        "unpriced_models": unpriced_models,
        "by_model": Value::Object(by_model),
    }))
}

const REQUEST_COMPLETED_EVENT_TYPES: &[&str] = &[
    "rust_llm_adapter_request_completed",
    "rust_agent_adapter_request_completed",
    "codex_app_server_request_completed",
    "claude_code_request_completed",
];
const SESSION_EVENT_TYPES: &[&str] = &[
    "rust_agent_session_event",
    "codex_app_server_session_event",
    "claude_code_session_event",
];

/// Aggregate a run's token usage from its journal.
///
/// Each backend journals exactly one request-completed event per LLM request
/// (contract-repair retries are separate requests), carrying the resolved
/// model and that request's usage — so summing them is exact. Those arrive as
/// `CodergenAdapter` events on the live-journaling path and as typed
/// `LLMRequestCompleted` events on the legacy post-hoc path. `LLMTokenUsage`
/// events duplicate request usage on the legacy path, so they only count when
/// no completed event carried usage at all.
pub fn project_run_usage(
    entries: &[JournalEntry],
    fallback_model: &str,
) -> Option<TokenUsageBreakdown> {
    RunUsageAccumulator::from_entries(entries, fallback_model).breakdown()
}
