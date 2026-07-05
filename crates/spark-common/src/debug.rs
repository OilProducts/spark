use crate::paths::{Environment, ProcessEnvironment};

pub const ENV_SPARK_DEBUG_CODEX_JSONRPC: &str = "SPARK_DEBUG_CODEX_JSONRPC";
pub const CODEX_JSONRPC_TRACE_FILE_NAME: &str = "codex-jsonrpc-trace.jsonl";
pub const CODEX_JSONRPC_TRACE_PATH_METADATA_KEY: &str = "spark.runtime.codex_jsonrpc_trace_path";

pub fn codex_jsonrpc_trace_enabled() -> bool {
    codex_jsonrpc_trace_enabled_with_env(&ProcessEnvironment)
}

pub fn codex_jsonrpc_trace_enabled_with_env(env: &impl Environment) -> bool {
    env.get_var(ENV_SPARK_DEBUG_CODEX_JSONRPC)
        .as_deref()
        .is_some_and(is_truthy_env_value)
}

pub fn is_truthy_env_value(value: &str) -> bool {
    matches!(
        value.trim(),
        "1" | "true" | "TRUE" | "yes" | "YES" | "on" | "ON"
    )
}
