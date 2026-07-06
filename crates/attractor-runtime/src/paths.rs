use std::path::{Component, Path, PathBuf};

use spark_common::project::build_project_id;
use spark_common::settings::SparkSettings;

use crate::error::{Result, RuntimeStorageError};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimePaths {
    pub runs_dir: PathBuf,
}

impl RuntimePaths {
    pub fn for_settings(settings: &SparkSettings) -> Self {
        Self {
            runs_dir: settings.runs_dir.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunRootPaths {
    pub runs_dir: PathBuf,
    pub project_id: String,
    pub run_id: String,
    pub root: PathBuf,
}

impl RunRootPaths {
    pub fn new(
        runs_dir: impl Into<PathBuf>,
        project_path: impl AsRef<str>,
        run_id: impl AsRef<str>,
    ) -> Result<Self> {
        let runs_dir = runs_dir.into();
        let run_id = validate_run_id(run_id.as_ref())?;
        let project_id = build_project_id(project_path.as_ref())?;
        let root = runs_dir.join(&project_id).join(&run_id);
        Ok(Self {
            runs_dir,
            project_id,
            run_id,
            root,
        })
    }

    pub fn from_existing_root(
        runs_dir: impl Into<PathBuf>,
        project_id: impl Into<String>,
        run_id: impl AsRef<str>,
        root: impl Into<PathBuf>,
    ) -> Result<Self> {
        let run_id = validate_run_id(run_id.as_ref())?;
        Ok(Self {
            runs_dir: runs_dir.into(),
            project_id: project_id.into(),
            run_id,
            root: root.into(),
        })
    }

    pub fn ensure_exists(&self) -> Result<()> {
        if self.root.is_dir() {
            Ok(())
        } else {
            Err(RuntimeStorageError::MissingRunRoot {
                path: self.root.clone(),
            })
        }
    }

    pub fn run_json(&self) -> PathBuf {
        self.root.join("run.json")
    }

    pub fn events_jsonl(&self) -> PathBuf {
        self.root.join("events.jsonl")
    }

    pub fn transcript_json(&self) -> PathBuf {
        self.root.join("transcript.json")
    }

    pub fn state_json(&self) -> PathBuf {
        self.root.join("state.json")
    }

    pub fn checkpoint_json(&self) -> PathBuf {
        self.root.join("checkpoint.json")
    }

    pub fn logs_checkpoint_json(&self) -> PathBuf {
        self.logs_dir().join("checkpoint.json")
    }

    pub fn manifest_json(&self) -> PathBuf {
        self.root.join("manifest.json")
    }

    pub fn logs_manifest_json(&self) -> PathBuf {
        self.logs_dir().join("manifest.json")
    }

    pub fn run_log(&self) -> PathBuf {
        self.root.join("run.log")
    }

    pub fn logs_dir(&self) -> PathBuf {
        self.root.join("logs")
    }

    pub fn artifacts_dir(&self) -> PathBuf {
        self.root.join("artifacts")
    }

    pub fn result_dir(&self) -> PathBuf {
        self.root.join("result")
    }

    pub fn result_json(&self) -> PathBuf {
        self.result_dir().join("result.json")
    }

    pub fn result_markdown(&self) -> PathBuf {
        self.result_dir().join("result.md")
    }

    pub fn safe_join(&self, relative_path: impl AsRef<str>) -> Result<PathBuf> {
        let relative = validate_relative_path(relative_path.as_ref())?;
        Ok(self.root.join(relative))
    }
}

pub fn validate_run_id(value: &str) -> Result<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(RuntimeStorageError::InvalidRunId {
            run_id: value.to_string(),
            reason: "run id must be non-empty".to_string(),
        });
    }
    let path = Path::new(trimmed);
    if path.is_absolute() {
        return Err(RuntimeStorageError::InvalidRunId {
            run_id: value.to_string(),
            reason: "run id must be a single relative path segment".to_string(),
        });
    }
    let components = path.components().collect::<Vec<_>>();
    if !matches!(components.as_slice(), [Component::Normal(_)]) {
        return Err(RuntimeStorageError::InvalidRunId {
            run_id: value.to_string(),
            reason: "run id must be a single relative path segment".to_string(),
        });
    }
    Ok(trimmed.to_string())
}

pub fn validate_relative_path(value: &str) -> Result<PathBuf> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(RuntimeStorageError::UnsafeArtifactPath {
            path: value.to_string(),
            reason: "path must be non-empty".to_string(),
        });
    }
    let path = Path::new(trimmed);
    if path.is_absolute() {
        return Err(RuntimeStorageError::UnsafeArtifactPath {
            path: value.to_string(),
            reason: "absolute paths are not allowed".to_string(),
        });
    }

    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => normalized.push(part),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(RuntimeStorageError::UnsafeArtifactPath {
                    path: value.to_string(),
                    reason: "path must stay within the run root".to_string(),
                });
            }
        }
    }
    if normalized.as_os_str().is_empty() {
        return Err(RuntimeStorageError::UnsafeArtifactPath {
            path: value.to_string(),
            reason: "path must be non-empty".to_string(),
        });
    }
    Ok(normalized)
}
