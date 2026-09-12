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

fn run_agent_turn(mut payload: Value) -> Result<Value, Value> {
    capture_configuration(&mut payload)?;
    let request: AgentTurnRequest = serde_json::from_value(payload.clone())
        .map_err(|error| error_payload("invalid_agent_turn_request", error.to_string(), false))?;
    let client = client_for_payload(&payload).map_err(adapter_error_payload)?;
    let backend = RustLlmAgentTurnBackend::new(client);
    let output = backend
        .run_turn(request)
        .map_err(|error| error_payload("agent_turn_failed", error.message, error.retryable))?;
    Ok(json!({ "output": output }))
}

fn run_codergen(mut payload: Value) -> Result<Value, Value> {
    capture_configuration(&mut payload)?;
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

fn capture_configuration(payload: &mut Value) -> Result<(), Value> {
    let invalid = || {
        error_payload(
            "invalid_configuration",
            "Invalid Spark execution configuration.",
            false,
        )
    };
    if !payload.is_object()
        || payload
            .get("metadata")
            .is_some_and(|value| !value.is_object())
    {
        return Err(invalid());
    }
    if payload["metadata"]["spark.execution.settings"].is_object() {
        return Ok(());
    }
    let explicit_config_dir = payload["metadata"]["spark.config_dir"]
        .as_str()
        .map(std::path::PathBuf::from);
    let mut settings = spark_common::settings::resolve_settings_with_env(
        &spark_common::settings::SettingsOverrides {
            data_dir: explicit_config_dir
                .as_deref()
                .and_then(Path::parent)
                .map(Path::to_path_buf),
            ..Default::default()
        },
        &spark_common::paths::ProcessEnvironment,
    )
    .map_err(|_| invalid())?;
    if let Some(path) = explicit_config_dir {
        settings.config_dir = path;
    }
    let mut configuration = spark_storage::settings::read_execution_configuration(
        &settings.config_dir,
        &spark_common::paths::ProcessEnvironment,
    )
    .map_err(|_| invalid())?;
    crate::config::capture_native_binaries(&mut configuration.agents);
    let profiles = unified_llm_adapter::load_llm_profiles(&settings.config_dir)
        .map_err(|_| error_payload("invalid_configuration", "Invalid LLM profiles.", false))?;
    let omitted = |value: &Value| {
        value.is_null() || value.as_str().is_some_and(|value| value.trim().is_empty())
    };
    let project_path = payload["project_path"].as_str().unwrap_or(".");
    let (models, _) = crate::config::read_project_model_defaults(&settings, project_path)
        .map_err(|error| error_payload("invalid_configuration", error, false))?;
    if omitted(&payload["provider"]) && omitted(&payload["llm_profile"]) {
        payload["provider"] = json!(models.provider);
        payload["llm_profile"] = json!(models.llm_profile);
        if omitted(&payload["model"]) {
            payload["model"] = json!(models.model);
        }
        if omitted(&payload["reasoning_effort"]) {
            payload["reasoning_effort"] = json!(models.reasoning_effort);
        }
    }
    payload["metadata"]["spark.execution.settings"] = json!({
        "configuration": configuration, "llm_profiles": profiles,
        "model_settings": {"provider": payload["provider"], "llm_profile": payload["llm_profile"], "model": payload["model"], "reasoning_effort": payload["reasoning_effort"]}
    });
    Ok(())
}

fn client_for_payload(payload: &Value) -> Result<Client, AdapterError> {
    let snapshot = &payload["metadata"]["spark.execution.settings"];
    if let Some(configuration) = snapshot.get("configuration") {
        let configuration: spark_storage::settings::ExecutionConfiguration =
            serde_json::from_value(configuration.clone()).map_err(|_| {
                AdapterError::new(
                    unified_llm_adapter::AdapterErrorKind::Configuration,
                    "Invalid captured execution configuration.",
                )
            })?;
        let profiles = snapshot
            .get("llm_profiles")
            .cloned()
            .unwrap_or_else(|| json!({}));
        let profiles = serde_json::from_value(profiles).map_err(|_| {
            AdapterError::new(
                unified_llm_adapter::AdapterErrorKind::Configuration,
                "Invalid captured profiles.",
            )
        })?;
        let env = std::env::vars().collect();
        return Client::from_env_map(&configuration.providers.execution_environment(&env), None)
            .and_then(|client| client.with_profile_definitions(profiles, &env))
            .and_then(|client| client.with_default_provider(None));
    }
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

#[cfg(test)]
mod settings_tests {
    use super::*;

    #[test]
    fn file_edited_defaults_fail_at_agent_boundaries_before_execution() {
        for scope in ["workspace", "project"] {
            for group in [
                "provider='nonexistent-provider'",
                "provider='openai'\nmodel='claude-sonnet-4-5'",
                "provider='codex'\nreasoning_effort='invalid'",
                "llm_profile='missing'",
                "provider='openai_compatible'",
            ] {
                let home = tempfile::tempdir().unwrap();
                let config = home.path().join("config");
                let project = home.path().join("project");
                let registry = spark_storage::ProjectRegistry::new(home.path());
                let (path, section) = if scope == "workspace" {
                    (config.join("spark.toml"), "models")
                } else {
                    (
                        registry
                            .project_paths(project.to_str().unwrap())
                            .unwrap()
                            .project_file,
                        "model_settings",
                    )
                };
                std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                std::fs::write(&path, format!("[{section}]\n{group}\n")).unwrap();
                let payload =
                    json!({"project_path":project, "metadata":{"spark.config_dir":config}});
                for result in [run_agent_turn(payload.clone()), run_codergen(payload)] {
                    let error = result.unwrap_err();
                    assert_eq!(
                        error["kind"], "invalid_configuration",
                        "{scope}: {group}: {error}"
                    );
                }
            }
        }
    }

    #[test]
    fn serialized_boundary_captures_current_settings_and_preserves_existing_captures() {
        let home = tempfile::tempdir().unwrap();
        let config = home.path().join("config");
        std::fs::create_dir_all(&config).unwrap();
        let path = config.join("spark.toml");
        std::fs::write(&path, "[models]\nprovider='openai_compatible'\nmodel='first'\n[providers.openai_compatible]\nbase_url='http://localhost:1'\n[agents]\nmax_turns=3\n").unwrap();
        let mut first =
            json!({"project_path": home.path(), "metadata": {"spark.config_dir": config}});
        capture_configuration(&mut first).unwrap();
        assert_eq!(first["model"], "first");
        assert_eq!(
            first["metadata"]["spark.execution.settings"]["configuration"]["agents"]["max_turns"],
            3
        );
        std::fs::write(
            &path,
            "[models]\nprovider='openai_compatible'\nmodel='second'\n[agents]\nmax_turns=9\n",
        )
        .unwrap();
        capture_configuration(&mut first).unwrap();
        assert_eq!(first["model"], "first");
        let mut next =
            json!({"project_path": home.path(), "metadata": {"spark.config_dir": config}});
        capture_configuration(&mut next).unwrap();
        assert_eq!(next["model"], "second");
        assert_eq!(
            next["metadata"]["spark.execution.settings"]["configuration"]["agents"]["max_turns"],
            9
        );
        let project_file = spark_storage::ProjectRegistry::new(home.path())
            .project_paths(home.path().to_str().unwrap())
            .unwrap()
            .project_file;
        std::fs::create_dir_all(project_file.parent().unwrap()).unwrap();
        std::fs::write(
            &project_file,
            "[model_settings]\nprovider='codex'\nmodel='project-model'\n",
        )
        .unwrap();
        let mut inherited =
            json!({"project_path":home.path(), "metadata":{"spark.config_dir":config}});
        capture_configuration(&mut inherited).unwrap();
        assert_eq!(inherited["provider"], "codex");
        assert_eq!(inherited["model"], "project-model");
        let mut explicit = json!({"project_path":home.path(), "provider":"anthropic", "model":"explicit-model", "metadata":{"spark.config_dir":config}});
        capture_configuration(&mut explicit).unwrap();
        assert_eq!(explicit["provider"], "anthropic");
        assert_eq!(explicit["model"], "explicit-model");
        capture_configuration(&mut next).unwrap();
        assert_eq!(next["model"], "second");
        assert!(capture_configuration(&mut json!({"metadata": "invalid"})).is_err());
    }
}
