use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use base64::Engine as _;
use serde_json::{json, Value};
use unified_llm_adapter::native::{
    build_anthropic_messages_request, build_gemini_generate_content_request,
    build_native_complete_request, build_openai_responses_request,
    translate_native_complete_response, translate_native_complete_response_with_headers,
};
use unified_llm_adapter::{
    AdapterErrorKind, AudioData, ContentPart, DocumentData, FinishReasonKind, ImageData, Message,
    MessageRole, NativeRequestConfig, Request, ResponseFormat, ThinkingData, ToolCall,
};

#[test]
fn openai_complete_request_uses_responses_api_and_native_body_shape() {
    let request = Request {
        model: "gpt-5.2".to_string(),
        messages: vec![
            Message::system("system instructions"),
            Message::developer("developer instructions"),
            Message::user("hello"),
            Message {
                role: MessageRole::Assistant,
                content: vec![
                    ContentPart::Text {
                        text: "use a tool".to_string(),
                    },
                    ContentPart::ToolCall {
                        tool_call: tool_call("call_1", "lookup", json!({"query": "Paris"})),
                    },
                ],
                ..Message::default()
            },
            Message::tool_result("call_1", json!({"answer": "72F"}), false),
        ],
        tools: vec![tool("lookup")],
        tool_choice: Some(json!({"mode": "named", "tool_name": "lookup"})),
        response_format: Some(ResponseFormat::JsonSchema {
            json_schema: json!({
                "type": "object",
                "properties": {"answer": {"type": "string"}},
            }),
            strict: true,
        }),
        temperature: Some(0.2),
        top_p: Some(0.9),
        max_tokens: Some(256),
        stop_sequences: vec!["END".to_string()],
        reasoning_effort: Some("high".to_string()),
        metadata: BTreeMap::from([("run_id".to_string(), json!("run-1"))]),
        provider_options: BTreeMap::from([
            (
                "openai".to_string(),
                json!({
                    "model": "wrong-model",
                    "parallel_tool_calls": false,
                    "tools": [{"type": "web_search_preview"}],
                }),
            ),
            (
                "anthropic".to_string(),
                json!({"beta_headers": ["should-not-leak"]}),
            ),
            (
                "gemini".to_string(),
                json!({"safetySettings": [{"category": "x"}]}),
            ),
        ]),
        ..Request::default()
    };
    let config = NativeRequestConfig {
        api_key: Some("openai-key".to_string()),
        base_url: Some("https://openai.example/api".to_string()),
        default_headers: BTreeMap::from([
            ("Authorization".to_string(), "wrong".to_string()),
            ("X-Custom".to_string(), "value".to_string()),
        ]),
        organization: Some("org-123".to_string()),
        project: Some("project-456".to_string()),
    };

    let prepared = build_openai_responses_request(&request, &config).unwrap();

    assert_eq!(prepared.method, "POST");
    assert_eq!(prepared.url, "https://openai.example/api/v1/responses");
    assert!(!prepared.url.contains("chat/completions"));
    assert_eq!(prepared.headers["Authorization"], "Bearer openai-key");
    assert_eq!(prepared.headers["OpenAI-Organization"], "org-123");
    assert_eq!(prepared.headers["OpenAI-Project"], "project-456");
    assert_eq!(prepared.headers["X-Custom"], "value");

    let body = object(&prepared.body);
    assert_eq!(body["model"], json!("gpt-5.2"));
    assert_eq!(
        body["instructions"],
        json!("system instructions\n\ndeveloper instructions")
    );
    assert!(body.get("messages").is_none());
    assert_eq!(
        body["tool_choice"],
        json!({"type": "function", "function": {"name": "lookup"}})
    );
    assert_eq!(body["temperature"], json!(0.2));
    assert_eq!(body["top_p"], json!(0.9));
    assert_eq!(body["max_output_tokens"], json!(256));
    assert_eq!(body["stop"], json!(["END"]));
    assert_eq!(body["metadata"], json!({"run_id": "run-1"}));
    assert_eq!(body["reasoning"], json!({"effort": "high"}));
    assert_eq!(
        body["response_format"],
        json!({
            "type": "json_schema",
            "json_schema": {
                "type": "object",
                "properties": {"answer": {"type": "string"}},
            },
            "strict": true,
        })
    );
    assert_eq!(body["parallel_tool_calls"], json!(false));
    assert_eq!(
        body["tools"],
        json!([
            {
                "type": "function",
                "function": {
                    "name": "lookup",
                    "description": "Lookup a fact",
                    "parameters": {
                        "type": "object",
                        "properties": {"query": {"type": "string"}},
                    },
                },
            },
            {"type": "web_search_preview"},
        ])
    );
    assert_eq!(
        body["input"],
        json!([
            {
                "type": "message",
                "role": "user",
                "content": [{"type": "input_text", "text": "hello"}],
            },
            {
                "type": "message",
                "role": "assistant",
                "content": [{"type": "output_text", "text": "use a tool"}],
            },
            {
                "type": "function_call",
                "id": "call_1",
                "name": "lookup",
                "arguments": "{\"query\":\"Paris\"}",
            },
            {
                "type": "function_call_output",
                "call_id": "call_1",
                "output": "{\"answer\":\"72F\"}",
            },
        ])
    );
    assert!(!prepared.body.to_string().contains("should-not-leak"));
    assert!(!prepared.body.to_string().contains("safetySettings"));
}

#[test]
fn openai_provider_options_ignore_unsupported_active_keys_without_breaking_request() {
    let request = Request {
        model: "gpt-5.2".to_string(),
        messages: vec![Message::user("hello")],
        reasoning_effort: Some("high".to_string()),
        provider_options: BTreeMap::from([
            (
                "openai".to_string(),
                json!({
                    "parallel_tool_calls": false,
                    "reasoning": {
                        "effort": "low",
                        "summary": "auto"
                    },
                    "tools": [{"type": "web_search_preview"}],
                    "unsupportedNativeKey": {"should": "drop"},
                    "gemini": {"safetySettings": [{"category": "leak"}]},
                    "anthropic": {"beta_headers": ["leak"]},
                }),
            ),
            (
                "gemini".to_string(),
                json!({"safetySettings": [{"category": "non-active"}]}),
            ),
        ]),
        ..Request::default()
    };
    let original_provider_options = request.provider_options.clone();

    let prepared =
        build_openai_responses_request(&request, NativeRequestConfig::default()).unwrap();

    let body = object(&prepared.body);
    assert_eq!(body["parallel_tool_calls"], json!(false));
    assert_eq!(
        body["reasoning"],
        json!({
            "effort": "high",
            "summary": "auto",
        })
    );
    assert_eq!(body["tools"], json!([{"type": "web_search_preview"}]));
    assert!(!body.contains_key("unsupportedNativeKey"));
    assert!(!body.contains_key("gemini"));
    assert!(!body.contains_key("anthropic"));
    assert!(!prepared.body.to_string().contains("safetySettings"));
    assert!(!prepared.body.to_string().contains("beta_headers"));
    assert_eq!(request.provider_options, original_provider_options);
}

