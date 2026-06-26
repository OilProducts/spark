use std::marker::PhantomData;
use std::path::{Path, PathBuf};

use serde::de::DeserializeOwned;
use serde::Serialize;

use crate::codecs::{
    append_jsonl, read_json, read_json_optional, read_jsonl, read_toml, read_toml_optional,
    write_json_atomic, write_toml_atomic, JsonLinesOptions, JsonWriteOptions,
};
use crate::error::{Result, StorageError};

/// Canonical durable file formats supported by the storage foundation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StorageFormat {
    Json,
    Toml,
    Jsonl,
}

impl StorageFormat {
    pub const fn extension(self) -> &'static str {
        match self {
            Self::Json => "json",
            Self::Toml => "toml",
            Self::Jsonl => "jsonl",
        }
    }
}

/// Schema-neutral document repository contract.
pub trait DocumentRepository<T> {
    fn path(&self) -> &Path;
    fn format(&self) -> StorageFormat;
    fn read(&self) -> Result<T>;
    fn read_optional(&self) -> Result<Option<T>>;
    fn write(&self, value: &T) -> Result<()>;
}

/// Append-only repository contract for ordered event and journal files.
pub trait AppendLogRepository<T> {
    fn path(&self) -> &Path;
    fn format(&self) -> StorageFormat;
    fn read_all(&self) -> Result<Vec<T>>;
    fn append(&self, value: &T) -> Result<()>;
}

#[derive(Debug, Clone)]
pub struct JsonRepository<T> {
    path: PathBuf,
    options: JsonWriteOptions,
    _record: PhantomData<fn() -> T>,
}

impl<T> JsonRepository<T> {
    pub fn new(path: impl Into<PathBuf>) -> Result<Self> {
        Ok(Self {
            path: validate_repository_path(path.into())?,
            options: JsonWriteOptions::default(),
            _record: PhantomData,
        })
    }

    pub fn with_write_options(mut self, options: JsonWriteOptions) -> Self {
        self.options = options;
        self
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn format(&self) -> StorageFormat {
        StorageFormat::Json
    }
}

impl<T> DocumentRepository<T> for JsonRepository<T>
where
    T: Serialize + DeserializeOwned,
{
    fn path(&self) -> &Path {
        &self.path
    }

    fn format(&self) -> StorageFormat {
        StorageFormat::Json
    }

    fn read(&self) -> Result<T> {
        read_json(&self.path)
    }

    fn read_optional(&self) -> Result<Option<T>> {
        read_json_optional(&self.path)
    }

    fn write(&self, value: &T) -> Result<()> {
        write_json_atomic(&self.path, value, self.options)
    }
}

#[derive(Debug, Clone)]
pub struct TomlRepository<T> {
    path: PathBuf,
    _record: PhantomData<fn() -> T>,
}

impl<T> TomlRepository<T> {
    pub fn new(path: impl Into<PathBuf>) -> Result<Self> {
        Ok(Self {
            path: validate_repository_path(path.into())?,
            _record: PhantomData,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn format(&self) -> StorageFormat {
        StorageFormat::Toml
    }
}

impl<T> DocumentRepository<T> for TomlRepository<T>
where
    T: Serialize + DeserializeOwned,
{
    fn path(&self) -> &Path {
        &self.path
    }

    fn format(&self) -> StorageFormat {
        StorageFormat::Toml
    }

    fn read(&self) -> Result<T> {
        read_toml(&self.path)
    }

    fn read_optional(&self) -> Result<Option<T>> {
        read_toml_optional(&self.path)
    }

    fn write(&self, value: &T) -> Result<()> {
        write_toml_atomic(&self.path, value)
    }
}

#[derive(Debug, Clone)]
pub struct JsonlRepository<T> {
    path: PathBuf,
    read_options: JsonLinesOptions,
    _record: PhantomData<fn() -> T>,
}

impl<T> JsonlRepository<T> {
    pub fn new(path: impl Into<PathBuf>) -> Result<Self> {
        Ok(Self {
            path: validate_repository_path(path.into())?,
            read_options: JsonLinesOptions::default(),
            _record: PhantomData,
        })
    }

    pub fn with_read_options(mut self, options: JsonLinesOptions) -> Self {
        self.read_options = options;
        self
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn format(&self) -> StorageFormat {
        StorageFormat::Jsonl
    }
}

impl<T> AppendLogRepository<T> for JsonlRepository<T>
where
    T: Serialize + DeserializeOwned,
{
    fn path(&self) -> &Path {
        &self.path
    }

    fn format(&self) -> StorageFormat {
        StorageFormat::Jsonl
    }

    fn read_all(&self) -> Result<Vec<T>> {
        read_jsonl(&self.path, self.read_options)
    }

    fn append(&self, value: &T) -> Result<()> {
        append_jsonl(&self.path, value)
    }
}

fn validate_repository_path(path: PathBuf) -> Result<PathBuf> {
    if path.as_os_str().is_empty() {
        return Err(StorageError::InvalidRepositoryPath {
            path,
            reason: "path must not be empty".to_string(),
        });
    }
    if path.exists() && path.is_dir() {
        return Err(StorageError::InvalidRepositoryPath {
            path,
            reason: "path must point to a file, not a directory".to_string(),
        });
    }
    Ok(path)
}
