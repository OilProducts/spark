use attractor_core::{
    attr_bool, attr_i64, attr_text, node_has_explicit_attr, DotGraph, DotNode, FailureKind,
    Outcome, OutcomeStatus,
};
use rand::Rng;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BackoffConfig {
    pub initial_delay_ms: u64,
    pub backoff_factor: f64,
    pub max_delay_ms: u64,
    pub jitter: bool,
}

impl BackoffConfig {
    pub fn delay_for_attempt(self, attempt: u64) -> f64 {
        let delay = self.base_delay_for_attempt(attempt);
        if self.jitter {
            delay * rand::thread_rng().gen_range(0.5..=1.5)
        } else {
            delay
        }
    }

    pub fn delay_for_attempt_with_jitter_factor(self, attempt: u64, jitter_factor: f64) -> f64 {
        let delay = self.base_delay_for_attempt(attempt);
        if self.jitter {
            delay * jitter_factor
        } else {
            delay
        }
    }

    fn base_delay_for_attempt(self, attempt: u64) -> f64 {
        let normalized_attempt = attempt.max(1);
        let delay = (self.initial_delay_ms as f64)
            * self
                .backoff_factor
                .powi((normalized_attempt.saturating_sub(1)) as i32);
        delay.min(self.max_delay_ms as f64)
    }
}

impl Default for BackoffConfig {
    fn default() -> Self {
        Self {
            initial_delay_ms: 200,
            backoff_factor: 2.0,
            max_delay_ms: 60_000,
            jitter: true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RetryPolicy {
    pub max_attempts: u64,
    pub backoff: BackoffConfig,
}

impl RetryPolicy {
    pub fn none() -> Self {
        Self {
            max_attempts: 1,
            backoff: BackoffConfig {
                initial_delay_ms: 0,
                backoff_factor: 1.0,
                max_delay_ms: 0,
                jitter: false,
            },
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetryPreset {
    None,
    Standard,
    Aggressive,
    Linear,
    Patient,
}

impl RetryPreset {
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "none" => Some(Self::None),
            "standard" => Some(Self::Standard),
            "aggressive" => Some(Self::Aggressive),
            "linear" => Some(Self::Linear),
            "patient" => Some(Self::Patient),
            _ => None,
        }
    }

    pub fn policy(self) -> RetryPolicy {
        match self {
            Self::None => RetryPolicy::none(),
            Self::Standard => RetryPolicy {
                max_attempts: 5,
                backoff: BackoffConfig::default(),
            },
            Self::Aggressive => RetryPolicy {
                max_attempts: 5,
                backoff: BackoffConfig {
                    initial_delay_ms: 500,
                    ..BackoffConfig::default()
                },
            },
            Self::Linear => RetryPolicy {
                max_attempts: 3,
                backoff: BackoffConfig {
                    initial_delay_ms: 500,
                    backoff_factor: 1.0,
                    max_delay_ms: 60_000,
                    jitter: false,
                },
            },
            Self::Patient => RetryPolicy {
                max_attempts: 3,
                backoff: BackoffConfig {
                    initial_delay_ms: 2_000,
                    backoff_factor: 3.0,
                    max_delay_ms: 60_000,
                    jitter: true,
                },
            },
        }
    }
}

pub fn retry_policy_for_node(graph: &DotGraph, node_id: &str) -> RetryPolicy {
    let Some(node) = graph.nodes.get(node_id) else {
        return RetryPolicy::none();
    };
    if let Some(preset) = attr_text(&node.attrs, "retry_policy")
        .and_then(|value| RetryPreset::parse(&value).map(RetryPreset::policy))
    {
        return preset;
    }

    let max_retries = max_retries_for_node(graph, node);
    RetryPolicy {
        max_attempts: max_retries.saturating_add(1).max(1),
        backoff: BackoffConfig::default(),
    }
}

pub fn max_retries_for_node(graph: &DotGraph, node: &DotNode) -> u64 {
    if node_has_explicit_attr(node, "max_retries") {
        return attr_i64(&node.attrs, "max_retries", 0).max(0) as u64;
    }

    let graph_default = attr_i64(&graph.graph_attrs, "default_max_retries", 0);
    if graph_default >= 0 {
        return graph_default as u64;
    }

    attr_i64(&node.attrs, "max_retries", 0).max(0) as u64
}

pub fn should_retry_outcome(outcome: &Outcome) -> bool {
    match outcome.status {
        OutcomeStatus::Retry => true,
        OutcomeStatus::Fail => {
            if matches!(
                outcome.failure_kind,
                Some(FailureKind::Business | FailureKind::Contract)
            ) {
                return false;
            }
            outcome.retryable.unwrap_or(true)
        }
        _ => false,
    }
}

pub fn should_retry_attempt(outcome: &Outcome, retries_so_far: u64, policy: &RetryPolicy) -> bool {
    retries_so_far.saturating_add(1) < policy.max_attempts && should_retry_outcome(outcome)
}

pub fn coerce_retry_exhausted_outcome(
    graph: &DotGraph,
    node_id: &str,
    outcome: &Outcome,
    retries_so_far: u64,
    max_retries: u64,
) -> Outcome {
    if !matches!(outcome.status, OutcomeStatus::Retry | OutcomeStatus::Fail) {
        return outcome.clone();
    }
    if retries_so_far < max_retries {
        return outcome.clone();
    }

    let allow_partial = graph
        .nodes
        .get(node_id)
        .is_some_and(|node| attr_bool(&node.attrs, "allow_partial", false));
    if !allow_partial {
        if outcome.status == OutcomeStatus::Fail {
            return outcome.clone();
        }
        return Outcome {
            status: OutcomeStatus::Fail,
            preferred_label: outcome.preferred_label.clone(),
            suggested_next_ids: outcome.suggested_next_ids.clone(),
            context_updates: outcome.context_updates.clone(),
            failure_reason: "max retries exceeded".to_string(),
            notes: outcome.notes.clone(),
            raw_response_text: outcome.raw_response_text.clone(),
            ..Outcome::new(OutcomeStatus::Fail)
        };
    }

    if outcome.status == OutcomeStatus::Fail && !should_retry_outcome(outcome) {
        return outcome.clone();
    }

    Outcome {
        status: OutcomeStatus::PartialSuccess,
        preferred_label: outcome.preferred_label.clone(),
        suggested_next_ids: outcome.suggested_next_ids.clone(),
        context_updates: outcome.context_updates.clone(),
        notes: if outcome.notes.trim().is_empty() {
            "retries exhausted, partial accepted".to_string()
        } else {
            outcome.notes.clone()
        },
        raw_response_text: outcome.raw_response_text.clone(),
        ..Outcome::new(OutcomeStatus::PartialSuccess)
    }
}