#[test]
fn anthropic_complete_request_uses_messages_api_headers_and_role_translation() {
    let request = Request {
        model: "claude-sonnet-4-5".to_string(),
        messages: vec![
            Message::system("system instructions"),
            Message::developer("developer instructions"),
            Message::user("first user"),
            Message {
                role: MessageRole::Assistant,
                content: vec![ContentPart::ToolCall {
                    tool_call: tool_call("call_1", "lookup", json!({"query": "Paris"})),
                }],
                ..Message::default()
            },
            Message::tool_result("call_1", json!({"answer": "72F"}), false),
            Message::user("follow up"),
        ],
        tools: vec![tool("lookup")],
        tool_choice: Some(json!({"mode": "named", "tool_name": "lookup"})),
        response_format: Some(ResponseFormat::JsonObject),
        temperature: Some(0.3),
        top_p: Some(0.8),
        stop_sequences: vec!["STOP".to_string()],
        metadata: BTreeMap::from([("run_id".to_string(), json!("run-2"))]),
        provider_options: BTreeMap::from([
            (
                "anthropic".to_string(),
                json!({
                    "beta_headers": ["tools-2025-01-01", "tools-2025-01-01", "thinking-2024-01-01"],
                    "thinking": {"type": "enabled", "budget_tokens": 128},
                }),
            ),
            ("openai".to_string(), json!({"parallel_tool_calls": false})),
            ("gemini".to_string(), json!({"topK": 4})),
        ]),
        ..Request::default()
    };
    let config = NativeRequestConfig {
        api_key: Some("anthropic-key".to_string()),
        base_url: Some("https://anthropic.example/custom/messages".to_string()),
        default_headers: BTreeMap::from([
            ("Authorization".to_string(), "wrong".to_string()),
            ("X-Custom".to_string(), "value".to_string()),
        ]),
        ..NativeRequestConfig::default()
    };

    let prepared = build_anthropic_messages_request(&request, &config).unwrap();

    assert_eq!(prepared.method, "POST");
    assert_eq!(prepared.url, "https://anthropic.example/custom/v1/messages");
    assert_eq!(prepared.headers["x-api-key"], "anthropic-key");
    assert_eq!(prepared.headers["anthropic-version"], "2023-06-01");
    assert_eq!(
        prepared.headers["anthropic-beta"],
        "tools-2025-01-01,thinking-2024-01-01,prompt-caching-2024-07-31"
    );
    assert_eq!(prepared.headers["X-Custom"], "value");
    assert!(!prepared
        .headers
        .keys()
        .any(|header| header.eq_ignore_ascii_case("authorization")));

    let body = object(&prepared.body);
    assert_eq!(body["model"], json!("claude-sonnet-4-5"));
    assert_eq!(body["max_tokens"], json!(4096));
    assert_eq!(body["temperature"], json!(0.3));
    assert_eq!(body["top_p"], json!(0.8));
    assert_eq!(body["stop_sequences"], json!(["STOP"]));
    assert_eq!(body["metadata"], json!({"run_id": "run-2"}));
    assert_eq!(
        body["system"],
        json!([{
            "type": "text",
            "text": "system instructions\n\ndeveloper instructions\n\nReturn only valid JSON.",
            "cache_control": {"type": "ephemeral"},
        }])
    );
    assert_eq!(
        body["thinking"],
        json!({"type": "enabled", "budget_tokens": 128})
    );
    assert_eq!(
        body["tools"],
        json!([{
            "name": "lookup",
            "description": "Lookup a fact",
            "input_schema": {
                "type": "object",
                "properties": {"query": {"type": "string"}},
            },
            "cache_control": {"type": "ephemeral"},
        }])
    );
    assert_eq!(
        body["tool_choice"],
        json!({"type": "tool", "name": "lookup"})
    );
    assert_eq!(
        body["messages"],
        json!([
            {
                "role": "user",
                "content": [{"type": "text", "text": "first user"}],
            },
            {
                "role": "assistant",
                "content": [{
                    "type": "tool_use",
                    "id": "call_1",
                    "name": "lookup",
                    "input": {"query": "Paris"},
                    "cache_control": {"type": "ephemeral"},
                }],
            },
            {
                "role": "user",
                "content": [
                    {
                        "type": "tool_result",
                        "tool_use_id": "call_1",
                        "content": "{\"answer\":\"72F\"}",
                    },
                    {"type": "text", "text": "follow up"},
                ],
            },
        ])
    );
    assert!(!prepared.body.to_string().contains("parallel_tool_calls"));
    assert!(!prepared.body.to_string().contains("topK"));
    assert_eq!(count_key(&prepared.body, "cache_control"), 3);
}

#[test]
fn anthropic_complete_request_round_trips_thinking_blocks_for_same_provider_continuation() {
    let request = Request {
        model: "claude-sonnet-4-5".to_string(),
        messages: vec![
            Message::user("hello"),
            Message {
                role: MessageRole::Assistant,
                content: vec![
                    ContentPart::Thinking {
                        thinking: ThinkingData {
                            text: "reasoning".to_string(),
                            signature: Some("sig-123".to_string()),
                            redacted: false,
                            source_provider: Some("anthropic".to_string()),
                            source_model: Some("claude-sonnet-4-5".to_string()),
                        },
                    },
                    ContentPart::RedactedThinking {
                        thinking: ThinkingData {
                            text: "  opaque-123  ".to_string(),
                            signature: None,
                            redacted: true,
                            source_provider: Some("anthropic".to_string()),
                            source_model: Some("claude-sonnet-4-5".to_string()),
                        },
                    },
                    ContentPart::Text {
                        text: "visible answer".to_string(),
                    },
                ],
                ..Message::default()
            },
            Message::user("continue"),
        ],
        provider_options: BTreeMap::from([("anthropic".to_string(), json!({"auto_cache": false}))]),
        ..Request::default()
    };

    let prepared =
        build_anthropic_messages_request(&request, NativeRequestConfig::default()).unwrap();

    assert_eq!(
        prepared.body["messages"][1]["content"],
        json!([
            {
                "type": "thinking",
                "thinking": "reasoning",
                "signature": "sig-123",
            },
            {
                "type": "redacted_thinking",
                "data": "  opaque-123  ",
            },
            {
                "type": "text",
                "text": "visible answer",
            },
        ])
    );
    assert_eq!(count_key(&prepared.body, "cache_control"), 0);
}

#[test]
fn anthropic_complete_request_rejects_cross_provider_signed_thinking() {
    let gemini_response = translate_native_complete_response(
        "gemini",
        json!({
            "responseId": "gemini-thinking",
            "modelVersion": "gemini-3.1-pro-preview",
            "candidates": [{
                "content": {
                    "parts": [{
                        "text": "private thought",
                        "thought": true,
                        "thoughtSignature": "gemini-sig",
                    }],
                },
            }],
        }),
    )
    .unwrap();
    let request = Request {
        model: "claude-sonnet-4-5".to_string(),
        messages: vec![gemini_response.message],
        provider_options: BTreeMap::from([("anthropic".to_string(), json!({"auto_cache": false}))]),
        ..Request::default()
    };

    let error =
        build_anthropic_messages_request(&request, NativeRequestConfig::default()).unwrap_err();

    assert_eq!(error.kind, AdapterErrorKind::InvalidRequest);
    assert_eq!(error.provider.as_deref(), Some("anthropic"));
    assert!(error.message.contains("gemini"));
    assert!(error.message.contains("same-provider continuation"));
}

