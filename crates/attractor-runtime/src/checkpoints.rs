use attractor_core::CheckpointState;
use spark_storage::{read_json_optional, write_json_atomic, JsonWriteOptions};

use crate::error::Result;
use crate::paths::RunRootPaths;

pub fn save_checkpoint(paths: &RunRootPaths, checkpoint: &CheckpointState) -> Result<()> {
    write_json_atomic(
        paths.checkpoint_json(),
        &normalize_checkpoint_for_write(checkpoint),
        JsonWriteOptions::default(),
    )?;
    Ok(())
}

pub fn read_checkpoint(paths: &RunRootPaths) -> Result<Option<CheckpointState>> {
    read_json_optional(paths.checkpoint_json()).map_err(Into::into)
}

pub fn normalize_checkpoint_for_write(checkpoint: &CheckpointState) -> CheckpointState {
    let mut normalized = checkpoint.clone();
    if normalized.timestamp.trim().is_empty() {
        normalized.timestamp = crate::events::utc_timestamp();
    }
    normalized
}
