use serde::{Deserialize, Serialize};

pub const EXECUTION_MODE_NATIVE: &str = "native";
pub const EXECUTION_MODE_LOCAL_CONTAINER: &str = "local_container";
pub const EXECUTION_MODES: &[&str] = &[EXECUTION_MODE_NATIVE, EXECUTION_MODE_LOCAL_CONTAINER];

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionMode {
    Native,
    LocalContainer,
}

impl ExecutionMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Native => EXECUTION_MODE_NATIVE,
            Self::LocalContainer => EXECUTION_MODE_LOCAL_CONTAINER,
        }
    }
}

impl std::fmt::Display for ExecutionMode {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

pub fn normalize_execution_mode(value: impl AsRef<str>) -> Result<ExecutionMode, String> {
    match value.as_ref().trim().to_ascii_lowercase().as_str() {
        EXECUTION_MODE_NATIVE => Ok(ExecutionMode::Native),
        EXECUTION_MODE_LOCAL_CONTAINER => Ok(ExecutionMode::LocalContainer),
        _ => Err("execution mode must be one of: native, local_container".to_string()),
    }
}
