//! Project records with inboxes (missions, standing roles): record writes are
//! atomic replacements, inbox events are appended, and both happen under one
//! lock per project and kind.
use crate::{append_jsonl_record, write_text_atomic, StorageError};
use fs2::FileExt;
use serde_json::Value;
use std::{
    fs::{self, OpenOptions},
    path::{Path, PathBuf},
};

pub struct RecordRepository {
    root: PathBuf,
}
impl RecordRepository {
    pub fn missions(project_root: &Path) -> Self {
        let root = project_root.join("missions");
        let tasks = project_root.join("tasks");
        // Missions supersede tasks: an existing task directory is adopted as-is.
        if !root.exists() && tasks.is_dir() {
            let _ = fs::rename(&tasks, &root);
        }
        Self { root }
    }
    pub fn roles(project_root: &Path) -> Self {
        Self {
            root: project_root.join("roles"),
        }
    }
    /// Record ids are file stems: mission ids and role file names alike.
    fn checked(&self, id: &str) -> Result<(), StorageError> {
        if !crate::conversation::is_safe_segment_file_id(id) {
            return Err(StorageError::InvalidRepositoryPath {
                path: self.root.clone(),
                reason: "Invalid record ID".into(),
            });
        }
        Ok(())
    }
    fn path(&self, id: &str) -> Result<PathBuf, StorageError> {
        self.checked(id)?;
        Ok(self.root.join(format!("{id}.json")))
    }
    fn events_path(&self, id: &str) -> Result<PathBuf, StorageError> {
        self.checked(id)?;
        Ok(self.root.join(id).join("events.jsonl"))
    }
    pub fn read(&self, id: &str) -> Result<Option<Value>, StorageError> {
        let path = self.path(id)?;
        match fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map(Some)
                .map_err(|source| StorageError::JsonRead { path, source }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(StorageError::io("read record", path, e)),
        }
    }
    pub fn list(&self) -> Result<Vec<Value>, StorageError> {
        if !self.root.exists() {
            return Ok(vec![]);
        }
        let mut records = vec![];
        for entry in
            fs::read_dir(&self.root).map_err(|e| StorageError::io("list records", &self.root, e))?
        {
            let path = entry
                .map_err(|e| StorageError::io("list records", &self.root, e))?
                .path();
            if path.extension().and_then(|s| s.to_str()) == Some("json") {
                if let Some(record) = self.read(path.file_stem().unwrap().to_str().unwrap_or(""))? {
                    records.push(record);
                }
            }
        }
        Ok(records)
    }
    pub fn write(&self, id: &str, value: &Value) -> Result<(), StorageError> {
        write_text_atomic(self.path(id)?, value.to_string())
    }
    pub fn read_events(&self, id: &str) -> Result<Vec<Value>, StorageError> {
        let path = self.events_path(id)?;
        let text = match fs::read_to_string(&path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
            Err(e) => return Err(StorageError::io("read inbox events", path, e)),
        };
        text.lines()
            .filter(|line| !line.trim().is_empty())
            .map(|line| {
                serde_json::from_str(line).map_err(|source| StorageError::JsonRead {
                    path: path.clone(),
                    source,
                })
            })
            .collect()
    }
    pub fn append_event(&self, id: &str, event: &Value) -> Result<(), StorageError> {
        append_jsonl_record(self.events_path(id)?, event)
    }
    /// Runs `work` while holding the project's lock for this kind of record.
    pub fn locked<T, E: From<StorageError>>(
        &self,
        work: impl FnOnce(&Self) -> Result<T, E>,
    ) -> Result<T, E> {
        fs::create_dir_all(&self.root)
            .map_err(|e| StorageError::io("create records", &self.root, e))?;
        // ponytail: one lock per project and kind; use record locks if write throughput requires it.
        let lock_path = self.root.join(".lock");
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(&lock_path)
            .map_err(|e| StorageError::io("open record lock", &lock_path, e))?;
        file.lock_exclusive()
            .map_err(|e| StorageError::io("lock records", &lock_path, e))?;
        // Closing the file also releases the cross-process lock on every error path.
        work(self)
    }
    pub fn transact<E: From<StorageError>>(
        &self,
        id: &str,
        edit: impl FnOnce(Option<Value>) -> Result<Value, E>,
    ) -> Result<Value, E> {
        self.path(id)?;
        self.locked(|repo| {
            let previous = repo.read(id)?;
            let next = edit(previous.clone())?;
            if previous.as_ref() != Some(&next) {
                repo.write(id, &next)?;
            }
            Ok(next)
        })
    }
}