#[test]
fn gemini_complete_request_rejects_cross_provider_signed_and_redacted_thinking() {
    let anthropic_signed = translate_native_complete_response(
        "anthropic",
        json!({
            "id": "anthropic-thinking",
            "model": "claude-sonnet-4-5",
            "content": [{
                "type": "thinking",
                "thinking": "private reasoning",
                "signature": "anthropic-sig",
            }],
        }),
    )
    .unwrap();
    let signed_request = Request {
        model: "gemini-3.1-pro-preview".to_string(),
        messages: vec![anthropic_signed.message],
        ..Request::default()
    };

    let error =
        build_gemini_generate_content_request(&signed_request, NativeRequestConfig::default())
            .unwrap_err();

    assert_eq!(error.kind, AdapterErrorKind::InvalidRequest);
    assert_eq!(error.provider.as_deref(), Some("gemini"));
    assert!(error.message.contains("anthropic"));
    assert!(error.message.contains("same-provider continuation"));

    let anthropic_redacted = translate_native_complete_response(
        "anthropic",
        json!({
            "id": "anthropic-redacted",
            "model": "claude-sonnet-4-5",
            "content": [{
                "type": "redacted_thinking",
                "data": "opaque-redacted-payload",
            }],
        }),
    )
    .unwrap();
    let redacted_request = Request {
        model: "gemini-3.1-pro-preview".to_string(),
        messages: vec![anthropic_redacted.message],
        ..Request::default()
    };

    let error =
        build_gemini_generate_content_request(&redacted_request, NativeRequestConfig::default())
            .unwrap_err();

    assert_eq!(error.kind, AdapterErrorKind::InvalidRequest);
    assert_eq!(error.provider.as_deref(), Some("gemini"));
    assert!(error.message.contains("redacted_thinking"));
}

#[test]
fn signed_or_redacted_thinking_requires_source_provenance_for_native_continuation() {
    let signed_request = Request {
        model: "claude-sonnet-4-5".to_string(),
        messages: vec![Message {
            role: MessageRole::Assistant,
            content: vec![ContentPart::Thinking {
                thinking: ThinkingData {
                    text: "private reasoning".to_string(),
                    signature: Some("sig-without-source".to_string()),
                    redacted: false,
                    source_provider: None,
                    source_model: None,
                },
            }],
            ..Message::default()
        }],
        provider_options: BTreeMap::from([("anthropic".to_string(), json!({"auto_cache": false}))]),
        ..Request::default()
    };

    let error = build_anthropic_messages_request(&signed_request, NativeRequestConfig::default())
        .unwrap_err();

    assert_eq!(error.kind, AdapterErrorKind::InvalidRequest);
    assert_eq!(error.provider.as_deref(), Some("anthropic"));
    assert!(error.message.contains("source_provider"));

    let redacted_request = Request {
        model: "claude-sonnet-4-5".to_string(),
        messages: vec![Message {
            role: MessageRole::Assistant,
            content: vec![ContentPart::RedactedThinking {
                thinking: ThinkingData {
                    text: "opaque".to_string(),
                    signature: None,
                    redacted: true,
                    source_provider: None,
                    source_model: None,
                },
            }],
            ..Message::default()
        }],
        provider_options: BTreeMap::from([("anthropic".to_string(), json!({"auto_cache": false}))]),
        ..Request::default()
    };

    let error = build_anthropic_messages_request(&redacted_request, NativeRequestConfig::default())
        .unwrap_err();

    assert_eq!(error.kind, AdapterErrorKind::InvalidRequest);
    assert_eq!(error.provider.as_deref(), Some("anthropic"));
    assert!(error.message.contains("source_provider"));
}

#[test]
fn anthropic_cache_control_preserves_explicit_annotations_when_auto_cache_is_disabled() {
    let mut explicit_tool = tool("lookup");
    explicit_tool.as_object_mut().unwrap().insert(
        "cache_control".to_string(),
        json!({"type": "ephemeral", "ttl": "1h"}),
    );
    let request = Request {
        model: "claude-sonnet-4-5".to_string(),
        messages: vec![
            Message {
                role: MessageRole::System,
                content: vec![ContentPart::Provider {
                    raw: json!({
                        "type": "text",
                        "text": "explicit system",
                        "cache_control": {"type": "ephemeral", "ttl": "1h"},
                    }),
                }],
                ..Message::default()
            },
            Message {
                role: MessageRole::User,
                content: vec![ContentPart::Text {
                    text: "explicit user".to_string(),
                }],
                provider_metadata: BTreeMap::from([(
                    "anthropic".to_string(),
                    json!({"cache_control": {"type": "ephemeral", "ttl": "5m"}}),
                )]),
                ..Message::default()
            },
        ],
        tools: vec![explicit_tool],
        provider_options: BTreeMap::from([(
            "anthropic".to_string(),
            json!({
                "auto_cache": false,
                "beta_headers": [
                    "prompt-caching-2024-07-31",
                    "custom-beta",
                    "prompt-caching-2024-07-31"
                ],
            }),
        )]),
        ..Request::default()
    };

    let prepared =
        build_anthropic_messages_request(&request, NativeRequestConfig::default()).unwrap();

    assert_eq!(
        prepared.headers["anthropic-beta"],
        "prompt-caching-2024-07-31,custom-beta"
    );
    let body = object(&prepared.body);
    assert_eq!(
        body["system"],
        json!([{
            "type": "text",
            "text": "explicit system",
            "cache_control": {"type": "ephemeral", "ttl": "1h"},
        }])
    );
    assert_eq!(
        body["tools"][0]["cache_control"],
        json!({"type": "ephemeral", "ttl": "1h"})
    );
    assert_eq!(
        body["messages"][0]["content"],
        json!([{
            "type": "text",
            "text": "explicit user",
            "cache_control": {"type": "ephemeral", "ttl": "5m"},
        }])
    );
    assert_eq!(count_key(&prepared.body, "cache_control"), 3);
}

#[test]
fn anthropic_auto_cache_false_disables_automatic_breakpoints() {
    let request = Request {
        model: "claude-sonnet-4-5".to_string(),
        messages: vec![
            Message::system("system instructions"),
            Message::user("cached user context"),
            Message::assistant("cached assistant context"),
            Message::user("hello"),
        ],
        tools: vec![tool("lookup")],
        provider_options: BTreeMap::from([("anthropic".to_string(), json!({"auto_cache": false}))]),
        ..Request::default()
    };

    let prepared =
        build_anthropic_messages_request(&request, NativeRequestConfig::default()).unwrap();

    assert!(!prepared.headers.contains_key("anthropic-beta"));
    assert_eq!(count_key(&prepared.body, "cache_control"), 0);
    let body = object(&prepared.body);
    assert_eq!(body["system"], json!("system instructions"));
    assert_eq!(
        body["tools"],
        json!([{
            "name": "lookup",
            "description": "Lookup a fact",
            "input_schema": {
                "type": "object",
                "properties": {"query": {"type": "string"}},
            },
        }])
    );
    assert_eq!(
        body["messages"][1]["content"],
        json!([{"type": "text", "text": "cached assistant context"}])
    );
}

