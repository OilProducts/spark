use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::error::{Result, StorageError};

/// Replace a file atomically after the full byte payload has been written.
pub fn write_atomic(path: impl AsRef<Path>, bytes: impl AsRef<[u8]>) -> Result<()> {
    let path = path.as_ref();
    let parent = prepare_parent(path)?;
    let mut temp = tempfile::Builder::new()
        .prefix(".spark-storage-")
        .tempfile_in(&parent)
        .map_err(|source| StorageError::io("create temporary file in", &parent, source))?;

    temp.write_all(bytes.as_ref())
        .map_err(|source| StorageError::io("write temporary file for", path, source))?;
    temp.flush()
        .map_err(|source| StorageError::io("flush temporary file for", path, source))?;
    temp.as_file()
        .sync_all()
        .map_err(|source| StorageError::io("sync temporary file for", path, source))?;

    let temp_path = temp.into_temp_path();
    let temp_path_buf = temp_path.to_path_buf();
    temp_path
        .persist(path)
        .map_err(|source| StorageError::AtomicPersist {
            path: path.to_path_buf(),
            temp_path: temp_path_buf,
            source,
        })?;
    Ok(())
}

/// UTF-8 wrapper for [`write_atomic`].
pub fn write_text_atomic(path: impl AsRef<Path>, text: impl AsRef<str>) -> Result<()> {
    write_atomic(path, text.as_ref().as_bytes())
}

/// Append one line, adding a trailing newline when the caller did not provide it.
pub fn append_line(path: impl AsRef<Path>, line: impl AsRef<str>) -> Result<()> {
    let mut bytes = line.as_ref().as_bytes().to_vec();
    if !bytes.ends_with(b"\n") {
        bytes.push(b'\n');
    }
    append_bytes(path.as_ref(), &bytes)
}

/// Serialize and append one JSONL record without truncating existing contents.
pub fn append_jsonl_record<T: Serialize>(path: impl AsRef<Path>, record: &T) -> Result<()> {
    let path = path.as_ref();
    let mut bytes = serde_json::to_vec(record).map_err(|source| StorageError::JsonWrite {
        path: path.to_path_buf(),
        source,
    })?;
    bytes.push(b'\n');
    append_bytes(path, &bytes)
}

fn append_bytes(path: &Path, bytes: &[u8]) -> Result<()> {
    let _parent = prepare_parent(path)?;
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|source| StorageError::io("open append-only file", path, source))?;
    file.write_all(bytes)
        .map_err(|source| StorageError::io("append to", path, source))?;
    file.flush()
        .map_err(|source| StorageError::io("flush append-only file", path, source))?;
    file.sync_all()
        .map_err(|source| StorageError::io("sync append-only file", path, source))?;
    Ok(())
}

fn prepare_parent(path: &Path) -> Result<PathBuf> {
    if path.as_os_str().is_empty() {
        return Err(StorageError::InvalidRepositoryPath {
            path: path.to_path_buf(),
            reason: "path must not be empty".to_string(),
        });
    }

    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf();
    fs::create_dir_all(&parent)
        .map_err(|source| StorageError::io("create parent directory", &parent, source))?;
    Ok(parent)
}
