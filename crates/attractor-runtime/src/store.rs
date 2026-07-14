use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

use attractor_core::{
    CheckpointState, FlowDefinition, RawRuntimeEvent, RunManifest, RunRecord, RunResult,
};
use spark_common::settings::SparkSettings;
use spark_storage::{write_json_atomic, write_text_atomic, JsonWriteOptions};

use crate::artifacts::{ensure_run_layout, list_artifacts, write_node_artifacts, NodeArtifacts};
use crate::checkpoints::{read_checkpoint, save_checkpoint, CheckpointWriteOptions};
use crate::error::{Result, RuntimeStorageError};
use crate::events::{
    append_event, lifecycle_event, run_metadata_event_with_graph_paths, runtime_status_event,
};
use crate::journals::journal_entries_from_events;
use crate::paths::{validate_relative_path, RunRootPaths};
use crate::records::{normalize_record_for_write, read_run_record, write_run_record};
use crate::results::{
    materialize_run_result, read_materialized_run_result, write_run_result, ResultSummaryAttempt,
};

/// Notified with the run id after every durable run mutation (journal append,
/// checkpoint save, record write) so a live layer can publish incrementally.
pub type RunEventObserver = Arc<dyn Fn(&str) + Send + Sync>;

#[derive(Clone)]
pub struct RunStore {
    runs_dir: PathBuf,
    run_event_observer: Option<RunEventObserver>,
}

impl std::fmt::Debug for RunStore {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RunStore")
            .field("runs_dir", &self.runs_dir)
            .field("run_event_observer", &self.run_event_observer.is_some())
            .finish()
    }
}

#[derive(Debug, Clone, Default)]
pub struct CreateRunRequest {
    pub record: RunRecord,
    pub checkpoint: Option<CheckpointState>,
    pub manifest: Option<RunManifest>,
    pub flow_source: Option<String>,
    pub flow_definition_json: Option<String>,
}

#[derive(Debug, Clone)]
pub struct RunBundle {
    pub paths: RunRootPaths,
    pub record: Option<RunRecord>,
    pub checkpoint: Option<CheckpointState>,
    pub raw_events: Vec<RawRuntimeEvent>,
    pub journal: Vec<attractor_core::JournalEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunArtifactFile {
    pub relative_path: String,
    pub absolute_path: PathBuf,
    pub content: Vec<u8>,
}

impl RunStore {
    pub fn for_settings(settings: &SparkSettings) -> Self {
        Self {
            runs_dir: settings.runs_dir.clone(),
            run_event_observer: None,
        }
    }

    pub fn for_runs_dir(runs_dir: impl Into<PathBuf>) -> Self {
        Self {
            runs_dir: runs_dir.into(),
            run_event_observer: None,
        }
    }

    pub fn with_run_event_observer(mut self, observer: RunEventObserver) -> Self {
        self.run_event_observer = Some(observer);
        self
    }

    pub fn run_event_observer(&self) -> Option<RunEventObserver> {
        self.run_event_observer.clone()
    }

    fn notify_run_event(&self, run_id: &str) {
        if let Some(observer) = &self.run_event_observer {
            observer(run_id);
        }
    }

    pub fn run_root(&self, project_path: &str, run_id: &str) -> Result<RunRootPaths> {
        RunRootPaths::new(self.runs_dir.clone(), project_path, run_id)
    }