#[test]
fn gemini_complete_request_uses_generate_content_endpoint_key_and_native_parts() {
    let request = Request {
        model: "models/gemini-3.1-pro-preview".to_string(),
        messages: vec![
            Message::system("system instructions"),
            Message::developer("developer instructions"),
            Message {
                role: MessageRole::User,
                content: vec![
                    ContentPart::Text {
                        text: "hello".to_string(),
                    },
                    ContentPart::Image {
                        image: ImageData::data(b"image".to_vec(), Some("image/png".to_string())),
                    },
                ],
                ..Message::default()
            },
            Message {
                role: MessageRole::Assistant,
                content: vec![
                    ContentPart::Text {
                        text: "assistant turn".to_string(),
                    },
                    ContentPart::ToolCall {
                        tool_call: tool_call("call_1", "lookup", json!({"query": "Paris"})),
                    },
                ],
                ..Message::default()
            },
            Message::tool_result("call_1", json!("72F"), false),
        ],
        tools: vec![tool("lookup")],
        tool_choice: Some(json!("required")),
        response_format: Some(ResponseFormat::JsonSchema {
            json_schema: json!({
                "type": "object",
                "properties": {"answer": {"type": "string"}},
            }),
            strict: false,
        }),
        temperature: Some(0.4),
        top_p: Some(0.7),
        max_tokens: Some(512),
        stop_sequences: vec!["END".to_string()],
        provider_options: BTreeMap::from([
            (
                "gemini".to_string(),
                json!({
                    "safetySettings": [{"category": "HARM_CATEGORY_DANGEROUS_CONTENT", "threshold": "BLOCK_ONLY_HIGH"}],
                    "generationConfig": {
                        "temperature": 0.99,
                        "thinkingConfig": {"includeThoughts": true}
                    },
                    "topK": 40,
                }),
            ),
            ("openai".to_string(), json!({"parallel_tool_calls": false})),
            (
                "anthropic".to_string(),
                json!({"beta_headers": ["should-not-leak"]}),
            ),
        ]),
        ..Request::default()
    };
    let config = NativeRequestConfig {
        api_key: Some("gemini key".to_string()),
        base_url: Some("https://gemini.example/custom/v1/models/old:generateContent".to_string()),
        default_headers: BTreeMap::from([
            ("Authorization".to_string(), "wrong".to_string()),
            ("X-Custom".to_string(), "value".to_string()),
        ]),
        ..NativeRequestConfig::default()
    };

    let prepared = build_gemini_generate_content_request(&request, &config).unwrap();

    assert_eq!(prepared.method, "POST");
    assert_eq!(
        prepared.url,
        "https://gemini.example/custom/v1beta/models/gemini-3.1-pro-preview:generateContent?key=gemini+key"
    );
    assert_eq!(prepared.headers["X-Custom"], "value");
    assert!(!prepared
        .headers
        .keys()
        .any(|header| header.eq_ignore_ascii_case("authorization")));

    let body = object(&prepared.body);
    assert_eq!(
        body["systemInstruction"],
        json!({"parts": [{"text": "system instructions\n\ndeveloper instructions"}]})
    );
    assert_eq!(
        body["tools"],
        json!([{
            "functionDeclarations": [{
                "name": "lookup",
                "description": "Lookup a fact",
                "parametersJsonSchema": {
                    "type": "object",
                    "properties": {"query": {"type": "string"}},
                },
            }],
        }])
    );
    assert_eq!(
        body["toolConfig"],
        json!({"functionCallingConfig": {"mode": "ANY"}})
    );
    assert_eq!(
        body["generationConfig"],
        json!({
            "temperature": 0.4,
            "topP": 0.7,
            "maxOutputTokens": 512,
            "stopSequences": ["END"],
            "responseMimeType": "application/json",
            "responseSchema": {
                "type": "object",
                "properties": {"answer": {"type": "string"}},
            },
            "thinkingConfig": {"includeThoughts": true},
            "topK": 40,
        })
    );
    assert_eq!(
        body["contents"],
        json!([
            {
                "role": "user",
                "parts": [
                    {"text": "hello"},
                    {"inlineData": {"data": "aW1hZ2U=", "mimeType": "image/png"}},
                ],
            },
            {
                "role": "model",
                "parts": [
                    {"text": "assistant turn"},
                    {"functionCall": {"name": "lookup", "args": {"query": "Paris"}}},
                ],
            },
            {
                "role": "user",
                "parts": [{
                    "functionResponse": {
                        "id": "call_1",
                        "name": "lookup",
                        "response": {"result": "72F"},
                    },
                }],
            },
        ])
    );
    assert_eq!(
        body["safetySettings"],
        json!([{
            "category": "HARM_CATEGORY_DANGEROUS_CONTENT",
            "threshold": "BLOCK_ONLY_HIGH",
        }])
    );
    assert!(!prepared.body.to_string().contains("parallel_tool_calls"));
    assert!(!prepared.body.to_string().contains("should-not-leak"));
}

#[test]
fn gemini_provider_options_ignore_unsupported_active_keys_and_merge_grounding_tools() {
    let request = Request {
        model: "gemini-3.1-pro-preview".to_string(),
        messages: vec![Message::user("return grounded answer")],
        tools: vec![tool("lookup")],
        temperature: Some(0.4),
        provider_options: BTreeMap::from([
            (
                "gemini".to_string(),
                json!({
                    "safetySettings": [{
                        "category": "HARM_CATEGORY_HATE_SPEECH",
                        "threshold": "BLOCK_ONLY_HIGH"
                    }],
                    "cachedContent": "cachedContents/session-123",
                    "groundingConfig": {"mode": "google_search"},
                    "tools": [{"googleSearch": {}}],
                    "temperature": 0.25,
                    "thinkingConfig": {"includeThoughts": true},
                    "unsupportedNativeKey": {"should": "drop"},
                    "openai": {"parallel_tool_calls": false},
                    "anthropic": {"beta_headers": ["leak"]},
                }),
            ),
            ("openai".to_string(), json!({"parallel_tool_calls": false})),
        ]),
        ..Request::default()
    };
    let original_provider_options = request.provider_options.clone();

    let prepared =
        build_gemini_generate_content_request(&request, NativeRequestConfig::default()).unwrap();

    let body = object(&prepared.body);
    assert_eq!(
        body["safetySettings"],
        json!([{
            "category": "HARM_CATEGORY_HATE_SPEECH",
            "threshold": "BLOCK_ONLY_HIGH",
        }])
    );
    assert_eq!(body["cachedContent"], json!("cachedContents/session-123"));
    assert_eq!(body["groundingConfig"], json!({"mode": "google_search"}));
    assert_eq!(
        body["generationConfig"],
        json!({
            "temperature": 0.4,
            "thinkingConfig": {"includeThoughts": true},
        })
    );
    assert_eq!(
        body["tools"],
        json!([
            {
                "functionDeclarations": [{
                    "name": "lookup",
                    "description": "Lookup a fact",
                    "parametersJsonSchema": {
                        "type": "object",
                        "properties": {"query": {"type": "string"}},
                    },
                }],
            },
            {"googleSearch": {}},
        ])
    );
    assert!(!body.contains_key("unsupportedNativeKey"));
    assert!(!body.contains_key("openai"));
    assert!(!body.contains_key("anthropic"));
    assert!(!prepared.body.to_string().contains("parallel_tool_calls"));
    assert!(!prepared.body.to_string().contains("beta_headers"));
    assert_eq!(request.provider_options, original_provider_options);
}

