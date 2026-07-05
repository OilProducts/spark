use std::fs;
use std::io::{BufRead, BufReader};
use std::path::Path;

use serde::de::DeserializeOwned;
use serde::Serialize;

use crate::atomic;
use crate::error::{Result, StorageError};

/// JSON document write options.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JsonWriteOptions {
    pub pretty: bool,
    pub trailing_newline: bool,
}

impl Default for JsonWriteOptions {
    fn default() -> Self {
        Self {
            pretty: true,
            trailing_newline: true,
        }
    }
}

/// JSONL parse behavior for blank lines.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JsonLinesPolicy {
    Strict,
    AllowBlankLines,
}

/// JSONL read options.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JsonLinesOptions {
    pub policy: JsonLinesPolicy,
}

impl JsonLinesOptions {
    pub const fn strict() -> Self {
        Self {
            policy: JsonLinesPolicy::Strict,
        }
    }

    pub const fn allow_blank_lines() -> Self {
        Self {
            policy: JsonLinesPolicy::AllowBlankLines,
        }
    }
}

impl Default for JsonLinesOptions {
    fn default() -> Self {
        Self::strict()
    }
}

pub fn read_json<T: DeserializeOwned>(path: impl AsRef<Path>) -> Result<T> {
    let path = path.as_ref();
    let text = fs::read_to_string(path)
        .map_err(|source| StorageError::io("read JSON file", path, source))?;
    serde_json::from_str(&text).map_err(|source| StorageError::JsonRead {
        path: path.to_path_buf(),
        source,
    })
}

pub fn read_json_optional<T: DeserializeOwned>(path: impl AsRef<Path>) -> Result<Option<T>> {
    let path = path.as_ref();
    match fs::read_to_string(path) {
        Ok(text) => {
            serde_json::from_str(&text)
                .map(Some)
                .map_err(|source| StorageError::JsonRead {
                    path: path.to_path_buf(),
                    source,
                })
        }
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(source) => Err(StorageError::io("read JSON file", path, source)),
    }
}

pub fn write_json_atomic<T: Serialize>(
    path: impl AsRef<Path>,
    value: &T,
    options: JsonWriteOptions,
) -> Result<()> {
    let path = path.as_ref();
    let mut bytes = if options.pretty {
        serde_json::to_vec_pretty(value)
    } else {
        serde_json::to_vec(value)
    }
    .map_err(|source| StorageError::JsonWrite {
        path: path.to_path_buf(),
        source,
    })?;
    if options.trailing_newline && !bytes.ends_with(b"\n") {
        bytes.push(b'\n');
    }
    atomic::write_atomic(path, bytes)
}

pub fn read_toml<T: DeserializeOwned>(path: impl AsRef<Path>) -> Result<T> {
    let path = path.as_ref();
    let text = fs::read_to_string(path)
        .map_err(|source| StorageError::io("read TOML file", path, source))?;
    toml::from_str(&text).map_err(|source| StorageError::TomlRead {
        path: path.to_path_buf(),
        source,
    })
}

pub fn read_toml_optional<T: DeserializeOwned>(path: impl AsRef<Path>) -> Result<Option<T>> {
    let path = path.as_ref();
    match fs::read_to_string(path) {
        Ok(text) => toml::from_str(&text)
            .map(Some)
            .map_err(|source| StorageError::TomlRead {
                path: path.to_path_buf(),
                source,
            }),
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(source) => Err(StorageError::io("read TOML file", path, source)),
    }
}

pub fn write_toml_atomic<T: Serialize>(path: impl AsRef<Path>, value: &T) -> Result<()> {
    let path = path.as_ref();
    let mut text = toml::to_string_pretty(value).map_err(|source| StorageError::TomlWrite {
        path: path.to_path_buf(),
        source,
    })?;
    if !text.ends_with('\n') {
        text.push('\n');
    }
    atomic::write_text_atomic(path, text)
}

pub fn read_jsonl<T: DeserializeOwned>(
    path: impl AsRef<Path>,
    options: JsonLinesOptions,
) -> Result<Vec<T>> {
    let path = path.as_ref();
    let file =
        fs::File::open(path).map_err(|source| StorageError::io("read JSONL file", path, source))?;
    let reader = BufReader::new(file);
    let mut records = Vec::new();

    for (index, line) in reader.lines().enumerate() {
        let line_number = index + 1;
        let line = line.map_err(|source| StorageError::io("read JSONL line", path, source))?;
        if matches!(options.policy, JsonLinesPolicy::AllowBlankLines) && line.trim().is_empty() {
            continue;
        }
        let record = serde_json::from_str(&line).map_err(|source| StorageError::JsonlLine {
            path: path.to_path_buf(),
            line: line_number,
            source,
        })?;
        records.push(record);
    }

    Ok(records)
}

pub fn append_jsonl<T: Serialize>(path: impl AsRef<Path>, value: &T) -> Result<()> {
    atomic::append_jsonl_record(path, value)
}
