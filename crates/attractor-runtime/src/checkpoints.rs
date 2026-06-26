use attractor_core::CheckpointState;
use spark_storage::{read_json_optional, write_json_atomic, JsonWriteOptions};

use crate::error::Result;
use crate::paths::RunRootPaths;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CheckpointWriteOptions {
    pub write_root_checkpoint: bool,
    pub mirror_logs_checkpoint: bool,
}

impl Default for CheckpointWriteOptions {
    fn default() -> Self {
        Self {
            write_root_checkpoint: true,
            mirror_logs_checkpoint: true,
        }
    }
}

pub fn save_checkpoint(
    paths: &RunRootPaths,
    checkpoint: &CheckpointState,
    options: CheckpointWriteOptions,
) -> Result<()> {
    let checkpoint = normalize_checkpoint_for_write(checkpoint);
    write_json_atomic(paths.state_json(), &checkpoint, JsonWriteOptions::default())?;
    if options.write_root_checkpoint {
        write_json_atomic(
            paths.checkpoint_json(),
            &checkpoint,
            JsonWriteOptions::default(),
        )?;
    }
    if options.mirror_logs_checkpoint {
        write_json_atomic(
            paths.logs_checkpoint_json(),
            &checkpoint,
            JsonWriteOptions::default(),
        )?;
    }
    Ok(())
}

pub fn read_checkpoint(paths: &RunRootPaths) -> Result<Option<CheckpointState>> {
    for path in [
        paths.state_json(),
        paths.checkpoint_json(),
        paths.logs_checkpoint_json(),
    ] {
        if let Some(checkpoint) = read_json_optional::<CheckpointState>(path)? {
            return Ok(Some(checkpoint));
        }
    }
    Ok(None)
}

pub fn normalize_checkpoint_for_write(checkpoint: &CheckpointState) -> CheckpointState {
    let mut normalized = checkpoint.clone();
    if normalized.timestamp.trim().is_empty() {
        normalized.timestamp = crate::events::utc_timestamp();
    }
    normalized
}