#[test]
fn native_thinking_provider_options_fail_with_provider_specific_errors_when_invalid() {
    let anthropic_request = Request {
        model: "claude-sonnet-4-5".to_string(),
        messages: vec![Message::user("hello")],
        provider_options: BTreeMap::from([(
            "anthropic".to_string(),
            json!({"thinking": "enabled"}),
        )]),
        ..Request::default()
    };

    let error =
        build_anthropic_messages_request(&anthropic_request, NativeRequestConfig::default())
            .unwrap_err();

    assert_eq!(error.kind, AdapterErrorKind::InvalidRequest);
    assert_eq!(error.provider.as_deref(), Some("anthropic"));
    assert!(error.message.contains("provider_options.thinking"));

    let gemini_request = Request {
        model: "gemini-3.1-pro-preview".to_string(),
        messages: vec![Message::user("hello")],
        provider_options: BTreeMap::from([(
            "gemini".to_string(),
            json!({"thinkingConfig": "enabled"}),
        )]),
        ..Request::default()
    };

    let error =
        build_gemini_generate_content_request(&gemini_request, NativeRequestConfig::default())
            .unwrap_err();

    assert_eq!(error.kind, AdapterErrorKind::InvalidRequest);
    assert_eq!(error.provider.as_deref(), Some("gemini"));
    assert!(error.message.contains("provider_options.thinkingConfig"));
}

#[test]
fn gemini_complete_request_preserves_thinking_signature_for_same_provider_continuation() {
    let request = Request {
        model: "gemini-3.1-pro-preview".to_string(),
        messages: vec![Message {
            role: MessageRole::Assistant,
            content: vec![ContentPart::Thinking {
                thinking: ThinkingData {
                    text: "private thought".to_string(),
                    signature: Some("sig-think".to_string()),
                    redacted: false,
                    source_provider: Some("gemini".to_string()),
                    source_model: Some("gemini-3.1-pro-preview".to_string()),
                },
            }],
            ..Message::default()
        }],
        ..Request::default()
    };

    let prepared =
        build_gemini_generate_content_request(&request, NativeRequestConfig::default()).unwrap();

    assert_eq!(
        prepared.body["contents"][0]["parts"],
        json!([{
            "text": "private thought",
            "thought": true,
            "thoughtSignature": "sig-think",
        }])
    );
}

#[test]
fn native_image_translation_normalizes_url_raw_data_and_local_path_sources() {
    let cwd = std::env::current_dir().unwrap();
    let temp_dir = tempfile::tempdir_in(&cwd).unwrap();
    let local_path = temp_dir.path().join("diagram.jpg");
    fs::write(&local_path, b"local-image").unwrap();
    let relative_local_path = relative_dot_path(&cwd, &local_path);
    let raw_encoded = encode_base64(b"raw-image");
    let local_encoded = encode_base64(b"local-image");

    let request = Request {
        model: "native-model".to_string(),
        messages: vec![Message {
            role: MessageRole::User,
            content: vec![
                ContentPart::Text {
                    text: "first".to_string(),
                },
                ContentPart::Image {
                    image: ImageData::url("https://example.test/cat.png"),
                },
                ContentPart::Text {
                    text: "second".to_string(),
                },
                ContentPart::Image {
                    image: ImageData {
                        url: None,
                        data: Some(b"raw-image".to_vec()),
                        media_type: None,
                        detail: None,
                    },
                },
                ContentPart::Text {
                    text: "third".to_string(),
                },
                ContentPart::Image {
                    image: ImageData {
                        url: Some(relative_local_path),
                        data: None,
                        media_type: None,
                        detail: Some("high".to_string()),
                    },
                },
            ],
            ..Message::default()
        }],
        ..Request::default()
    };

    let openai_body = build_openai_responses_request(&request, NativeRequestConfig::default())
        .unwrap()
        .body;
    assert_eq!(
        openai_body["input"][0]["content"],
        json!([
            {"type": "input_text", "text": "first"},
            {"type": "input_image", "image_url": "https://example.test/cat.png"},
            {"type": "input_text", "text": "second"},
            {
                "type": "input_image",
                "image_url": format!("data:image/png;base64,{raw_encoded}"),
            },
            {"type": "input_text", "text": "third"},
            {
                "type": "input_image",
                "image_url": format!("data:image/jpeg;base64,{local_encoded}"),
                "detail": "high",
            },
        ])
    );

    let anthropic_body = build_anthropic_messages_request(&request, NativeRequestConfig::default())
        .unwrap()
        .body;
    assert_eq!(
        anthropic_body["messages"][0]["content"],
        json!([
            {"type": "text", "text": "first"},
            {
                "type": "image",
                "source": {
                    "type": "url",
                    "url": "https://example.test/cat.png",
                },
            },
            {"type": "text", "text": "second"},
            {
                "type": "image",
                "source": {
                    "type": "base64",
                    "media_type": "image/png",
                    "data": raw_encoded,
                },
            },
            {"type": "text", "text": "third"},
            {
                "type": "image",
                "source": {
                    "type": "base64",
                    "media_type": "image/jpeg",
                    "data": local_encoded,
                },
            },
        ])
    );

    let gemini_body =
        build_gemini_generate_content_request(&request, NativeRequestConfig::default())
            .unwrap()
            .body;
    assert_eq!(
        gemini_body["contents"][0]["parts"],
        json!([
            {"text": "first"},
            {
                "fileData": {
                    "fileUri": "https://example.test/cat.png",
                    "mimeType": "image/png",
                },
            },
            {"text": "second"},
            {
                "inlineData": {
                    "data": raw_encoded,
                    "mimeType": "image/png",
                },
            },
            {"text": "third"},
            {
                "inlineData": {
                    "data": local_encoded,
                    "mimeType": "image/jpeg",
                },
            },
        ])
    );
}

