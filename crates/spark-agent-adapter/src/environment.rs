use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;
use thiserror::Error;

use crate::local_environment::LocalExecutionEnvironment;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecResult {
    #[serde(default)]
    pub stdout: String,
    #[serde(default)]
    pub stderr: String,
    #[serde(default)]
    pub exit_code: i32,
    #[serde(default)]
    pub timed_out: bool,
    #[serde(default)]
    pub duration_ms: u64,
}

impl Default for ExecResult {
    fn default() -> Self {
        Self {
            stdout: String::new(),
            stderr: String::new(),
            exit_code: 0,
            timed_out: false,
            duration_ms: 0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DirEntry {
    pub name: String,
    pub is_dir: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GrepOptions {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub glob_filter: Option<String>,
    #[serde(default)]
    pub case_insensitive: bool,
    #[serde(default = "default_grep_max_results")]
    pub max_results: usize,
}

impl Default for GrepOptions {
    fn default() -> Self {
        Self {
            glob_filter: None,
            case_insensitive: false,
            max_results: default_grep_max_results(),
        }
    }
}

impl GrepOptions {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn validate(&self) -> EnvironmentResult<()> {
        if self.max_results < 1 {
            return Err(EnvironmentError::InvalidInput(
                "max_results must be at least 1".to_string(),
            ));
        }
        Ok(())
    }
}

fn default_grep_max_results() -> usize {
    100
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EnvironmentInheritancePolicy {
    InheritAll,
    InheritNone,
    InheritCoreOnly,
}

impl Default for EnvironmentInheritancePolicy {
    fn default() -> Self {
        Self::InheritCoreOnly
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CommandOptions {
    pub timeout_ms: Option<u64>,
    pub working_dir: Option<PathBuf>,
    pub env_vars: BTreeMap<String, String>,
}

#[derive(Debug, Error)]
pub enum EnvironmentError {
    #[error("file not found: {0}")]
    FileNotFound(PathBuf),
    #[error("is a directory: {0}")]
    IsDirectory(PathBuf),
    #[error("not a directory: {0}")]
    NotDirectory(PathBuf),
    #[error("already exists: {0}")]
    AlreadyExists(PathBuf),
    #[error("permission denied: {0}")]
    PermissionDenied(PathBuf),
    #[error("invalid input: {0}")]
    InvalidInput(String),
    #[error("invalid utf-8 content: {0}")]
    InvalidUtf8(PathBuf),
    #[error("io error at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("command failed to start: {0}")]
    CommandStart(String),
    #[error("operation failed: {0}")]
    Other(String),
}

pub type EnvironmentResult<T> = Result<T, EnvironmentError>;

pub trait ExecutionEnvironmentBackend: fmt::Debug + Send + Sync {
    fn read_file(
        &self,
        path: &Path,
        offset: Option<usize>,
        limit: Option<usize>,
    ) -> EnvironmentResult<String>;

    fn read_file_bytes(&self, path: &Path) -> EnvironmentResult<Vec<u8>>;

    fn write_file(&self, path: &Path, content: &str) -> EnvironmentResult<()>;

    fn file_exists(&self, path: &Path) -> bool;

    fn is_directory(&self, path: &Path) -> bool;

    fn delete_file(&self, path: &Path) -> EnvironmentResult<()>;

    fn rename_file(&self, source_path: &Path, destination_path: &Path) -> EnvironmentResult<()>;

    fn list_directory(&self, path: &Path, depth: usize) -> EnvironmentResult<Vec<DirEntry>>;

    fn exec_command(&self, command: &str, options: CommandOptions)
        -> EnvironmentResult<ExecResult>;

    fn grep(&self, pattern: &str, path: &Path, options: &GrepOptions) -> EnvironmentResult<String>;

    fn glob(&self, pattern: &str, path: &Path) -> EnvironmentResult<Vec<String>>;

    fn initialize(&self) -> EnvironmentResult<()>;

    fn cleanup(&self) -> EnvironmentResult<()>;

    fn working_directory(&self) -> String;

    fn platform(&self) -> String;

    fn os_version(&self) -> String;
}

#[derive(Clone)]
pub struct ExecutionEnvironment {
    pub working_dir: Option<PathBuf>,
    pub env: BTreeMap<String, String>,
    pub metadata: BTreeMap<String, Value>,
    backend: Arc<dyn ExecutionEnvironmentBackend>,
}

impl Default for ExecutionEnvironment {
    fn default() -> Self {
        Self {
            working_dir: None,
            env: BTreeMap::new(),
            metadata: BTreeMap::new(),
            backend: Arc::new(LocalExecutionEnvironment::default()),
        }
    }
}

impl ExecutionEnvironment {
    pub fn local(working_dir: impl Into<PathBuf>) -> Self {
        let working_dir = working_dir.into();
        Self {
            working_dir: Some(working_dir.clone()),
            env: BTreeMap::new(),
            metadata: BTreeMap::new(),
            backend: Arc::new(LocalExecutionEnvironment::new(working_dir)),
        }
    }

    pub fn from_backend<B>(backend: B) -> Self
    where
        B: ExecutionEnvironmentBackend + 'static,
    {
        Self {
            working_dir: Some(PathBuf::from(backend.working_directory())),
            env: BTreeMap::new(),
            metadata: BTreeMap::new(),
            backend: Arc::new(backend),
        }
    }

    pub fn from_backend_arc(
        backend: Arc<dyn ExecutionEnvironmentBackend>,
        working_dir: Option<PathBuf>,
    ) -> Self {
        Self {
            working_dir,
            env: BTreeMap::new(),
            metadata: BTreeMap::new(),
            backend,
        }
    }

    pub fn with_metadata(mut self, metadata: BTreeMap<String, Value>) -> Self {
        self.metadata = metadata;
        self
    }

    pub fn with_env(mut self, env: BTreeMap<String, String>) -> Self {
        self.env = env;
        self
    }

    pub fn backend(&self) -> &dyn ExecutionEnvironmentBackend {
        self.backend.as_ref()
    }

    pub fn read_file(
        &self,
        path: impl AsRef<Path>,
        offset: Option<usize>,
        limit: Option<usize>,
    ) -> EnvironmentResult<String> {
        self.backend.read_file(path.as_ref(), offset, limit)
    }

    pub fn read_file_bytes(&self, path: impl AsRef<Path>) -> EnvironmentResult<Vec<u8>> {
        self.backend.read_file_bytes(path.as_ref())
    }

    pub fn write_file(&self, path: impl AsRef<Path>, content: &str) -> EnvironmentResult<()> {
        self.backend.write_file(path.as_ref(), content)
    }

    pub fn file_exists(&self, path: impl AsRef<Path>) -> bool {
        self.backend.file_exists(path.as_ref())
    }

    pub fn is_directory(&self, path: impl AsRef<Path>) -> bool {
        self.backend.is_directory(path.as_ref())
    }

    pub fn delete_file(&self, path: impl AsRef<Path>) -> EnvironmentResult<()> {
        self.backend.delete_file(path.as_ref())
    }

    pub fn rename_file(
        &self,
        source_path: impl AsRef<Path>,
        destination_path: impl AsRef<Path>,
    ) -> EnvironmentResult<()> {
        self.backend
            .rename_file(source_path.as_ref(), destination_path.as_ref())
    }

    pub fn list_directory(
        &self,
        path: impl AsRef<Path>,
        depth: usize,
    ) -> EnvironmentResult<Vec<DirEntry>> {
        self.backend.list_directory(path.as_ref(), depth)
    }

    pub fn exec_command(
        &self,
        command: impl AsRef<str>,
        options: CommandOptions,
    ) -> EnvironmentResult<ExecResult> {
        self.backend.exec_command(command.as_ref(), options)
    }

    pub fn grep(
        &self,
        pattern: impl AsRef<str>,
        path: impl AsRef<Path>,
        options: &GrepOptions,
    ) -> EnvironmentResult<String> {
        self.backend.grep(pattern.as_ref(), path.as_ref(), options)
    }

    pub fn glob(
        &self,
        pattern: impl AsRef<str>,
        path: impl AsRef<Path>,
    ) -> EnvironmentResult<Vec<String>> {
        self.backend.glob(pattern.as_ref(), path.as_ref())
    }

    pub fn initialize(&self) -> EnvironmentResult<()> {
        self.backend.initialize()
    }

    pub fn cleanup(&self) -> EnvironmentResult<()> {
        self.backend.cleanup()
    }

    pub fn working_directory(&self) -> String {
        self.backend.working_directory()
    }

    pub fn platform(&self) -> String {
        self.backend.platform()
    }

    pub fn os_version(&self) -> String {
        self.backend.os_version()
    }
}

impl fmt::Debug for ExecutionEnvironment {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ExecutionEnvironment")
            .field("working_dir", &self.working_dir)
            .field("env", &self.env)
            .field("metadata", &self.metadata)
            .field("backend", &self.backend.working_directory())
            .finish()
    }
}

impl PartialEq for ExecutionEnvironment {
    fn eq(&self, other: &Self) -> bool {
        self.working_dir == other.working_dir
            && self.env == other.env
            && self.metadata == other.metadata
            && self.working_directory() == other.working_directory()
    }
}

impl Serialize for ExecutionEnvironment {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        ExecutionEnvironmentSerde {
            working_dir: self.working_dir.clone(),
            env: self.env.clone(),
            metadata: self.metadata.clone(),
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for ExecutionEnvironment {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = ExecutionEnvironmentSerde::deserialize(deserializer)?;
        let mut environment = match value.working_dir.clone() {
            Some(working_dir) => Self::local(working_dir),
            None => Self::default(),
        };
        environment.env = value.env;
        environment.metadata = value.metadata;
        Ok(environment)
    }
}

#[derive(Serialize, Deserialize)]
struct ExecutionEnvironmentSerde {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    working_dir: Option<PathBuf>,
    #[serde(default)]
    env: BTreeMap<String, String>,
    #[serde(default)]
    metadata: BTreeMap<String, Value>,
}