    pub fn create_run(&self, request: CreateRunRequest) -> Result<RunRootPaths> {
        let mut record = request.record;
        let project_path = if record.project_path.trim().is_empty() {
            record.working_directory.clone()
        } else {
            record.project_path.clone()
        };
        let paths = self.run_root(&project_path, &record.run_id)?;
        fs::create_dir_all(&paths.root)
            .map_err(|source| RuntimeStorageError::io("create run root", &paths.root, source))?;
        ensure_run_layout(&paths)?;
        if record.root_run_id.is_none() {
            record.root_run_id = Some(record.run_id.clone());
        }
        normalize_record_for_write(&mut record);
        write_run_record(&paths, &record)?;

        let checkpoint = request
            .checkpoint
            .unwrap_or_else(|| CheckpointState::new(""));
        save_checkpoint(&paths, &checkpoint, CheckpointWriteOptions::default())?;

        let manifest = request.manifest.unwrap_or_else(|| RunManifest {
            goal: String::new(),
            graph_id: record.flow_name.clone(),
            start_node: checkpoint.current_node.clone(),
            started_at: record.started_at.clone(),
            extra: Default::default(),
        });
        write_json_atomic(
            paths.logs_manifest_json(),
            &manifest,
            JsonWriteOptions::default(),
        )?;

        if let Some(source) = request.flow_source.as_deref() {
            let flow_dir = paths.artifacts_dir().join("flow");
            fs::create_dir_all(&flow_dir).map_err(|source| {
                RuntimeStorageError::io("create flow artifacts", &flow_dir, source)
            })?;
            write_text_atomic(flow_dir.join("flow-source.yaml"), source)?;
        }
        if let Some(definition_json) = request.flow_definition_json.as_deref() {
            let flow_dir = paths.artifacts_dir().join("flow");
            fs::create_dir_all(&flow_dir).map_err(|source| {
                RuntimeStorageError::io("create flow artifacts", &flow_dir, source)
            })?;
            write_text_atomic(flow_dir.join("flow-definition.json"), definition_json)?;
        }

        append_event(&paths, lifecycle_event(&record.run_id, "INITIALIZE"))?;
        append_event(
            &paths,
            runtime_status_event(
                &record.run_id,
                &record.status,
                record.outcome.clone(),
                record.outcome_reason_code.clone(),
                record.outcome_reason_message.clone(),
                none_if_empty(&record.last_error),
            ),
        )?;
        append_event(&paths, run_metadata_event_with_graph_paths(&record, &paths))?;
        write_json_atomic(
            paths.result_json(),
            &RunResult::pending(&record.run_id, &record.status),
            JsonWriteOptions::default(),
        )?;
        write_text_atomic(paths.result_markdown(), "")?;
        self.notify_run_event(&record.run_id);
        Ok(paths)
    }

    pub fn find_run_root(&self, run_id: &str) -> Result<Option<RunRootPaths>> {
        let run_id = crate::paths::validate_run_id(run_id)?;
        if !self.runs_dir.exists() {
            return Ok(None);
        }
        let projects = fs::read_dir(&self.runs_dir).map_err(|source| {
            RuntimeStorageError::io("list run projects", &self.runs_dir, source)
        })?;
        for project_entry in projects {
            let project_entry = project_entry.map_err(|source| {
                RuntimeStorageError::io("list run project entry", &self.runs_dir, source)
            })?;
            let project_path = project_entry.path();
            if !project_path.is_dir() {
                continue;
            }
            let candidate = project_path.join(&run_id);
            if candidate.is_dir() {
                let project_id = project_entry.file_name().to_string_lossy().to_string();
                return Ok(Some(RunRootPaths::from_existing_root(
                    self.runs_dir.clone(),
                    project_id,
                    &run_id,
                    candidate,
                )?));
            }
        }
        Ok(None)
    }

    pub fn read_run_bundle(&self, run_id: &str) -> Result<Option<RunBundle>> {
        let Some(paths) = self.find_run_root(run_id)? else {
            return Ok(None);
        };
        Ok(Some(self.read_bundle_for_paths(paths)?))
    }

    pub fn list_run_bundles(&self) -> Result<Vec<RunBundle>> {
        let mut bundles = self
            .list_existing_run_roots()?
            .into_iter()
            .map(|paths| self.read_bundle_for_paths(paths))
            .collect::<Result<Vec<_>>>()?;
        bundles.sort_by(|left, right| {
            let left_key = bundle_sort_key(left);
            let right_key = bundle_sort_key(right);
            right_key
                .cmp(&left_key)
                .then_with(|| left.paths.run_id.cmp(&right.paths.run_id))
        });
        Ok(bundles)
    }

    pub fn list_run_records(&self) -> Result<Vec<RunRecord>> {
        Ok(self
            .list_run_bundles()?
            .into_iter()
            .filter_map(|bundle| bundle.record)
            .collect())
    }

    pub fn list_child_run_bundles(&self, parent_run_id: &str) -> Result<Vec<RunBundle>> {
        let mut children = Vec::new();
        for paths in self.list_existing_run_roots()? {
            let Some(record) = self.read_run_record(&paths)? else {
                continue;
            };
            if record.parent_run_id.as_deref() != Some(parent_run_id) {
                continue;
            }
            let checkpoint = self.read_checkpoint(&paths)?;
            let raw_events = self.read_raw_events(&paths)?;
            let journal = journal_entries_from_events(&raw_events);
            children.push(RunBundle {
                paths,
                record: Some(record),
                checkpoint,
                raw_events,
                journal,
            });
        }
        children.sort_by(|left, right| {
            let left_record = left.record.as_ref();
            let right_record = right.record.as_ref();
            let left_index = left_record
                .and_then(|record| record.child_invocation_index)
                .unwrap_or(0);
            let right_index = right_record
                .and_then(|record| record.child_invocation_index)
                .unwrap_or(0);
            left_index
                .cmp(&right_index)
                .then_with(|| left.paths.run_id.cmp(&right.paths.run_id))
        });
        Ok(children)
    }

