use serde::{Deserialize, Serialize};

use crate::errors::AdapterError;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct RetryPolicy {
    pub max_retries: u32,
    pub base_delay: f64,
    pub max_delay: f64,
    pub backoff_multiplier: f64,
    pub jitter: bool,
    #[serde(default)]
    pub on_retry: Option<()>,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_retries: 2,
            base_delay: 1.0,
            max_delay: 60.0,
            backoff_multiplier: 2.0,
            jitter: true,
            on_retry: None,
        }
    }
}

impl RetryPolicy {
    pub fn calculate_delay(
        &self,
        attempt: u32,
        error: Option<&AdapterError>,
        random_multiplier: Option<f64>,
    ) -> Option<f64> {
        if let Some(retry_after) = error.and_then(|error| error.retry_after) {
            if retry_after > self.max_delay {
                return None;
            }
            return Some(retry_after);
        }

        let mut delay = self.base_delay
            * self
                .backoff_multiplier
                .powi(i32::try_from(attempt).unwrap_or(i32::MAX));
        delay = delay.min(self.max_delay);
        if self.jitter {
            delay *= random_multiplier.unwrap_or(1.0).clamp(0.5, 1.5);
        }
        Some(delay)
    }
}

pub fn calculate_retry_delay(
    policy: &RetryPolicy,
    attempt: u32,
    error: Option<&AdapterError>,
    random_multiplier: Option<f64>,
) -> Option<f64> {
    policy.calculate_delay(attempt, error, random_multiplier)
}

pub fn is_retryable_error(error: &AdapterError) -> bool {
    error.retryable
}