#[test]
fn native_image_translation_reads_absolute_local_paths_and_rejects_ambiguous_sources() {
    let temp_dir = tempfile::tempdir().unwrap();
    let local_path = temp_dir.path().join("diagram.webp");
    fs::write(&local_path, b"webp-image").unwrap();
    let local_encoded = encode_base64(b"webp-image");

    let local_request = Request {
        model: "native-model".to_string(),
        messages: vec![Message {
            role: MessageRole::User,
            content: vec![ContentPart::Image {
                image: ImageData {
                    url: Some(local_path.display().to_string()),
                    data: None,
                    media_type: None,
                    detail: None,
                },
            }],
            ..Message::default()
        }],
        ..Request::default()
    };

    let body = build_openai_responses_request(&local_request, NativeRequestConfig::default())
        .unwrap()
        .body;
    assert_eq!(
        body["input"][0]["content"][0]["image_url"],
        json!(format!("data:image/webp;base64,{local_encoded}"))
    );

    let ambiguous_request = Request {
        model: "native-model".to_string(),
        messages: vec![Message {
            role: MessageRole::User,
            content: vec![ContentPart::Image {
                image: ImageData {
                    url: Some("https://example.test/cat.png".to_string()),
                    data: Some(b"raw-image".to_vec()),
                    media_type: None,
                    detail: None,
                },
            }],
            ..Message::default()
        }],
        ..Request::default()
    };
    let error = build_openai_responses_request(&ambiguous_request, NativeRequestConfig::default())
        .unwrap_err();

    assert_eq!(error.kind, AdapterErrorKind::InvalidRequest);
    assert_eq!(error.provider.as_deref(), Some("openai"));
    assert!(error.message.contains("exactly one of url or data"));

    let tilde_request = Request {
        model: "native-model".to_string(),
        messages: vec![Message {
            role: MessageRole::User,
            content: vec![ContentPart::Image {
                image: ImageData {
                    url: Some(format!("~/.spark-missing-image-{}.png", std::process::id())),
                    data: None,
                    media_type: None,
                    detail: None,
                },
            }],
            ..Message::default()
        }],
        ..Request::default()
    };
    let error =
        build_gemini_generate_content_request(&tilde_request, NativeRequestConfig::default())
            .unwrap_err();

    assert_eq!(error.kind, AdapterErrorKind::InvalidRequest);
    assert_eq!(error.provider.as_deref(), Some("gemini"));
    assert!(error.message.contains("unable to read local image input"));
}

#[test]
fn native_providers_reject_unsupported_audio_and_document_content_with_provider_errors() {
    for provider in ["openai", "anthropic", "gemini"] {
        for (kind, part) in [
            (
                "audio",
                ContentPart::Audio {
                    audio: AudioData {
                        url: Some("https://example.test/audio.mp3".to_string()),
                        data: None,
                        media_type: Some("audio/mpeg".to_string()),
                    },
                },
            ),
            (
                "document",
                ContentPart::Document {
                    document: DocumentData {
                        url: Some("https://example.test/doc.pdf".to_string()),
                        data: None,
                        media_type: Some("application/pdf".to_string()),
                        file_name: Some("doc.pdf".to_string()),
                    },
                },
            ),
        ] {
            let request = Request {
                model: "native-model".to_string(),
                messages: vec![Message {
                    role: MessageRole::User,
                    content: vec![
                        ContentPart::Text {
                            text: "kept before unsupported media".to_string(),
                        },
                        part,
                    ],
                    ..Message::default()
                }],
                ..Request::default()
            };

            let error =
                build_native_complete_request(provider, &request, NativeRequestConfig::default())
                    .unwrap_err();
            assert_eq!(error.kind, AdapterErrorKind::InvalidRequest);
            assert_eq!(error.provider.as_deref(), Some(provider));
            assert!(error.message.contains(kind));
        }
    }
}

#[test]
fn openai_response_translation_maps_cached_input_tokens_to_cache_read_usage() {
    let payload = json!({
        "id": "resp_123",
        "model": "gpt-5.2",
        "status": "completed",
        "output": [
            {
                "type": "function_call",
                "id": "call_123",
                "name": "lookup_weather",
                "arguments": {"city": "Paris"},
            },
            {
                "type": "reasoning",
                "text": "private reasoning summary",
            },
            {
                "type": "output_text",
                "text": "Hello",
            },
        ],
        "usage": {
            "input_tokens": 12,
            "output_tokens": 34,
            "output_tokens_details": {"reasoning_tokens": 5},
            "input_tokens_details": {"cached_tokens": 4},
        },
    });

    let response = translate_native_complete_response("openai", payload.clone()).unwrap();

    assert_eq!(response.provider, "openai");
    assert_eq!(response.id, "resp_123");
    assert_eq!(response.model, "gpt-5.2");
    assert_eq!(response.text(), "Hello");
    assert!(!response.text().contains("private reasoning summary"));
    assert_eq!(response.finish_reason.reason, FinishReasonKind::ToolCalls);
    assert_eq!(response.finish_reason.raw.as_deref(), Some("completed"));
    assert_eq!(response.tool_calls().len(), 1);
    assert_eq!(response.tool_calls()[0].name, "lookup_weather");
    assert_eq!(response.usage.input_tokens, 12);
    assert_eq!(response.usage.output_tokens, 34);
    assert_eq!(response.usage.total_tokens, 46);
    assert_eq!(response.usage.reasoning_tokens, Some(5));
    assert_eq!(response.usage.cache_read_tokens, Some(4));
    assert_eq!(response.usage.cache_write_tokens, None);
    assert_eq!(response.usage.raw, Some(payload["usage"].clone()));
    assert_eq!(response.raw, Some(payload));
}

#[test]
fn openai_response_translation_maps_non_stop_finish_reasons_and_preserves_raw_values() {
    for (payload, expected_reason, expected_raw) in [
        (
            json!({
                "id": "resp_length",
                "model": "gpt-5.2",
                "status": "incomplete",
                "incomplete_details": {"reason": "max_output_tokens"},
                "output": [{"type": "output_text", "text": "Partial"}],
            }),
            FinishReasonKind::Length,
            Some("incomplete"),
        ),
        (
            json!({
                "id": "resp_filtered",
                "model": "gpt-5.2",
                "status": "incomplete",
                "incomplete_details": {"reason": "content_filter"},
                "output": [{"type": "output_text", "text": "Filtered"}],
            }),
            FinishReasonKind::ContentFilter,
            Some("incomplete"),
        ),
        (
            json!({
                "id": "resp_failed",
                "model": "gpt-5.2",
                "status": "failed",
                "output": [{"type": "output_text", "text": ""}],
            }),
            FinishReasonKind::Error,
            Some("failed"),
        ),
        (
            json!({
                "id": "resp_unknown",
                "model": "gpt-5.2",
                "finish_reason": "provider_specific_shutdown",
                "output": [{"type": "output_text", "text": "Done"}],
            }),
            FinishReasonKind::Other,
            Some("provider_specific_shutdown"),
        ),
    ] {
        assert_native_finish_reason("openai", payload, expected_reason, expected_raw);
    }
}

