use serde_json::{json, Value};

use crate::client::Client;
use crate::errors::{AdapterError, AdapterErrorKind};
use crate::generation::{
    generate_with_policy, generate_with_policy_and_hooks, stream_with_policy,
    stream_with_policy_and_hooks, GenerateRequest, GenerateResult, StreamResult,
};
use crate::request::{Response, ResponseFormat, ToolCall};
use crate::retry::RetryPolicy;

pub(crate) const STRUCTURED_OUTPUT_TOOL_NAME: &str = "structured_output";

#[derive(Debug, Clone, PartialEq)]
pub struct GenerateObjectResult {
    pub value: Value,
    pub raw_text: String,
    pub response: Response,
    pub generation: GenerateResult,
}

pub struct StreamObjectResult {
    stream: StreamResult,
    schema: Value,
    final_value: Option<Value>,
    last_partial_value: Option<Value>,
    final_response: Option<Response>,
    terminal_error: Option<AdapterError>,
    finished: bool,
}

pub fn generate_object(
    client: &Client,
    input: impl Into<GenerateRequest>,
    schema: Value,
) -> Result<GenerateObjectResult, AdapterError> {
    generate_object_with_policy(client, input, schema, &RetryPolicy::default())
}

pub fn generate_object_with_policy(
    client: &Client,
    input: impl Into<GenerateRequest>,
    schema: Value,
    policy: &RetryPolicy,
) -> Result<GenerateObjectResult, AdapterError> {
    validate_schema_shape(&schema)?;
    let request = request_with_schema_response_format(input, &schema);
    let generation = generate_with_policy(client, request, policy)?;
    finish_structured_generation(generation, schema)
}

pub fn generate_object_with_policy_and_hooks<R, S>(
    client: &Client,
    input: impl Into<GenerateRequest>,
    schema: Value,
    policy: &RetryPolicy,
    random_multiplier: R,
    sleeper: S,
) -> Result<GenerateObjectResult, AdapterError>
where
    R: FnMut() -> f64,
    S: FnMut(f64),
{
    validate_schema_shape(&schema)?;
    let request = request_with_schema_response_format(input, &schema);
    let generation =
        generate_with_policy_and_hooks(client, request, policy, random_multiplier, sleeper)?;
    finish_structured_generation(generation, schema)
}

pub fn stream_object(
    client: &Client,
    input: impl Into<GenerateRequest>,
    schema: Value,
) -> Result<StreamObjectResult, AdapterError> {
    stream_object_with_policy(client, input, schema, &RetryPolicy::default())
}

pub fn stream_object_with_policy(
    client: &Client,
    input: impl Into<GenerateRequest>,
    schema: Value,
    policy: &RetryPolicy,
) -> Result<StreamObjectResult, AdapterError> {
    validate_schema_shape(&schema)?;
    let request = request_with_schema_response_format(input, &schema);
    let stream = stream_with_policy(client, request, policy)?;
    Ok(StreamObjectResult::new(stream, schema))
}

pub fn stream_object_with_policy_and_hooks<R, S>(
    client: &Client,
    input: impl Into<GenerateRequest>,
    schema: Value,
    policy: &RetryPolicy,
    random_multiplier: R,
    sleeper: S,
) -> Result<StreamObjectResult, AdapterError>
where
    R: FnMut() -> f64 + Send + 'static,
    S: FnMut(f64) + Send + 'static,
{
    validate_schema_shape(&schema)?;
    let request = request_with_schema_response_format(input, &schema);
    let stream = stream_with_policy_and_hooks(client, request, policy, random_multiplier, sleeper)?;
    Ok(StreamObjectResult::new(stream, schema))
}

pub fn parse_structured_output(
    text: &str,
    schema: &Value,
    response: Option<&Response>,
) -> Result<Value, AdapterError> {
    validate_schema_shape(schema)?;
    let value = serde_json::from_str::<Value>(text).map_err(|error| {
        no_object_generated_error(
            "failed to parse structured output as JSON",
            text,
            response,
            schema,
            None,
            Some(error.to_string()),
        )
    })?;
    validate_json_value(&value, schema, "$").map_err(|message| {
        no_object_generated_error(
            "structured output did not match the provided JSON Schema",
            text,
            response,
            schema,
            Some(&value),
            Some(message),
        )
    })?;
    Ok(value)
}

