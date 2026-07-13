use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use serde_json::Value;
use unified_llm_adapter::Message;

pub(crate) const INITIAL_CONTEXT_PATH_METADATA_KEY: &str = "spark.runtime.initial_context_path";

pub(crate) fn path_from_metadata(
    metadata: &std::collections::BTreeMap<String, Value>,
) -> Option<PathBuf> {
    metadata
        .get(INITIAL_CONTEXT_PATH_METADATA_KEY)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(PathBuf::from)
}

pub(crate) fn assembled_message_text(messages: &[Message]) -> String {
    messages
        .iter()
        .flat_map(|message| message.content.iter())
        .filter_map(|part| part.text_content())
        .collect()
}

pub(crate) fn capture_if_configured(
    metadata: &std::collections::BTreeMap<String, Value>,
    content: &str,
) -> io::Result<()> {
    let Some(path) = path_from_metadata(metadata) else {
        return Ok(());
    };
    write_if_absent(&path, content)
}

pub(crate) fn write_if_absent(path: &Path, content: &str) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    match OpenOptions::new().write(true).create_new(true).open(path) {
        Ok(mut file) => {
            if let Err(error) = file
                .write_all(content.as_bytes())
                .and_then(|_| file.sync_all())
            {
                drop(file);
                let _ = fs::remove_file(path);
                return Err(error);
            }
            Ok(())
        }
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists && path.is_file() => Ok(()),
        Err(error) => Err(error),
    }
}