    pub fn next_child_invocation_index(
        &self,
        parent_run_id: &str,
        parent_node_id: &str,
    ) -> Result<u64> {
        let mut max_index = 0_u64;
        for paths in self.list_existing_run_roots()? {
            let Some(record) = self.read_run_record(&paths)? else {
                continue;
            };
            if record.parent_run_id.as_deref() == Some(parent_run_id)
                && record.parent_node_id.as_deref() == Some(parent_node_id)
            {
                max_index = max_index.max(record.child_invocation_index.unwrap_or(0));
            }
        }
        Ok(max_index.saturating_add(1))
    }

    pub fn read_run_record(&self, paths: &RunRootPaths) -> Result<Option<RunRecord>> {
        read_run_record(paths)
    }

    pub fn write_run_record(&self, paths: &RunRootPaths, record: &RunRecord) -> Result<()> {
        write_run_record(paths, record)?;
        self.notify_run_event(&paths.run_id);
        Ok(())
    }

    pub fn update_run_record<F>(&self, run_id: &str, update: F) -> Result<Option<RunRecord>>
    where
        F: FnOnce(&mut RunRecord),
    {
        let Some(bundle) = self.read_run_bundle(run_id)? else {
            return Ok(None);
        };
        let Some(mut record) = bundle.record else {
            return Ok(None);
        };
        update(&mut record);
        self.write_run_record(&bundle.paths, &record)?;
        Ok(Some(record))
    }

    pub fn append_event(
        &self,
        paths: &RunRootPaths,
        event: RawRuntimeEvent,
    ) -> Result<RawRuntimeEvent> {
        let event = append_event(paths, event)?;
        self.notify_run_event(&paths.run_id);
        Ok(event)
    }

    pub fn read_raw_events(&self, paths: &RunRootPaths) -> Result<Vec<RawRuntimeEvent>> {
        crate::events::read_raw_events(paths)
    }

    pub fn read_journal(&self, paths: &RunRootPaths) -> Result<Vec<attractor_core::JournalEntry>> {
        Ok(journal_entries_from_events(&self.read_raw_events(paths)?))
    }

    pub fn save_checkpoint(
        &self,
        paths: &RunRootPaths,
        checkpoint: &CheckpointState,
        options: CheckpointWriteOptions,
    ) -> Result<()> {
        save_checkpoint(paths, checkpoint, options)?;
        self.notify_run_event(&paths.run_id);
        Ok(())
    }

    pub fn read_checkpoint(&self, paths: &RunRootPaths) -> Result<Option<CheckpointState>> {
        read_checkpoint(paths)
    }

    pub fn write_node_artifacts(
        &self,
        paths: &RunRootPaths,
        node_id: &str,
        artifacts: &NodeArtifacts,
    ) -> Result<PathBuf> {
        write_node_artifacts(paths, node_id, artifacts)
    }

    pub fn list_artifacts(
        &self,
        paths: &RunRootPaths,
    ) -> Result<Vec<attractor_core::ArtifactInfo>> {
        list_artifacts(paths)
    }

    pub fn read_artifact(
        &self,
        paths: &RunRootPaths,
        relative_path: &str,
    ) -> Result<Option<RunArtifactFile>> {
        let relative = validate_relative_path(relative_path)?;
        let absolute_path = paths.root.join(&relative);
        let metadata = match fs::metadata(&absolute_path) {
            Ok(metadata) => metadata,
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(source) => {
                return Err(RuntimeStorageError::io(
                    "stat artifact",
                    &absolute_path,
                    source,
                ));
            }
        };
        if !metadata.is_file() {
            return Ok(None);
        }
        let canonical_run_root = fs::canonicalize(&paths.root).map_err(|source| {
            RuntimeStorageError::io("canonicalize run root", &paths.root, source)
        })?;
        let canonical_artifact = fs::canonicalize(&absolute_path).map_err(|source| {
            RuntimeStorageError::io("canonicalize artifact", &absolute_path, source)
        })?;
        if !canonical_artifact.starts_with(&canonical_run_root) {
            return Err(RuntimeStorageError::UnsafeArtifactPath {
                path: relative.to_string_lossy().replace('\\', "/"),
                reason: "resolved path must stay within the run root".to_string(),
            });
        }
        let content = fs::read(&canonical_artifact)
            .map_err(|source| RuntimeStorageError::io("read artifact", &absolute_path, source))?;
        Ok(Some(RunArtifactFile {
            relative_path: relative.to_string_lossy().replace('\\', "/"),
            absolute_path,
            content,
        }))
    }