impl StreamObjectResult {
    fn new(stream: StreamResult, schema: Value) -> Self {
        Self {
            stream,
            schema,
            final_value: None,
            last_partial_value: None,
            final_response: None,
            terminal_error: None,
            finished: false,
        }
    }

    pub fn partial_response(&self) -> Response {
        self.stream.partial_response()
    }

    pub fn partial_object(&self) -> Option<Value> {
        self.final_value
            .clone()
            .or_else(|| self.last_partial_value.clone())
    }

    pub fn response(&mut self) -> Result<Response, AdapterError> {
        if let Some(error) = self.terminal_error.clone() {
            return Err(error);
        }
        if let Some(response) = self.final_response.clone() {
            return Ok(response);
        }

        let response = self.stream.response()?;
        match parse_structured_response(&response, &self.schema) {
            Ok((value, _raw_text)) => {
                self.final_value = Some(value);
                self.final_response = Some(response.clone());
                self.finished = true;
                Ok(response)
            }
            Err(error) => {
                self.terminal_error = Some(error.clone());
                self.finished = true;
                Err(error)
            }
        }
    }

    pub fn object(&mut self) -> Result<Value, AdapterError> {
        if let Some(error) = self.terminal_error.clone() {
            return Err(error);
        }
        if let Some(value) = self.final_value.clone() {
            return Ok(value);
        }

        self.response()?;
        self.final_value.clone().ok_or_else(|| {
            AdapterError::new(
                AdapterErrorKind::NoObjectGenerated,
                "no structured object was generated",
            )
        })
    }

    pub fn close(&mut self) -> Result<(), AdapterError> {
        self.finished = true;
        match self.stream.close() {
            Ok(()) => Ok(()),
            Err(error) => {
                self.terminal_error = Some(error.clone());
                Err(error)
            }
        }
    }
}

impl Iterator for StreamObjectResult {
    type Item = Result<Value, AdapterError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.finished {
            return None;
        }

        loop {
            match self.stream.next() {
                Some(Ok(_event)) => {
                    let partial_response = self.stream.partial_response();
                    let source = structured_output_tool_raw_text(&partial_response)
                        .unwrap_or_else(|| partial_response.text());
                    let Some(value) = parse_partial_json_value(&source) else {
                        continue;
                    };
                    if self.last_partial_value.as_ref() == Some(&value) {
                        continue;
                    }
                    self.last_partial_value = Some(value.clone());
                    return Some(Ok(value));
                }
                Some(Err(error)) => {
                    self.terminal_error = Some(error.clone());
                    self.finished = true;
                    return Some(Err(error));
                }
                None => match self.response() {
                    Ok(_) => {
                        let value = self.final_value.clone()?;
                        self.finished = true;
                        if self.last_partial_value.as_ref() == Some(&value) {
                            return None;
                        }
                        self.last_partial_value = Some(value.clone());
                        return Some(Ok(value));
                    }
                    Err(error) => return Some(Err(error)),
                },
            }
        }
    }
}

fn request_with_schema_response_format<I>(input: I, schema: &Value) -> GenerateRequest
where
    I: Into<GenerateRequest>,
{
    let mut request = input.into();
    request.response_format = Some(ResponseFormat::JsonSchema {
        json_schema: schema.clone(),
        strict: true,
    });
    request
}

fn finish_structured_generation(
    mut generation: GenerateResult,
    schema: Value,
) -> Result<GenerateObjectResult, AdapterError> {
    let (value, raw_text) = parse_structured_response(&generation.response, &schema)?;
    generation.output = Some(value.clone());
    Ok(GenerateObjectResult {
        value,
        raw_text,
        response: generation.response.clone(),
        generation,
    })
}

fn validate_schema_shape(schema: &Value) -> Result<(), AdapterError> {
    if !schema.is_object() {
        return Err(invalid_request_error("schema must be a JSON object"));
    }

    jsonschema::meta::validate(schema).map_err(|error| {
        invalid_request_error(format!("schema must be a valid JSON Schema: {error}"))
    })
}

