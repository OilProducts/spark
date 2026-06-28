use std::fmt;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::ser::SerializeStruct;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::errors::{AdapterError, AdapterErrorKind};
use crate::events::{StreamEvent, StreamEventStream, StreamEvents};

pub type RetryCallback = Arc<dyn Fn(&AdapterError, u32, f64) + Send + Sync + 'static>;

#[derive(Clone)]
pub struct RetryPolicy {
    pub max_retries: u32,
    pub base_delay: f64,
    pub max_delay: f64,
    pub backoff_multiplier: f64,
    pub jitter: bool,
    pub retry_timeouts: bool,
    pub on_retry: Option<RetryCallback>,
}

impl fmt::Debug for RetryPolicy {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RetryPolicy")
            .field("max_retries", &self.max_retries)
            .field("base_delay", &self.base_delay)
            .field("max_delay", &self.max_delay)
            .field("backoff_multiplier", &self.backoff_multiplier)
            .field("jitter", &self.jitter)
            .field("retry_timeouts", &self.retry_timeouts)
            .field("on_retry", &self.on_retry.is_some())
            .finish()
    }
}

impl PartialEq for RetryPolicy {
    fn eq(&self, other: &Self) -> bool {
        self.max_retries == other.max_retries
            && self.base_delay == other.base_delay
            && self.max_delay == other.max_delay
            && self.backoff_multiplier == other.backoff_multiplier
            && self.jitter == other.jitter
            && self.retry_timeouts == other.retry_timeouts
            && self.on_retry.is_some() == other.on_retry.is_some()
    }
}

impl Serialize for RetryPolicy {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let field_count = if self.retry_timeouts { 7 } else { 6 };
        let mut state = serializer.serialize_struct("RetryPolicy", field_count)?;
        state.serialize_field("max_retries", &self.max_retries)?;
        state.serialize_field("base_delay", &self.base_delay)?;
        state.serialize_field("max_delay", &self.max_delay)?;
        state.serialize_field("backoff_multiplier", &self.backoff_multiplier)?;
        state.serialize_field("jitter", &self.jitter)?;
        if self.retry_timeouts {
            state.serialize_field("retry_timeouts", &self.retry_timeouts)?;
        }
        state.serialize_field("on_retry", &Option::<()>::None)?;
        state.end()
    }
}

#[derive(Deserialize)]
struct RetryPolicyWire {
    #[serde(default = "default_max_retries")]
    max_retries: u32,
    #[serde(default = "default_base_delay")]
    base_delay: f64,
    #[serde(default = "default_max_delay")]
    max_delay: f64,
    #[serde(default = "default_backoff_multiplier")]
    backoff_multiplier: f64,
    #[serde(default = "default_jitter")]
    jitter: bool,
    #[serde(default)]
    retry_timeouts: bool,
    #[serde(default)]
    on_retry: Option<serde_json::Value>,
}