#[test]
fn native_response_translation_parses_rate_limit_headers_without_body_metadata() {
    let payload = json!({
        "id": "resp_rate",
        "model": "gpt-5.2",
        "status": "completed",
        "output": [{"type": "output_text", "text": "Hello"}],
    });
    let headers = BTreeMap::from([
        (
            "X-RateLimit-Remaining-Requests".to_string(),
            "7".to_string(),
        ),
        ("x-ratelimit-limit-requests".to_string(), "10".to_string()),
        ("x-ratelimit-remaining-tokens".to_string(), "99".to_string()),
        ("x-ratelimit-limit-tokens".to_string(), "123".to_string()),
        (
            "x-ratelimit-reset".to_string(),
            "2026-06-27T18:00:00Z".to_string(),
        ),
    ]);

    let response =
        translate_native_complete_response_with_headers("openai", payload, &headers).unwrap();

    let rate_limit = response
        .rate_limit
        .expect("rate-limit headers should parse");
    assert_eq!(rate_limit.requests_remaining, Some(7));
    assert_eq!(rate_limit.requests_limit, Some(10));
    assert_eq!(rate_limit.tokens_remaining, Some(99));
    assert_eq!(rate_limit.tokens_limit, Some(123));
    assert_eq!(rate_limit.reset_at.as_deref(), Some("2026-06-27T18:00:00Z"));

    let anthropic_payload = json!({
        "id": "msg_rate",
        "model": "claude-sonnet-4-5",
        "content": [{"type": "text", "text": "Hello"}],
        "stop_reason": "end_turn",
    });
    let anthropic_headers = BTreeMap::from([
        (
            "anthropic-ratelimit-requests-remaining".to_string(),
            "3".to_string(),
        ),
        (
            "anthropic-ratelimit-tokens-remaining".to_string(),
            "88".to_string(),
        ),
        (
            "anthropic-ratelimit-tokens-reset".to_string(),
            "2026-06-27T18:05:00Z".to_string(),
        ),
    ]);

    let response = translate_native_complete_response_with_headers(
        "anthropic",
        anthropic_payload,
        &anthropic_headers,
    )
    .unwrap();

    let rate_limit = response
        .rate_limit
        .expect("anthropic rate-limit headers should parse");
    assert_eq!(rate_limit.requests_remaining, Some(3));
    assert_eq!(rate_limit.tokens_remaining, Some(88));
    assert_eq!(rate_limit.reset_at.as_deref(), Some("2026-06-27T18:05:00Z"));
}

#[test]
fn anthropic_response_translation_maps_cache_read_and_creation_usage() {
    let payload = json!({
        "id": "msg_123",
        "type": "message",
        "role": "assistant",
        "model": "claude-sonnet-4-5",
        "content": [
            {
                "type": "text",
                "text": "Hello",
            }
        ],
        "stop_reason": "end_turn",
        "usage": {
            "input_tokens": 11,
            "output_tokens": 13,
            "cache_read_input_tokens": 5,
            "cache_creation_input_tokens": 2,
        },
    });

    let response = translate_native_complete_response("anthropic", payload.clone()).unwrap();

    assert_eq!(response.provider, "anthropic");
    assert_eq!(response.id, "msg_123");
    assert_eq!(response.model, "claude-sonnet-4-5");
    assert_eq!(response.text(), "Hello");
    assert_eq!(response.finish_reason.reason, FinishReasonKind::Stop);
    assert_eq!(response.finish_reason.raw.as_deref(), Some("end_turn"));
    assert_eq!(response.usage.input_tokens, 11);
    assert_eq!(response.usage.output_tokens, 13);
    assert_eq!(response.usage.total_tokens, 24);
    assert_eq!(response.usage.reasoning_tokens, None);
    assert_eq!(response.usage.cache_read_tokens, Some(5));
    assert_eq!(response.usage.cache_write_tokens, Some(2));
    assert_eq!(response.usage.raw, Some(payload["usage"].clone()));
    assert_eq!(response.raw, Some(payload));
}

#[test]
fn anthropic_response_translation_maps_non_stop_finish_reasons_and_preserves_raw_values() {
    for (stop_reason, expected_reason) in [
        ("max_tokens", FinishReasonKind::Length),
        ("content_filter", FinishReasonKind::ContentFilter),
        ("error", FinishReasonKind::Error),
        ("pause_turn", FinishReasonKind::Other),
    ] {
        assert_native_finish_reason(
            "anthropic",
            json!({
                "id": format!("msg_{stop_reason}"),
                "type": "message",
                "role": "assistant",
                "model": "claude-sonnet-4-5",
                "content": [{"type": "text", "text": "Hello"}],
                "stop_reason": stop_reason,
            }),
            expected_reason,
            Some(stop_reason),
        );
    }
}

#[test]
fn anthropic_response_translation_preserves_thinking_blocks_and_estimates_reasoning_usage() {
    let payload = json!({
        "id": "msg_thinking",
        "type": "message",
        "role": "assistant",
        "model": "claude-sonnet-4-5",
        "content": [
            {
                "type": "thinking",
                "thinking": "reasoning ",
                "signature": "sig-123",
            },
            {
                "type": "redacted_thinking",
                "data": "  opaque-123  ",
            },
            {
                "type": "text",
                "text": "Done",
            },
        ],
        "stop_reason": "end_turn",
        "usage": {
            "input_tokens": 2,
            "output_tokens": 3,
            "cache_read_input_tokens": 5,
            "cache_creation_input_tokens": 7,
        },
    });

    let response = translate_native_complete_response("anthropic", payload.clone()).unwrap();

    assert_eq!(response.text(), "Done");
    assert_eq!(
        response.reasoning().as_deref(),
        Some("reasoning   opaque-123  ")
    );
    assert_eq!(response.usage.input_tokens, 2);
    assert_eq!(response.usage.output_tokens, 3);
    assert_eq!(response.usage.total_tokens, 5);
    assert_eq!(response.usage.reasoning_tokens, Some(7));
    assert_eq!(response.usage.cache_read_tokens, Some(5));
    assert_eq!(response.usage.cache_write_tokens, Some(7));
    match &response.message.content[0] {
        ContentPart::Thinking { thinking } => {
            assert_eq!(thinking.text, "reasoning ");
            assert_eq!(thinking.signature.as_deref(), Some("sig-123"));
            assert!(!thinking.redacted);
            assert_eq!(thinking.source_provider.as_deref(), Some("anthropic"));
            assert_eq!(thinking.source_model.as_deref(), Some("claude-sonnet-4-5"));
        }
        other => panic!("expected thinking content part, got {other:?}"),
    }
    match &response.message.content[1] {
        ContentPart::RedactedThinking { thinking } => {
            assert_eq!(thinking.text, "  opaque-123  ");
            assert!(thinking.redacted);
            assert_eq!(thinking.source_provider.as_deref(), Some("anthropic"));
            assert_eq!(thinking.source_model.as_deref(), Some("claude-sonnet-4-5"));
        }
        other => panic!("expected redacted thinking content part, got {other:?}"),
    }
    assert_eq!(response.raw, Some(payload));

    let payload_with_exact_count = json!({
        "content": [{"type": "thinking", "thinking": "reasoning"}],
        "usage": {
            "input_tokens": 2,
            "output_tokens": 3,
            "reasoning_tokens": 42,
        },
    });
    let response =
        translate_native_complete_response("anthropic", payload_with_exact_count).unwrap();
    assert_eq!(response.usage.reasoning_tokens, Some(42));
}

