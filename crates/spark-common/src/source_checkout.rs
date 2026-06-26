use std::path::{Path, PathBuf};

use crate::error::{Result, SparkCommonError};
use crate::paths::{Environment, ProcessEnvironment};

pub const DEFAULT_API_BASE_URL: &str = "http://127.0.0.1:8000";

pub fn is_source_checkout(project_root: impl AsRef<Path>) -> bool {
    let root = project_root.as_ref();
    root.join(".git").exists()
        || (root.join("pyproject.toml").is_file()
            && root.join("src/spark/flows").is_dir()
            && root.join("frontend").is_dir())
}

pub fn require_explicit_dev_home(
    command: &str,
    data_dir: Option<&Path>,
    project_root: &Path,
) -> Result<()> {
    require_explicit_dev_home_with_env(command, data_dir, project_root, &ProcessEnvironment)
}

pub fn require_explicit_dev_home_with_env(
    command: &str,
    data_dir: Option<&Path>,
    project_root: &Path,
    env: &impl Environment,
) -> Result<()> {
    if !is_source_checkout(project_root) {
        return Ok(());
    }
    if data_dir.is_some()
        || env
            .get_var("SPARK_HOME")
            .map(|value| !value.is_empty())
            .unwrap_or(false)
    {
        return Ok(());
    }
    Err(SparkCommonError::SourceCheckoutGuard(
        default_runtime_home_refusal_message(command, project_root),
    ))
}

pub fn require_explicit_agent_base_url(
    command: &str,
    base_url: Option<&str>,
    project_root: &Path,
) -> Result<()> {
    require_explicit_agent_base_url_with_env(command, base_url, project_root, &ProcessEnvironment)
}

pub fn require_explicit_agent_base_url_with_env(
    command: &str,
    base_url: Option<&str>,
    project_root: &Path,
    env: &impl Environment,
) -> Result<()> {
    if !is_source_checkout(project_root) {
        return Ok(());
    }
    if base_url
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .is_some()
        || env
            .get_var("SPARK_API_BASE_URL")
            .map(|value| !value.trim().is_empty())
            .unwrap_or(false)
    {
        return Ok(());
    }
    Err(SparkCommonError::SourceCheckoutGuard(
        default_api_target_refusal_message(command, project_root),
    ))
}

pub fn default_runtime_home_refusal_message(command: &str, project_root: &Path) -> String {
    join_lines([
        format!(
            "Refusing to use default runtime home ~/.spark from a source checkout at {}.",
            project_root.display()
        ),
        String::new(),
        "The default home is reserved for the installed or stable Spark instance.".to_string(),
        format!(
            "Run the source checkout with an explicit dev home before `{command}`, for example:"
        ),
        String::new(),
        "  SPARK_HOME=~/.spark-dev uv run spark-server init".to_string(),
        "  SPARK_HOME=~/.spark-dev uv run spark-server serve --reload --port 8010".to_string(),
    ])
}

pub fn default_api_target_refusal_message(command: &str, project_root: &Path) -> String {
    join_lines([
        format!(
            "Refusing to use default API target {DEFAULT_API_BASE_URL} from a source checkout at {}.",
            project_root.display()
        ),
        String::new(),
        "The default API target is reserved for the installed or stable Spark instance.".to_string(),
        format!(
            "Run the source checkout with an explicit dev server target before `{command}`, for example:"
        ),
        String::new(),
        "  SPARK_API_BASE_URL=http://127.0.0.1:8010 uv run spark flow list".to_string(),
        "  SPARK_API_BASE_URL=http://127.0.0.1:8010 uv run spark flow describe --flow examples/simple-linear.dot".to_string(),
        "  uv run spark flow validate --file src/spark/flows/examples/simple-linear.dot --text".to_string(),
    ])
}

pub fn source_checkout_root_from_manifest() -> PathBuf {
    crate::paths::detect_project_root()
}

fn join_lines<const N: usize>(lines: [String; N]) -> String {
    lines.join("\n")
}