pub(crate) fn validate_json_value(value: &Value, schema: &Value, path: &str) -> Result<(), String> {
    validate_schema_shape(schema)
        .map_err(|error| format!("{path} schema is invalid: {}", error.message))?;
    let validator = jsonschema::validator_for(schema)
        .map_err(|error| format!("{path} schema is invalid: {error}"))?;
    validator
        .validate(value)
        .map_err(|error| format!("{path} {error}"))
}

fn invalid_request_error(message: impl Into<String>) -> AdapterError {
    AdapterError::new(AdapterErrorKind::InvalidRequest, message)
}

fn parse_structured_response(
    response: &Response,
    schema: &Value,
) -> Result<(Value, String), AdapterError> {
    if let Some((value, raw_text)) = structured_output_tool_value(response, schema)? {
        return Ok((value, raw_text));
    }

    let raw_text = response.text();
    let value = parse_structured_output(&raw_text, schema, Some(response))?;
    Ok((value, raw_text))
}

fn structured_output_tool_value(
    response: &Response,
    schema: &Value,
) -> Result<Option<(Value, String)>, AdapterError> {
    let Some(raw_text) = structured_output_tool_raw_text(response) else {
        return Ok(None);
    };
    let value = parse_structured_output(&raw_text, schema, Some(response))?;
    Ok(Some((value, raw_text)))
}

fn structured_output_tool_raw_text(response: &Response) -> Option<String> {
    response
        .tool_calls()
        .into_iter()
        .find(|tool_call| tool_call.name == STRUCTURED_OUTPUT_TOOL_NAME)
        .map(|tool_call| structured_output_tool_call_raw_text(&tool_call))
}

fn structured_output_tool_call_raw_text(tool_call: &ToolCall) -> String {
    if let Some(raw_arguments) = tool_call.raw_arguments.clone() {
        return raw_arguments;
    }
    if let Value::String(raw_arguments) = &tool_call.arguments {
        return raw_arguments.clone();
    }
    json_compact(&tool_call.arguments)
}

fn parse_partial_json_value(text: &str) -> Option<Value> {
    let text = text.trim_start();
    if text.is_empty() {
        return None;
    }
    if let Ok(value) = serde_json::from_str::<Value>(text) {
        return Some(value);
    }
    if !matches!(text.chars().next(), Some('{') | Some('[')) {
        return None;
    }

    let mut cuts = text
        .char_indices()
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    cuts.push(text.len());
    for end in cuts.into_iter().rev() {
        if end == 0 {
            continue;
        }
        let Some(candidate) = repair_json_prefix(&text[..end]) else {
            continue;
        };
        if let Ok(value) = serde_json::from_str::<Value>(&candidate) {
            return Some(value);
        }
    }
    None
}

fn repair_json_prefix(prefix: &str) -> Option<String> {
    let mut candidate = prefix.trim_end().to_string();
    if candidate.is_empty() {
        return None;
    }

    while candidate.ends_with(',') || candidate.ends_with(':') {
        candidate.pop();
        candidate = candidate.trim_end().to_string();
    }

    let mut stack = Vec::new();
    let mut in_string = false;
    let mut escaped = false;
    for ch in candidate.chars() {
        if in_string {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }

        match ch {
            '"' => in_string = true,
            '{' | '[' => stack.push(ch),
            '}' => {
                if stack.pop() != Some('{') {
                    return None;
                }
            }
            ']' => {
                if stack.pop() != Some('[') {
                    return None;
                }
            }
            _ => {}
        }
    }

    if in_string {
        if escaped && candidate.ends_with('\\') {
            candidate.pop();
        }
        candidate.push('"');
    }

    for opener in stack.into_iter().rev() {
        match opener {
            '{' => candidate.push('}'),
            '[' => candidate.push(']'),
            _ => {}
        }
    }

    Some(candidate)
}

fn json_compact(value: &Value) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "null".to_string())
}

fn no_object_generated_error(
    message: impl Into<String>,
    raw_text: &str,
    response: Option<&Response>,
    schema: &Value,
    parsed: Option<&Value>,
    cause: Option<String>,
) -> AdapterError {
    let mut error = AdapterError::new(AdapterErrorKind::NoObjectGenerated, message);
    error.raw = Some(json!({
        "raw_text": raw_text,
        "response": response,
        "schema": schema,
        "parsed": parsed,
        "cause": cause,
    }));
    error
}