    pub fn read_graph_source(&self, paths: &RunRootPaths) -> Result<Option<String>> {
        for relative in ["artifacts/flow/flow-source.yaml"] {
            let path = paths.root.join(relative);
            if path.is_file() {
                return fs::read_to_string(&path)
                    .map(Some)
                    .map_err(|source| RuntimeStorageError::io("read flow source", path, source));
            }
        }
        Ok(None)
    }

    pub fn materialize_result(
        &self,
        paths: &RunRootPaths,
        run_id: &str,
        status: &str,
        flow: &FlowDefinition,
        checkpoint: &CheckpointState,
        summary: Option<ResultSummaryAttempt>,
    ) -> Result<RunResult> {
        materialize_run_result(paths, run_id, status, flow, checkpoint, summary)
    }

    pub fn write_result(&self, paths: &RunRootPaths, result: &RunResult) -> Result<()> {
        write_run_result(paths, result)
    }

    pub fn read_result(&self, paths: &RunRootPaths) -> Result<Option<RunResult>> {
        read_materialized_run_result(paths)
    }

    fn list_existing_run_roots(&self) -> Result<Vec<RunRootPaths>> {
        if !self.runs_dir.exists() {
            return Ok(Vec::new());
        }
        let mut roots = Vec::new();
        let projects = fs::read_dir(&self.runs_dir).map_err(|source| {
            RuntimeStorageError::io("list run projects", &self.runs_dir, source)
        })?;
        for project_entry in projects {
            let project_entry = project_entry.map_err(|source| {
                RuntimeStorageError::io("list run project entry", &self.runs_dir, source)
            })?;
            let project_path = project_entry.path();
            if !project_path.is_dir() {
                continue;
            }
            let project_id = project_entry.file_name().to_string_lossy().to_string();
            let runs = fs::read_dir(&project_path).map_err(|source| {
                RuntimeStorageError::io("list project runs", &project_path, source)
            })?;
            for run_entry in runs {
                let run_entry = run_entry.map_err(|source| {
                    RuntimeStorageError::io("list project run entry", &project_path, source)
                })?;
                let run_root = run_entry.path();
                if !run_root.is_dir() {
                    continue;
                }
                let run_id = run_entry.file_name().to_string_lossy().to_string();
                roots.push(RunRootPaths::from_existing_root(
                    self.runs_dir.clone(),
                    project_id.clone(),
                    &run_id,
                    run_root,
                )?);
            }
        }
        Ok(roots)
    }

    pub fn delete_all_runs(&self) -> Result<()> {
        if self.runs_dir.exists() {
            fs::remove_dir_all(&self.runs_dir).map_err(|source| {
                RuntimeStorageError::io("delete runs directory", &self.runs_dir, source)
            })?;
        }
        fs::create_dir_all(&self.runs_dir).map_err(|source| {
            RuntimeStorageError::io("create runs directory", &self.runs_dir, source)
        })?;
        Ok(())
    }

    fn read_bundle_for_paths(&self, paths: RunRootPaths) -> Result<RunBundle> {
        let record = self.read_run_record(&paths)?;
        let checkpoint = self.read_checkpoint(&paths)?;
        let raw_events = self.read_raw_events(&paths)?;
        let journal = journal_entries_from_events(&raw_events);
        Ok(RunBundle {
            paths,
            record,
            checkpoint,
            raw_events,
            journal,
        })
    }
}

fn none_if_empty(value: &str) -> Option<String> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

fn bundle_sort_key(bundle: &RunBundle) -> String {
    bundle
        .record
        .as_ref()
        .and_then(|record| {
            let started_at = record.started_at.trim();
            if !started_at.is_empty() {
                Some(started_at.to_string())
            } else {
                record.ended_at.clone()
            }
        })
        .unwrap_or_default()
}