impl<'de> Deserialize<'de> for RetryPolicy {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = RetryPolicyWire::deserialize(deserializer)?;
        let _ = wire.on_retry;
        Ok(Self {
            max_retries: wire.max_retries,
            base_delay: wire.base_delay,
            max_delay: wire.max_delay,
            backoff_multiplier: wire.backoff_multiplier,
            jitter: wire.jitter,
            retry_timeouts: wire.retry_timeouts,
            on_retry: None,
        })
    }
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_retries: default_max_retries(),
            base_delay: default_base_delay(),
            max_delay: default_max_delay(),
            backoff_multiplier: default_backoff_multiplier(),
            jitter: default_jitter(),
            retry_timeouts: false,
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

    pub fn with_on_retry<F>(mut self, on_retry: F) -> Self
    where
        F: Fn(&AdapterError, u32, f64) + Send + Sync + 'static,
    {
        self.on_retry = Some(Arc::new(on_retry));
        self
    }

    pub fn notify_retry(&self, error: &AdapterError, attempt: u32, delay: f64) {
        if let Some(on_retry) = &self.on_retry {
            on_retry(error, attempt, delay);
        }
    }

    pub fn is_retryable_error(&self, error: &AdapterError) -> bool {
        error.retryable || (self.retry_timeouts && error.kind == AdapterErrorKind::RequestTimeout)
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

pub fn retry<T, F>(policy: &RetryPolicy, operation: F) -> Result<T, AdapterError>
where
    F: FnMut() -> Result<T, AdapterError>,
{
    retry_with_hooks(policy, operation, default_random_multiplier, default_sleep)
}

pub fn retry_with_hooks<T, F, R, S>(
    policy: &RetryPolicy,
    mut operation: F,
    mut random_multiplier: R,
    mut sleeper: S,
) -> Result<T, AdapterError>
where
    F: FnMut() -> Result<T, AdapterError>,
    R: FnMut() -> f64,
    S: FnMut(f64),
{
    let mut attempt = 0;
    loop {
        match operation() {
            Ok(value) => return Ok(value),
            Err(error) => {
                if !policy.is_retryable_error(&error) || attempt >= policy.max_retries {
                    return Err(error);
                }

                let multiplier = if policy.jitter {
                    Some(random_multiplier())
                } else {
                    None
                };
                let Some(delay) = policy.calculate_delay(attempt, Some(&error), multiplier) else {
                    return Err(error);
                };

                policy.notify_retry(&error, attempt, delay);
                sleeper(delay);
                attempt += 1;
            }
        }
    }
}

pub fn retry_stream_before_first_event<F>(
    policy: RetryPolicy,
    open_stream: F,
) -> Result<StreamEvents, AdapterError>
where
    F: FnMut() -> Result<StreamEvents, AdapterError> + Send + 'static,
{
    retry_stream_before_first_event_with_hooks(
        policy,
        open_stream,
        default_random_multiplier,
        default_sleep,
    )
}

pub fn retry_stream_before_first_event_with_hooks<F, R, S>(
    policy: RetryPolicy,
    mut open_stream: F,
    mut random_multiplier: R,
    mut sleeper: S,
) -> Result<StreamEvents, AdapterError>
where
    F: FnMut() -> Result<StreamEvents, AdapterError> + Send + 'static,
    R: FnMut() -> f64 + Send + 'static,
    S: FnMut(f64) + Send + 'static,
{
    let (stream, attempt) = retry_open_stream(
        &policy,
        &mut open_stream,
        &mut random_multiplier,
        &mut sleeper,
        0,
    )?;

    Ok(Box::new(RetryBeforeFirstEventStream {
        policy,
        open_stream,
        random_multiplier,
        sleeper,
        current: Some(stream),
        attempt,
        yielded_any: false,
        finished: false,
    }))
}

fn retry_open_stream<F, R, S>(
    policy: &RetryPolicy,
    open_stream: &mut F,
    random_multiplier: &mut R,
    sleeper: &mut S,
    mut attempt: u32,
) -> Result<(StreamEvents, u32), AdapterError>
where
    F: FnMut() -> Result<StreamEvents, AdapterError>,
    R: FnMut() -> f64,
    S: FnMut(f64),
{
    loop {
        match open_stream() {
            Ok(stream) => return Ok((stream, attempt)),
            Err(error) => {
                if !policy.is_retryable_error(&error) || attempt >= policy.max_retries {
                    return Err(error);
                }

                let multiplier = if policy.jitter {
                    Some(random_multiplier())
                } else {
                    None
                };
                let Some(delay) = policy.calculate_delay(attempt, Some(&error), multiplier) else {
                    return Err(error);
                };

                policy.notify_retry(&error, attempt, delay);
                sleeper(delay);
                attempt += 1;
            }
        }
    }
}

struct RetryBeforeFirstEventStream<F, R, S>
where
    F: FnMut() -> Result<StreamEvents, AdapterError>,
    R: FnMut() -> f64,
    S: FnMut(f64),
{
    policy: RetryPolicy,
    open_stream: F,
    random_multiplier: R,
    sleeper: S,
    current: Option<StreamEvents>,
    attempt: u32,
    yielded_any: bool,
    finished: bool,
}

impl<F, R, S> Iterator for RetryBeforeFirstEventStream<F, R, S>
where
    F: FnMut() -> Result<StreamEvents, AdapterError>,
    R: FnMut() -> f64,
    S: FnMut(f64),
{
    type Item = Result<StreamEvent, AdapterError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.finished {
            return None;
        }

        loop {
            let Some(stream) = self.current.as_mut() else {
                self.finished = true;
                return None;
            };

            match stream.next() {
                Some(Ok(event)) => {
                    self.yielded_any = true;
                    return Some(Ok(event));
                }
                Some(Err(error)) => {
                    if self.yielded_any
                        || !self.policy.is_retryable_error(&error)
                        || self.attempt >= self.policy.max_retries
                    {
                        self.finished = true;
                        return Some(Err(error));
                    }

                    let multiplier = if self.policy.jitter {
                        Some((self.random_multiplier)())
                    } else {
                        None
                    };
                    let Some(delay) =
                        self.policy
                            .calculate_delay(self.attempt, Some(&error), multiplier)
                    else {
                        self.finished = true;
                        return Some(Err(error));
                    };

                    self.policy.notify_retry(&error, self.attempt, delay);
                    (self.sleeper)(delay);
                    self.attempt += 1;

                    let _ = stream.close();
                    self.current = None;
                    match retry_open_stream(
                        &self.policy,
                        &mut self.open_stream,
                        &mut self.random_multiplier,
                        &mut self.sleeper,
                        self.attempt,
                    ) {
                        Ok((stream, attempt)) => {
                            self.current = Some(stream);
                            self.attempt = attempt;
                            continue;
                        }
                        Err(open_error) => {
                            self.finished = true;
                            return Some(Err(open_error));
                        }
                    }
                }
                None => {
                    self.finished = true;
                    return None;
                }
            }
        }
    }
}

impl<F, R, S> StreamEventStream for RetryBeforeFirstEventStream<F, R, S>
where
    F: FnMut() -> Result<StreamEvents, AdapterError> + Send,
    R: FnMut() -> f64 + Send,
    S: FnMut(f64) + Send,
{
    fn close(&mut self) -> Result<(), AdapterError> {
        self.finished = true;
        if let Some(stream) = self.current.as_mut() {
            stream.close()?;
        }
        self.current = None;
        Ok(())
    }
}

fn default_max_retries() -> u32 {
    2
}

fn default_base_delay() -> f64 {
    1.0
}

fn default_max_delay() -> f64 {
    60.0
}

fn default_backoff_multiplier() -> f64 {
    2.0
}

fn default_jitter() -> bool {
    true
}

fn default_sleep(delay: f64) {
    if delay <= 0.0 || !delay.is_finite() {
        return;
    }
    thread::sleep(Duration::from_secs_f64(delay));
}

fn default_random_multiplier() -> f64 {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.subsec_nanos())
        .unwrap_or(0);
    0.5 + (f64::from(nanos) / 999_999_999.0)
}
