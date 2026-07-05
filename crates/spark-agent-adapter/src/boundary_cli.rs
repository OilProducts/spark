use std::io::{self, Read, Write};
use std::path::Path;

use serde_json::{json, Value};
use unified_llm_adapter::{AdapterError, Client};

use crate::{
    AgentTurnBackend, AgentTurnRequest, CodergenBackend, CodergenBackendRequest,
    RustLlmAgentTurnBackend, RustLlmCodergenBackend,
};

pub fn run() -> i32 {
    match run_inner() {
        Ok(output) => {
            print_json(&output);
            0
        }
        Err(error) => {
            print_json(&json!({ "error": error }));
            0
        }
    }
}

fn run_inner() -> Result<Value, Value> {
    let operation = std::env::args().nth(1).ok_or_else(|| {
        error_payload(
            "missing_operation",
            "missing Rust boundary operation",
            false,
        )
    })?;
    let mut stdin = String::new();
    io::stdin()
        .read_to_string(&mut stdin)
        .map_err(|error| error_payload("stdin_read_failed", error.to_string(), false))?;
    let payload: Value = serde_json::from_str(if stdin.trim().is_empty() {
        "{}"
    } else {
        &stdin
    })
    .map_err(|error| error_payload("invalid_json", error.to_string(), false))?;

    match operation.as_str() {
        "agent-turn" => run_agent_turn(payload),
        "codergen" => run_codergen(payload),
        "codergen-steer" => steer_codergen_turn(payload),
        other => Err(error_payload(
            "unsupported_operation",
            format!("unsupported Rust boundary operation: {other}"),
            false,
        )),
    }
}

fn run_agent_turn(payload: Value) -> Result<Value, Value> {
    let request: AgentTurnRequest = serde_json::from_value(payload.clone())
        .map_err(|error| error_payload("invalid_agent_turn_request", error.to_string(), false))?;
    let client = client_for_payload(&payload).map_err(adapter_error_payload)?;
    let backend = RustLlmAgentTurnBackend::new(client);
    let output = backend
        .run_turn(request)
        .map_err(|error| error_payload("agent_turn_failed", error.message, error.retryable))?;
    Ok(json!({ "output": output }))
}

fn run_codergen(payload: Value) -> Result<Value, Value> {
    let request: CodergenBackendRequest = serde_json::from_value(payload.clone())
        .map_err(|error| error_payload("invalid_codergen_request", error.to_string(), false))?;
    let client = client_for_payload(&payload).map_err(adapter_error_payload)?;
    let mut backend = RustLlmCodergenBackend::new(client);
    let output = backend
        .run(request)
        .map_err(|error| error_payload("codergen_failed", error.to_string(), false))?;
    Ok(json!({ "output": output }))
}

fn steer_codergen_turn(payload: Value) -> Result<Value, Value> {
    let turn_id = payload
        .get("turn_id")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim();
    if turn_id.is_empty() {
        return Err(error_payload(
            "missing_turn_id",
            "codergen steering requires turn_id",
            false,
        ));
    }
    let message = payload
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim();
    if message.is_empty() {
        return Err(error_payload(
            "missing_message",
            "codergen steering requires message",
            false,
        ));
    }
    Ok(json!({
        "output": {
            "status": "rejected",
            "delivery_mode": "rust_boundary",
            "reason": "backend_steering_unsupported",
            "message": "The serialized Rust boundary does not have an active codergen turn transport for steering.",
            "turn_id": turn_id,
        }
    }))
}

fn client_for_payload(payload: &Value) -> Result<Client, AdapterError> {
    if let Some(config_dir) = payload
        .get("metadata")
        .and_then(|metadata| metadata.get("spark.config_dir"))
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
    {
        return Client::from_env_and_profiles(Path::new(config_dir), None);
    }
    Client::from_env_with_default(None)
}

fn adapter_error_payload(error: AdapterError) -> Value {
    error_payload(
        error.kind.spec_error_name(),
        format!("{}: {}", error.kind.spec_error_name(), error.message),
        error.retryable,
    )
}

fn error_payload(kind: impl Into<String>, message: impl Into<String>, retryable: bool) -> Value {
    json!({
        "kind": kind.into(),
        "message": message.into(),
        "retryable": retryable,
    })
}

fn print_json(value: &Value) {
    let mut stdout = io::stdout().lock();
    let _ = serde_json::to_writer(&mut stdout, value);
    let _ = stdout.write_all(b"\n");
}
