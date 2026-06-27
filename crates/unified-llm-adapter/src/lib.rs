#![forbid(unsafe_code)]

//! Rust-owned Unified LLM adapter contracts for Spark runtime migration.
//!
//! This crate intentionally exposes DTOs, normalization helpers, native
//! provider adapter boundaries, and deterministic adapter policy. Production
//! live transports and packaged provider resources remain owned by later
//! milestones.

pub mod catalog;
pub mod client;
pub mod defaults;
pub mod env;
pub mod errors;
pub mod events;
pub mod middleware;
pub mod native;
pub mod profiles;
pub mod request;
pub mod resolution;
pub mod retry;
pub mod usage;

pub use catalog::{get_latest_model, get_model_info, list_models, ModelCatalog, ModelInfo};
pub use client::{Client, ProviderAdapter};
pub use defaults::{default_client, get_default_client, set_default_client};
pub use env::{ProviderConfig, ProviderEnvironment};
pub use errors::{
    classify_provider_error_message, error_from_status_code, AdapterError, AdapterErrorKind,
};
pub use events::{StreamAccumulator, StreamEvent, StreamEventType, StreamEvents};
pub use middleware::{CompleteNext, Middleware, StreamNext};
pub use native::{
    NativeCompleteRequest, NativeCompleteResponse, NativeCompleteTransport, NativeProviderAdapter,
    NativeRequestConfig,
};
pub use profiles::{
    get_llm_profile, get_llm_profile_with_env, load_llm_profiles, public_llm_profiles,
    public_llm_profiles_with_env, LlmProfile, LlmProfileConfigRoot, LlmProfileConfigurationError,
    LlmProfileEnvironment, ProcessLlmProfileEnvironment, PROFILE_CONFIG_FILE,
};
pub use request::{
    AudioData, ContentKind, ContentPart, DocumentData, FinishReason, FinishReasonKind, ImageData,
    LlmRequest, LlmResponse, Message, MessageRole, RateLimitInfo, Request, Response,
    ResponseFormat, Role, ThinkingData, ToolCall, ToolCallData, ToolResult, ToolResultData,
    Warning,
};
pub use resolution::{
    resolve_effective_llm_model, resolve_effective_llm_profile, resolve_effective_llm_provider,
    resolve_effective_reasoning_effort, LlmResolutionInputs, RUNTIME_LAUNCH_MODEL_KEY,
    RUNTIME_LAUNCH_PROFILE_KEY, RUNTIME_LAUNCH_PROVIDER_KEY, RUNTIME_LAUNCH_REASONING_EFFORT_KEY,
};
pub use retry::{calculate_retry_delay, is_retryable_error, RetryPolicy};
pub use usage::{CostEstimate, Usage};
