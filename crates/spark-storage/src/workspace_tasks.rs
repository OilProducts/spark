//! Project task transactions: record and history share one atomic replacement.
use crate::{write_text_atomic, StorageError};
use fs2::FileExt;
use serde_json::Value;
use std::{
    fs::{self, OpenOptions},
    path::{Path, PathBuf},
};

pub struct TaskRepository {
    root: PathBuf,
}
impl TaskRepository {
    pub fn new(project_root: &Path) -> Self {
        Self {
            root: project_root.join("tasks"),
        }
    }
    fn path(&self, id: &str) -> Result<PathBuf, StorageError> {
        if id.is_empty() || !id.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-') {
            return Err(StorageError::InvalidRepositoryPath {
                path: self.root.clone(),
                reason: "Invalid task ID".into(),
            });
        }
        Ok(self.root.join(format!("{id}.json")))
    }
    pub fn read(&self, id: &str) -> Result<Option<Value>, StorageError> {
        let path = self.path(id)?;
        match fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map(Some)
                .map_err(|source| StorageError::JsonRead { path, source }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(StorageError::io("read task", path, e)),
        }
    }
    pub fn list(&self) -> Result<Vec<Value>, StorageError> {
        if !self.root.exists() {
            return Ok(vec![]);
        }
        let mut tasks = vec![];
        for entry in
            fs::read_dir(&self.root).map_err(|e| StorageError::io("list tasks", &self.root, e))?
        {
            let path = entry
                .map_err(|e| StorageError::io("list tasks", &self.root, e))?
                .path();
            if path.extension().and_then(|s| s.to_str()) == Some("json") {
                if let Some(task) = self.read(path.file_stem().unwrap().to_str().unwrap_or(""))? {
                    tasks.push(task);
                }
            }
        }
        Ok(tasks)
    }
    pub fn transact<E: From<StorageError>>(
        &self,
        id: &str,
        edit: impl FnOnce(Option<Value>) -> Result<Value, E>,
    ) -> Result<Value, E> {
        let path = self.path(id)?;
        fs::create_dir_all(&self.root)
            .map_err(|e| StorageError::io("create tasks", &self.root, e))?;
        // ponytail: one project lock; use task locks if write throughput requires it.
        let lock_path = self.root.join(".lock");
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(&lock_path)
            .map_err(|e| StorageError::io("open task lock", &lock_path, e))?;
        file.lock_exclusive()
            .map_err(|e| StorageError::io("lock tasks", &lock_path, e))?;
        // Closing the file also releases the cross-process lock on every error path.
        let previous = self.read(id)?;
        let next = edit(previous.clone())?;
        if previous.as_ref() != Some(&next) {
            write_text_atomic(path, next.to_string())?;
        }
        Ok(next)
    }
}
