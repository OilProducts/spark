use std::collections::BTreeMap;

use serde_json::Value;

use crate::errors::AdapterError;
use crate::events::{stream_events, StreamEvents};
use crate::request::{Request, Response};

use super::anthropic::{
    build_anthropic_messages_request, build_anthropic_messages_stream_request,
    translate_anthropic_messages_response,
};
use super::common::{configuration_error, normalize_provider, normalize_rate_limit_headers};
use super::gemini::{
    build_gemini_generate_content_request, build_gemini_stream_generate_content_request,
    translate_gemini_generate_content_response,
};
use super::openai::{
    build_openai_responses_request, build_openai_responses_stream_request,
    translate_openai_responses_response,
};
use super::streaming::native_translated_stream;
use super::types::{NativeCompleteRequest, NativeRequestConfig, NativeStreamBody};

pub fn build_native_complete_request<C>(
    provider: &str,
    request: &Request,
    config: C,
) -> Result<NativeCompleteRequest, AdapterError>
where
    C: Into<NativeRequestConfig>,
{
    match normalize_provider(provider).as_str() {
        "openai" => build_openai_responses_request(request, config),
        "anthropic" => build_anthropic_messages_request(request, config),
        "gemini" => build_gemini_generate_content_request(request, config),
        other => Err(configuration_error(
            other,
            format!("Unsupported native provider {other:?}"),
        )),
    }
}

pub fn build_native_stream_request<C>(
    provider: &str,
    request: &Request,
    config: C,
) -> Result<NativeCompleteRequest, AdapterError>
where
    C: Into<NativeRequestConfig>,
{
    match normalize_provider(provider).as_str() {
        "openai" => build_openai_responses_stream_request(request, config),
        "anthropic" => build_anthropic_messages_stream_request(request, config),
        "gemini" => build_gemini_stream_generate_content_request(request, config),
        other => Err(configuration_error(
            other,
            format!("Unsupported native provider {other:?}"),
        )),
    }
}
pub fn translate_native_complete_response(
    provider: &str,
    payload: Value,
) -> Result<Response, AdapterError> {
    translate_native_complete_response_with_headers(provider, payload, &BTreeMap::new())
}

pub fn translate_native_complete_response_with_headers(
    provider: &str,
    payload: Value,
    headers: &BTreeMap<String, String>,
) -> Result<Response, AdapterError> {
    let rate_limit = normalize_rate_limit_headers(headers);
    match normalize_provider(provider).as_str() {
        "openai" => translate_openai_responses_response(payload, rate_limit),
        "anthropic" => translate_anthropic_messages_response(payload, rate_limit),
        "gemini" => translate_gemini_generate_content_response(payload, rate_limit),
        other => Err(configuration_error(
            other,
            format!("Unsupported native provider {other:?}"),
        )),
    }
}

pub fn translate_native_stream_response(
    provider: &str,
    body: NativeStreamBody,
    headers: &BTreeMap<String, String>,
) -> StreamEvents {
    let provider = normalize_provider(provider);
    let headers = headers.clone();
    match provider.as_str() {
        "openai" | "anthropic" | "gemini" => native_translated_stream(provider, headers, body),
        other => stream_events(
            vec![Err(configuration_error(
                other,
                format!("Unsupported native provider {other:?}"),
            ))]
            .into_iter(),
        ),
    }
}
