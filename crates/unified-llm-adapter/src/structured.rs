use serde_json::{json, Value};

use crate::client::Client;
use crate::errors::{AdapterError, AdapterErrorKind};
use crate::generation::{generate_with_policy, generate_with_policy_and_hooks, GenerateResult};
use crate::request::{Request, Response, ResponseFormat};
use crate::retry::RetryPolicy;

#[derive(Debug, Clone, PartialEq)]
pub struct GenerateObjectResult {
    pub value: Value,
    pub raw_text: String,
    pub response: Response,
    pub generation: GenerateResult,
}

pub fn generate_object(
    client: &Client,
    request: Request,
    schema: Value,
) -> Result<GenerateObjectResult, AdapterError> {
    generate_object_with_policy(client, request, schema, &RetryPolicy::default())
}

pub fn generate_object_with_policy(
    client: &Client,
    request: Request,
    schema: Value,
    policy: &RetryPolicy,
) -> Result<GenerateObjectResult, AdapterError> {
    validate_schema_shape(&schema)?;
    let request = request_with_schema_response_format(request, &schema);
    let generation = generate_with_policy(client, request, policy)?;
    finish_structured_generation(generation, schema)
}

pub fn generate_object_with_policy_and_hooks<R, S>(
    client: &Client,
    request: Request,
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
    let request = request_with_schema_response_format(request, &schema);
    let generation =
        generate_with_policy_and_hooks(client, request, policy, random_multiplier, sleeper)?;
    finish_structured_generation(generation, schema)
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

fn request_with_schema_response_format(mut request: Request, schema: &Value) -> Request {
    request.response_format = Some(ResponseFormat::JsonSchema {
        json_schema: schema.clone(),
        strict: true,
    });
    request
}

fn finish_structured_generation(
    generation: GenerateResult,
    schema: Value,
) -> Result<GenerateObjectResult, AdapterError> {
    let raw_text = generation.response.text();
    let value = parse_structured_output(&raw_text, &schema, Some(&generation.response))?;
    Ok(GenerateObjectResult {
        value,
        raw_text,
        response: generation.response.clone(),
        generation,
    })
}

fn validate_schema_shape(schema: &Value) -> Result<(), AdapterError> {
    let Some(object) = schema.as_object() else {
        return Err(invalid_request_error("schema must be a JSON object"));
    };

    if let Some(schema_type) = object.get("type") {
        validate_type_spec(schema_type)?;
    }

    if let Some(required) = object.get("required") {
        let Some(required) = required.as_array() else {
            return Err(invalid_request_error("schema required must be an array"));
        };
        if required.iter().any(|field| !field.is_string()) {
            return Err(invalid_request_error(
                "schema required entries must be strings",
            ));
        }
    }

    if let Some(properties) = object.get("properties") {
        let Some(properties) = properties.as_object() else {
            return Err(invalid_request_error("schema properties must be an object"));
        };
        for property_schema in properties.values() {
            validate_schema_shape(property_schema)?;
        }
    }

    if let Some(items) = object.get("items") {
        validate_schema_shape(items)?;
    }

    Ok(())
}

fn validate_type_spec(schema_type: &Value) -> Result<(), AdapterError> {
    match schema_type {
        Value::String(value) => {
            if is_supported_type(value) {
                Ok(())
            } else {
                Err(invalid_request_error(format!(
                    "unsupported schema type {value:?}"
                )))
            }
        }
        Value::Array(values) => {
            if values.is_empty() {
                return Err(invalid_request_error("schema type array must not be empty"));
            }
            for value in values {
                let Some(schema_type) = value.as_str() else {
                    return Err(invalid_request_error(
                        "schema type array entries must be strings",
                    ));
                };
                if !is_supported_type(schema_type) {
                    return Err(invalid_request_error(format!(
                        "unsupported schema type {schema_type:?}"
                    )));
                }
            }
            Ok(())
        }
        _ => Err(invalid_request_error(
            "schema type must be a string or array of strings",
        )),
    }
}

fn validate_json_value(value: &Value, schema: &Value, path: &str) -> Result<(), String> {
    let Some(schema_object) = schema.as_object() else {
        return Err(format!("{path} schema must be an object"));
    };

    if let Some(schema_type) = schema_object.get("type") {
        validate_value_type(value, schema_type, path)?;
    }

    if let Some(enum_values) = schema_object.get("enum") {
        let Some(enum_values) = enum_values.as_array() else {
            return Err(format!("{path} schema enum must be an array"));
        };
        if !enum_values.iter().any(|candidate| candidate == value) {
            return Err(format!("{path} value is not one of the schema enum values"));
        }
    }

    if schema_object.contains_key("required")
        || schema_object.contains_key("properties")
        || schema_object.contains_key("additionalProperties")
    {
        validate_object_value(value, schema_object, path)?;
    }

    if let Some(items_schema) = schema_object.get("items") {
        let Some(values) = value.as_array() else {
            return Err(format!("{path} must be an array"));
        };
        for (index, item) in values.iter().enumerate() {
            validate_json_value(item, items_schema, &format!("{path}[{index}]"))?;
        }
    }

    Ok(())
}

fn validate_value_type(value: &Value, schema_type: &Value, path: &str) -> Result<(), String> {
    let matches = match schema_type {
        Value::String(schema_type) => value_matches_type(value, schema_type),
        Value::Array(schema_types) => schema_types
            .iter()
            .filter_map(Value::as_str)
            .any(|schema_type| value_matches_type(value, schema_type)),
        _ => false,
    };
    if matches {
        Ok(())
    } else {
        Err(format!(
            "{path} expected {}, got {}",
            describe_type_spec(schema_type),
            json_type_name(value)
        ))
    }
}

fn validate_object_value(
    value: &Value,
    schema_object: &serde_json::Map<String, Value>,
    path: &str,
) -> Result<(), String> {
    let Some(value_object) = value.as_object() else {
        return Err(format!("{path} must be an object"));
    };

    if let Some(required) = schema_object.get("required").and_then(Value::as_array) {
        for field in required.iter().filter_map(Value::as_str) {
            if !value_object.contains_key(field) {
                return Err(format!("{path}.{field} is required"));
            }
        }
    }

    let properties = schema_object.get("properties").and_then(Value::as_object);
    if let Some(properties) = properties {
        for (field, property_schema) in properties {
            if let Some(field_value) = value_object.get(field) {
                validate_json_value(field_value, property_schema, &format!("{path}.{field}"))?;
            }
        }
    }

    if schema_object
        .get("additionalProperties")
        .and_then(Value::as_bool)
        == Some(false)
    {
        for field in value_object.keys() {
            let is_known_property = properties
                .map(|properties| properties.contains_key(field))
                .unwrap_or(false);
            if !is_known_property {
                return Err(format!("{path}.{field} is not allowed by the schema"));
            }
        }
    }

    Ok(())
}

fn value_matches_type(value: &Value, schema_type: &str) -> bool {
    match schema_type {
        "object" => value.is_object(),
        "array" => value.is_array(),
        "string" => value.is_string(),
        "number" => value.is_number(),
        "integer" => value.as_i64().is_some() || value.as_u64().is_some(),
        "boolean" => value.is_boolean(),
        "null" => value.is_null(),
        _ => false,
    }
}

fn json_type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(number) if number.is_i64() || number.is_u64() => "integer",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

fn describe_type_spec(schema_type: &Value) -> String {
    match schema_type {
        Value::String(value) => value.clone(),
        Value::Array(values) => values
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>()
            .join(" or "),
        _ => "valid JSON type".to_string(),
    }
}

fn is_supported_type(schema_type: &str) -> bool {
    matches!(
        schema_type,
        "object" | "array" | "string" | "number" | "integer" | "boolean" | "null"
    )
}

fn invalid_request_error(message: impl Into<String>) -> AdapterError {
    AdapterError::new(AdapterErrorKind::InvalidRequest, message)
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
