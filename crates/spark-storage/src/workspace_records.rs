//! Project records of missions and standing roles: record writes are atomic
//! replacements under one lock per project and kind. Their inboxes live in
//! the conversation store.
use crate::{write_text_atomic, StorageError};
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
        Self {
            root: project_root.join("missions"),
        }
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
