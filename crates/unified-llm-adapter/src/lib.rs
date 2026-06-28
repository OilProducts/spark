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
pub mod generation;
pub mod middleware;
pub mod native;
pub mod openai_compatible;
pub mod profiles;
pub mod provider_utils;
pub mod request;
pub mod resolution;
pub mod retry;
pub mod structured;
pub mod timeouts;
pub mod tools;
pub mod usage;

pub use catalog::{get_latest_model, get_model_info, list_models, ModelCatalog, ModelInfo};
pub use client::{Client, LlmProfileRoute, ProviderAdapter};
pub use defaults::{default_client, get_default_client, set_default_client};
pub use env::{ProviderConfig, ProviderEnvironment};
pub use errors::{
    classify_grpc_code, classify_http_status_code, classify_provider_error_message,
    error_from_grpc_code, error_from_status_code, retry_after_from_headers, AdapterError,
    AdapterErrorKind, ProviderError, SDKError, SDKErrorKind,
};
pub use events::{
    managed_stream, stream_events, StreamAccumulator, StreamEvent, StreamEventStream,
    StreamEventType, StreamEvents,
};
pub use generation::{
    generate, generate_steps_with_policy, generate_steps_with_policy_and_hooks,
    generate_with_policy, generate_with_policy_and_hooks, stream, stream_with_policy,
    stream_with_policy_and_hooks, GenerateRequest, GenerateResult, GenerateStep, StepResult,
    StopWhen, StreamResult, TextStream,
};
pub use middleware::{CompleteNext, Middleware, StreamNext};
pub use native::{
    NativeCompleteRequest, NativeCompleteResponse, NativeCompleteTransport, NativeProviderAdapter,
    NativeRequestConfig, NativeStreamResponse,
};
pub use openai_compatible::{
    build_openai_compatible_chat_request, build_openai_compatible_chat_stream_request,
    translate_chat_completions_response, translate_chat_completions_response_with_headers,
    translate_chat_completions_stream_response, LiteLLMAdapter, OpenAICompatibleAdapter,
    OpenAICompatibleRequestConfig, OpenRouterAdapter,
};
pub use profiles::{
    get_llm_profile, get_llm_profile_with_env, load_llm_profiles, public_llm_profiles,
    public_llm_profiles_with_env, LlmProfile, LlmProfileConfigRoot, LlmProfileConfigurationError,
    LlmProfileEnvironment, ProcessLlmProfileEnvironment, PROFILE_CONFIG_FILE,
};
pub use provider_utils::{
    parse_sse_stream, provider_json_event_name, ProviderStreamPayloadError, ProviderStreamRecord,
    SseParser,
};
pub use request::{
    AudioData, ContentKind, ContentPart, DocumentData, FinishReason, FinishReasonKind, ImageData,
    LlmRequest, LlmResponse, Message, MessageRole, RateLimitInfo, Request, Response,
    ResponseFormat, Role, ThinkingData, ToolCall, ToolCallData, ToolResult, ToolResultData,
    Warning,
};
pub use resolution::{
    is_display_model_placeholder, resolve_effective_llm_model, resolve_effective_llm_profile,
    resolve_effective_llm_provider, resolve_effective_reasoning_effort,
    resolve_high_level_provider_and_model, ActiveLlmProfile, HighLevelLlmResolutionInputs,
    LlmResolutionInputs, ModelCapabilities, ResolvedLlmModel, DISPLAY_MODEL_PLACEHOLDER,
    RUNTIME_LAUNCH_MODEL_KEY, RUNTIME_LAUNCH_PROFILE_KEY, RUNTIME_LAUNCH_PROVIDER_KEY,
    RUNTIME_LAUNCH_REASONING_EFFORT_KEY,
};
pub use retry::{
    calculate_retry_delay, is_retryable_error, retry, retry_stream_before_first_event,
    retry_stream_before_first_event_with_hooks, retry_with_hooks, RetryCallback, RetryPolicy,
};
pub use structured::{
    generate_object, generate_object_with_policy, generate_object_with_policy_and_hooks,
    parse_structured_output, stream_object, stream_object_with_policy,
    stream_object_with_policy_and_hooks, GenerateObjectResult, StreamObjectResult,
};
pub use timeouts::{
    abort_error, check_abort, timeout_error, AbortController, AbortSignal, AdapterTimeout,
    IntoAbortReason, TimeoutConfig, DEFAULT_CONNECT_TIMEOUT_SECONDS,
    DEFAULT_REQUEST_TIMEOUT_SECONDS, DEFAULT_STREAM_READ_TIMEOUT_SECONDS,
};
pub use tools::{
    Tool, ToolChoice, ToolExecuteHandler, ToolInvocation, ToolRepair, ToolRepairInvocation,
};
pub use usage::{CostEstimate, Usage};