#[test]
fn gemini_response_translation_maps_cached_content_tokens_to_cache_read_usage() {
    let payload = json!({
        "responseId": "resp_123",
        "modelVersion": "gemini-3.1-pro-preview",
        "candidates": [
            {
                "content": {
                    "role": "model",
                    "parts": [
                        {"text": "private thought", "thought": true},
                        {"text": "Hello"},
                    ],
                },
                "finishReason": "STOP",
            }
        ],
        "usageMetadata": {
            "promptTokenCount": 12,
            "candidatesTokenCount": 34,
            "totalTokenCount": 46,
            "thoughtsTokenCount": 5,
            "cachedContentTokenCount": 4,
        },
    });

    let response = translate_native_complete_response("gemini", payload.clone()).unwrap();

    assert_eq!(response.provider, "gemini");
    assert_eq!(response.id, "resp_123");
    assert_eq!(response.model, "gemini-3.1-pro-preview");
    assert_eq!(response.text(), "Hello");
    assert_eq!(response.reasoning().as_deref(), Some("private thought"));
    assert_eq!(response.finish_reason.reason, FinishReasonKind::Stop);
    assert_eq!(response.finish_reason.raw.as_deref(), Some("STOP"));
    assert_eq!(response.usage.input_tokens, 12);
    assert_eq!(response.usage.output_tokens, 34);
    assert_eq!(response.usage.total_tokens, 46);
    assert_eq!(response.usage.reasoning_tokens, Some(5));
    assert_eq!(response.usage.cache_read_tokens, Some(4));
    assert_eq!(response.usage.cache_write_tokens, None);
    match &response.message.content[0] {
        ContentPart::Thinking { thinking } => {
            assert_eq!(thinking.source_provider.as_deref(), Some("gemini"));
            assert_eq!(
                thinking.source_model.as_deref(),
                Some("gemini-3.1-pro-preview")
            );
        }
        other => panic!("expected thinking content part, got {other:?}"),
    }
    assert_eq!(response.usage.raw, Some(payload["usageMetadata"].clone()));
    assert_eq!(response.raw, Some(payload));
}

#[test]
fn gemini_response_translation_maps_non_stop_finish_reasons_and_preserves_raw_values() {
    for (finish_reason, expected_reason) in [
        ("MAX_TOKENS", FinishReasonKind::Length),
        ("SAFETY", FinishReasonKind::ContentFilter),
        ("MALFORMED_FUNCTION_CALL", FinishReasonKind::Error),
        ("FINISH_REASON_UNSPECIFIED", FinishReasonKind::Other),
    ] {
        assert_native_finish_reason(
            "gemini",
            json!({
                "responseId": format!("resp_{finish_reason}"),
                "modelVersion": "gemini-3.1-pro-preview",
                "candidates": [{
                    "content": {
                        "role": "model",
                        "parts": [{"text": "Hello"}],
                    },
                    "finishReason": finish_reason,
                }],
            }),
            expected_reason,
            Some(finish_reason),
        );
    }
}

#[test]
fn gemini_response_translation_uses_stable_stateless_synthetic_tool_ids() {
    let payload = json!({
        "responseId": "resp_tools",
        "modelVersion": "gemini-3.1-pro-preview",
        "candidates": [
            {
                "content": {
                    "role": "model",
                    "parts": [
                        {"text": "private thought", "thought": true},
                        {
                            "functionCall": {
                                "name": "lookup_weather",
                                "args": {"city": "Paris"},
                            }
                        },
                        {"text": "Final answer"},
                    ],
                },
                "finishReason": "STOP",
            }
        ],
        "usageMetadata": {
            "promptTokenCount": 12,
            "candidatesTokenCount": 34,
            "totalTokenCount": 46,
        },
    });

    let first = translate_native_complete_response("gemini", payload.clone()).unwrap();
    let second = translate_native_complete_response("gemini", payload.clone()).unwrap();

    assert_eq!(first.finish_reason.reason, FinishReasonKind::ToolCalls);
    assert_eq!(first.finish_reason.raw.as_deref(), Some("STOP"));
    assert_eq!(first.text(), "Final answer");
    assert_eq!(first.reasoning().as_deref(), Some("private thought"));
    let first_call = &first.tool_calls()[0];
    let second_call = &second.tool_calls()[0];
    assert_eq!(first_call.id, second_call.id);
    assert!(first_call
        .id
        .starts_with("gemini_call_lookup_weather_c0_p1_"));
    assert_eq!(first_call.name, "lookup_weather");
    assert_eq!(first_call.arguments, json!({"city": "Paris"}));

    let changed_arguments = json!({
        "responseId": "resp_tools",
        "modelVersion": "gemini-3.1-pro-preview",
        "candidates": [{
            "content": {
                "parts": [{
                    "functionCall": {
                        "name": "lookup_weather",
                        "args": {"city": "Berlin"},
                    }
                }],
            },
            "finishReason": "STOP",
        }],
    });
    let changed = translate_native_complete_response("gemini", changed_arguments).unwrap();
    assert_ne!(first_call.id, changed.tool_calls()[0].id);
}

#[test]
fn gemini_tool_result_translation_recovers_function_name_after_serialized_replay() {
    let assistant_response = translate_native_complete_response(
        "gemini",
        json!({
            "responseId": "resp_tool",
            "modelVersion": "gemini-3.1-pro-preview",
            "candidates": [{
                "content": {
                    "role": "model",
                    "parts": [{
                        "functionCall": {
                            "name": "lookup_weather",
                            "args": {"city": "Paris"},
                        }
                    }],
                },
                "finishReason": "STOP",
            }],
        }),
    )
    .unwrap();
    let tool_call_id = assistant_response.tool_calls()[0].id.clone();
    let replayed_assistant_message: Message =
        serde_json::from_value(serde_json::to_value(assistant_response.message).unwrap()).unwrap();
    let request = Request {
        model: "gemini-3.1-pro-preview".to_string(),
        messages: vec![
            Message::user("hello"),
            replayed_assistant_message,
            Message::tool_result(tool_call_id.clone(), json!("72F and sunny"), false),
        ],
        ..Request::default()
    };

    let prepared =
        build_gemini_generate_content_request(&request, NativeRequestConfig::default()).unwrap();

    assert_eq!(
        prepared.body["contents"][2]["parts"][0]["functionResponse"],
        json!({
            "id": tool_call_id,
            "name": "lookup_weather",
            "response": {"result": "72F and sunny"},
        })
    );
}

fn tool(name: &str) -> Value {
    json!({
        "name": name,
        "description": "Lookup a fact",
        "parameters": {
            "type": "object",
            "properties": {"query": {"type": "string"}},
        },
    })
}

fn tool_call(id: &str, name: &str, arguments: Value) -> ToolCall {
    ToolCall {
        id: id.to_string(),
        name: name.to_string(),
        arguments,
        raw_arguments: None,
        r#type: "function".to_string(),
    }
}

fn object(value: &Value) -> &serde_json::Map<String, Value> {
    value.as_object().expect("prepared JSON object body")
}

fn assert_native_finish_reason(
    provider: &str,
    payload: Value,
    expected_reason: FinishReasonKind,
    expected_raw: Option<&str>,
) {
    let raw_payload = payload.clone();
    let response = translate_native_complete_response(provider, payload).unwrap();
    assert_eq!(response.finish_reason.reason, expected_reason);
    assert_eq!(response.finish_reason.raw.as_deref(), expected_raw);
    assert_eq!(response.raw, Some(raw_payload));
}

fn count_key(value: &Value, key: &str) -> usize {
    match value {
        Value::Object(object) => {
            usize::from(object.contains_key(key))
                + object
                    .values()
                    .map(|value| count_key(value, key))
                    .sum::<usize>()
        }
        Value::Array(values) => values.iter().map(|value| count_key(value, key)).sum(),
        _ => 0,
    }
}

fn relative_dot_path(cwd: &Path, path: &Path) -> String {
    format!(
        "./{}",
        path.strip_prefix(cwd)
            .expect("test temp file should be under the current directory")
            .display()
    )
}

fn encode_base64(data: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(data)
}
