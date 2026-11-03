//! Shared append-only activity storage for conversations and node executions.

use std::fs::{self, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use fs2::FileExt;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::conversation::{
    externalize_segment_tool_output, Transcript, TranscriptSegment, TranscriptTurn,
};
use crate::{Result, StorageError};

pub const ACTIVITY_EVENTS_FILE_NAME: &str = "events.jsonl";
pub const ACTIVITY_TRANSCRIPT_FILE_NAME: &str = "transcript.jsonl";
const ACTIVITY_LOCK_FILE_NAME: &str = ".activity.lock";
const TAIL_CHUNK_BYTES: u64 = 64 * 1024;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ActivityEvent {
    pub sequence: u64,
    pub committed_at: String,
    pub event: Value,
}

/// A complete logical transcript record. Streaming deltas have no variant in
/// this schema and therefore cannot accidentally become transcript authority.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TranscriptRecord {
    TurnUpsert {
        revision: u64,
        committed_at: String,
        source_event_sequence: u64,
        turn: TranscriptTurn,
    },
    SegmentUpsert {
        revision: u64,
        committed_at: String,
        source_event_sequence: u64,
        segment: TranscriptSegment,
    },
    /// Removal of one obsolete projected segment (compatibility repair).
    /// Appended like every other record; hydration applies it in order, so
    /// raw history is never rewritten.
    SegmentTombstone {
        revision: u64,
        committed_at: String,
        source_event_sequence: u64,
        turn_id: String,
        segment_id: String,
    },
}

impl TranscriptRecord {
    pub fn revision(&self) -> u64 {
        match self {
            Self::TurnUpsert { revision, .. }
            | Self::SegmentUpsert { revision, .. }
            | Self::SegmentTombstone { revision, .. } => *revision,
        }
    }

    pub fn source_event_sequence(&self) -> u64 {
        match self {
            Self::TurnUpsert {
                source_event_sequence,
                ..
            }
            | Self::SegmentUpsert {
                source_event_sequence,
                ..
            }
            | Self::SegmentTombstone {
                source_event_sequence,
                ..
            } => *source_event_sequence,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ActivityRepository {
    root: PathBuf,
    append_lock: Arc<Mutex<ActivityTail>>,
}

#[derive(Debug, Default)]
struct ActivityTail {
    event_sequence: Option<u64>,
    event_len: u64,
    transcript_revision: Option<u64>,
    transcript_len: u64,
}

impl ActivityRepository {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            append_lock: Arc::new(Mutex::new(ActivityTail::default())),
        }
    }

    pub fn events_path(&self) -> PathBuf {
        self.root.join(ACTIVITY_EVENTS_FILE_NAME)
    }

    pub fn transcript_path(&self) -> PathBuf {
        self.root.join(ACTIVITY_TRANSCRIPT_FILE_NAME)
    }

    pub fn externalize_tool_output(&self, artifact_id: &str, tool_call: &mut Value) -> Result<()> {
        let Some(tool_call) = tool_call.as_object_mut() else {
            return Ok(());
        };
        crate::conversation::externalize_tool_output(&self.root, artifact_id, tool_call)
    }

    /// Append detailed activity first and return its dense, tail-derived sequence.
    pub fn append_event(
        &self,
        event: Value,
        committed_at: impl Into<String>,
    ) -> Result<ActivityEvent> {
        let mut guard = self.lock()?;
        let _file_lock = ActivityFileLock::acquire(&self.root)?;
        let current_len = file_len(&self.events_path())?;
        let tail = if guard.event_sequence.is_some() && guard.event_len == current_len {
            guard.event_sequence.unwrap_or(0)
        } else {
            repair_partial_tail::<ActivityEvent>(&self.events_path())?;
            tail_event_sequence(&self.events_path())?
        };
        let sequence = tail.saturating_add(1);
        let record = ActivityEvent {
            sequence,
            committed_at: committed_at.into(),
            event,
        };
        append_activity_record(&self.events_path(), &record)?;
        guard.event_sequence = Some(sequence);
        guard.event_len = file_len(&self.events_path())?;
        Ok(record)
    }

    /// Append a full semantic record after its owning detailed event.
    pub fn append_transcript(&self, record: &TranscriptRecord) -> Result<()> {
        let mut record = record.clone();
        externalize_tool_output(&self.root, &mut record)?;
        let mut guard = self.lock()?;
        let _file_lock = ActivityFileLock::acquire(&self.root)?;
        repair_partial_tail::<TranscriptRecord>(&self.transcript_path())?;
        let current_len = file_len(&self.transcript_path())?;
        let transcript_tail =
            if guard.transcript_revision.is_some() && guard.transcript_len == current_len {
                guard.transcript_revision.unwrap_or(0)
            } else {
                tail_transcript_revision(&self.transcript_path())?
            };
        let expected = transcript_tail.saturating_add(1);
        if record.revision() != expected {
            return Err(StorageError::InvalidRepositoryPath {
                path: self.transcript_path(),
                reason: format!(
                    "transcript revision must be {expected}, got {}",
                    record.revision()
                ),
            });
        }
        let event_tail = tail_event_sequence(&self.events_path())?;
        if record.source_event_sequence() > event_tail {
            return Err(StorageError::InvalidRepositoryPath {
                path: self.transcript_path(),
                reason: "transcript source event has not been committed".to_string(),
            });
        }
        append_activity_record(&self.transcript_path(), &record)?;
        guard.transcript_revision = Some(record.revision());
        guard.transcript_len = file_len(&self.transcript_path())?;
        Ok(())
    }

    /// Flush both logs to disk. Appends do not sync; a commit calls this once
    /// before publishing state that refers to the appended records.
    pub fn sync(&self) -> Result<()> {
        for path in [self.events_path(), self.transcript_path()] {
            // Windows `sync_all` (FlushFileBuffers) needs write access; no
            // create or truncate, so a missing log stays missing.
            match fs::OpenOptions::new().write(true).open(&path) {
                Ok(file) => file
                    .sync_all()
                    .map_err(|error| activity_io("sync", &path, error))?,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(activity_io("open", &path, error)),
            }
        }
        Ok(())
    }

    pub fn read_events(&self) -> Result<Vec<ActivityEvent>> {
        read_lines(&self.events_path())
    }

    pub fn read_transcript_records(&self) -> Result<Vec<TranscriptRecord>> {
        read_lines(&self.transcript_path())
    }

    /// Read only records appended after a caller-owned byte cursor. An ignored
    /// crash tail is not consumed: the returned cursor stops at its start so
    /// records appended after append-time repair are delivered exactly once.
    pub fn read_transcript_records_from(
        &self,
        byte_offset: u64,
    ) -> Result<(Vec<TranscriptRecord>, u64)> {
        read_activity_records(&self.transcript_path(), byte_offset)
    }

    /// Hydrate in logical-record time; cost is independent of delta-event count.
    pub fn hydrate_transcript(&self) -> Result<Transcript> {
        self.hydrate_transcript_through_event_sequence(u64::MAX)
    }

    /// Hydrate only semantic records whose detailed event is published.
    pub fn hydrate_transcript_through_event_sequence(
        &self,
        event_sequence: u64,
    ) -> Result<Transcript> {
        let records = self.read_transcript_records()?;
        let mut transcript = Transcript::default();
        for record in records
            .into_iter()
            .filter(|record| record.source_event_sequence() <= event_sequence)
        {
            match record {
                TranscriptRecord::TurnUpsert { turn, .. } => transcript.upsert_turn(turn),
                TranscriptRecord::SegmentUpsert { segment, .. } => {
                    transcript.upsert_segment(segment)
                }
                TranscriptRecord::SegmentTombstone { segment_id, .. } => {
                    transcript.remove_segment(&segment_id);
                }
            }
        }
        transcript.segments.sort_by_key(|segment| segment.order);
        Ok(transcript)
    }

    /// Only events after the last semantic commit need inspection on recovery.
    pub fn uncommitted_event_suffix(&self) -> Result<Vec<ActivityEvent>> {
        let committed = tail_record::<TranscriptRecord>(&self.transcript_path())?
            .map(|record| record.source_event_sequence())
            .unwrap_or(0);
        read_event_suffix(&self.events_path(), committed)
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, ActivityTail>> {
        self.append_lock
            .lock()
            .map_err(|_| StorageError::InvalidRepositoryPath {
                path: self.root.clone(),
                reason: "activity append lock poisoned".to_string(),
            })
    }
}

fn externalize_tool_output(root: &Path, record: &mut TranscriptRecord) -> Result<()> {
    let TranscriptRecord::SegmentUpsert { segment, .. } = record else {
        return Ok(());
    };
    externalize_segment_tool_output(root, segment)
}

fn read_lines<T: DeserializeOwned>(path: &Path) -> Result<Vec<T>> {
    read_activity_records(path, 0).map(|(records, _)| records)
}

/// Decode newline-terminated records from `offset` strictly. A final
/// unterminated record is returned when valid and ignored when invalid (an
/// interrupted append that the next append repairs). Returns the offset just
/// past the last record consumed. Blank lines are skipped only after a cursor,
/// where append may have inserted the newline of an already returned record.
fn read_activity_records<T: DeserializeOwned>(path: &Path, offset: u64) -> Result<(Vec<T>, u64)> {
    let mut file = match fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok((Vec::new(), 0)),
        Err(error) => return Err(activity_io("open", path, error)),
    };
    let length = file
        .metadata()
        .map_err(|error| activity_io("stat", path, error))?
        .len();
    let start = offset.min(length);
    file.seek(SeekFrom::Start(start))
        .map_err(|error| activity_io("seek", path, error))?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)
        .map_err(|error| activity_io("read", path, error))?;
    // The cursor comes from the bytes actually read, never a stale length:
    // an append racing this read is either fully consumed or left for later.
    let length = start + bytes.len() as u64;
    let terminated_len = bytes
        .iter()
        .rposition(|byte| *byte == b'\n')
        .map_or(0, |index| index + 1);
    let (terminated, tail) = bytes.split_at(terminated_len);
    let mut records = Vec::new();
    let lines = terminated
        .strip_suffix(b"\n")
        .map(|lines| lines.split(|byte| *byte == b'\n'));
    for (index, line) in lines.into_iter().flatten().enumerate() {
        if start > 0 && line.trim_ascii().is_empty() {
            continue;
        }
        records.push(
            serde_json::from_slice(line).map_err(|source| StorageError::JsonlLine {
                path: path.to_path_buf(),
                line: index + 1,
                source,
            })?,
        );
    }
    if tail.is_empty() {
        return Ok((records, length));
    }
    match serde_json::from_slice(tail) {
        Ok(record) => {
            records.push(record);
            Ok((records, length))
        }
        Err(_) => Ok((records, length - tail.len() as u64)),
    }
}

fn file_len(path: &Path) -> Result<u64> {
    if !path.exists() {
        return Ok(0);
    }
    fs::metadata(path)
        .map(|metadata| metadata.len())
        .map_err(|error| activity_io("stat", path, error))
}

fn append_activity_record<T: Serialize>(path: &Path, record: &T) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| activity_io("create directory", parent, error))?;
    }
    let mut bytes = serde_json::to_vec(record).map_err(|source| StorageError::JsonWrite {
        path: path.to_path_buf(),
        source,
    })?;
    bytes.push(b'\n');
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .and_then(|mut file| file.write_all(&bytes))
        .map_err(|error| activity_io("append", path, error))
}

/// Make an unterminated final record consistent with reads before appending:
/// a valid record gets its missing newline, an invalid one is truncated away.
pub(crate) fn repair_partial_tail<T: DeserializeOwned>(path: &Path) -> Result<()> {
    let mut file = match OpenOptions::new().read(true).write(true).open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(activity_io("open", path, error)),
    };
    let length = file
        .metadata()
        .map_err(|error| activity_io("stat", path, error))?
        .len();
    if length == 0 {
        return Ok(());
    }
    file.seek(SeekFrom::End(-1))
        .map_err(|error| activity_io("seek", path, error))?;
    let mut last = [0];
    file.read_exact(&mut last)
        .map_err(|error| activity_io("read", path, error))?;
    if last[0] == b'\n' {
        return Ok(());
    }
    let mut tail_start = 0;
    let mut end = length;
    while end > 0 {
        let start = end.saturating_sub(TAIL_CHUNK_BYTES);
        file.seek(SeekFrom::Start(start))
            .map_err(|error| activity_io("seek", path, error))?;
        let mut chunk = vec![0; (end - start) as usize];
        file.read_exact(&mut chunk)
            .map_err(|error| activity_io("read", path, error))?;
        if let Some(index) = chunk.iter().rposition(|byte| *byte == b'\n') {
            tail_start = start + index as u64 + 1;
            break;
        }
        end = start;
    }
    file.seek(SeekFrom::Start(tail_start))
        .map_err(|error| activity_io("seek", path, error))?;
    let mut tail = Vec::new();
    file.read_to_end(&mut tail)
        .map_err(|error| activity_io("read", path, error))?;
    if serde_json::from_slice::<T>(&tail).is_ok() {
        file.write_all(b"\n")
            .map_err(|error| activity_io("append", path, error))
    } else {
        file.set_len(tail_start)
            .map_err(|error| activity_io("truncate", path, error))
    }
}

fn tail_event_sequence(path: &Path) -> Result<u64> {
    Ok(tail_record::<ActivityEvent>(path)?
        .map(|record| record.sequence)
        .unwrap_or(0))
}

fn tail_transcript_revision(path: &Path) -> Result<u64> {
    Ok(tail_record::<TranscriptRecord>(path)?
        .map(|record| record.revision())
        .unwrap_or(0))
}

fn tail_record<T: DeserializeOwned>(path: &Path) -> Result<Option<T>> {
    if !path.exists() {
        return Ok(None);
    }
    let mut file = fs::File::open(path).map_err(|error| activity_io("open", path, error))?;
    let length = file
        .metadata()
        .map_err(|error| activity_io("stat", path, error))?
        .len();
    let mut end = length;
    let mut bytes = Vec::new();
    while end > 0 {
        let start = end.saturating_sub(TAIL_CHUNK_BYTES);
        file.seek(SeekFrom::Start(start))
            .map_err(|error| activity_io("seek", path, error))?;
        let mut chunk = vec![0; (end - start) as usize];
        file.read_exact(&mut chunk)
            .map_err(|error| activity_io("read", path, error))?;
        chunk.extend(bytes);
        bytes = chunk;
        if let Some(record) = activity_lines(path, &bytes, start > 0)?.pop() {
            return Ok(Some(record));
        }
        end = start;
    }
    Ok(None)
}

fn read_event_suffix(path: &Path, committed: u64) -> Result<Vec<ActivityEvent>> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let mut file = fs::File::open(path).map_err(|error| activity_io("open", path, error))?;
    let mut end = file
        .metadata()
        .map_err(|error| activity_io("stat", path, error))?
        .len();
    let mut bytes = Vec::new();
    while end > 0 {
        let start = end.saturating_sub(TAIL_CHUNK_BYTES);
        file.seek(SeekFrom::Start(start))
            .map_err(|error| activity_io("seek", path, error))?;
        let mut chunk = vec![0; (end - start) as usize];
        file.read_exact(&mut chunk)
            .map_err(|error| activity_io("read", path, error))?;
        chunk.extend(bytes);
        bytes = chunk;
        let records: Vec<ActivityEvent> = activity_lines(path, &bytes, start > 0)?
            .into_iter()
            .filter(|record: &ActivityEvent| record.sequence > committed)
            .collect();
        if records
            .first()
            .is_some_and(|record| record.sequence == committed + 1)
            || start == 0
        {
            return Ok(records);
        }
        end = start;
    }
    Ok(Vec::new())
}

/// Decode a window that ends at end of file, skipping its first (partial)
/// line when the window starts mid-file. Like full reads, malformed
/// terminated records are errors and only an invalid unterminated final
/// record is ignored.
fn activity_lines<T: DeserializeOwned>(
    path: &Path,
    bytes: &[u8],
    skip_first: bool,
) -> Result<Vec<T>> {
    let mut lines: Vec<&[u8]> = bytes.split(|byte| *byte == b'\n').collect();
    let tail = lines.pop().unwrap_or_default();
    let mut records = Vec::new();
    for line in lines.into_iter().skip(usize::from(skip_first)) {
        if line.is_empty() {
            continue;
        }
        records.push(
            serde_json::from_slice(line).map_err(|source| StorageError::JsonRead {
                path: path.to_path_buf(),
                source,
            })?,
        );
    }
    // Without a newline, a mid-file window is all one partial first line.
    if !skip_first || bytes.contains(&b'\n') {
        if let Ok(record) = serde_json::from_slice(tail) {
            records.push(record);
        }
    }
    Ok(records)
}

struct ActivityFileLock(fs::File);

impl ActivityFileLock {
    fn acquire(root: &Path) -> Result<Self> {
        fs::create_dir_all(root).map_err(|error| activity_io("create directory", root, error))?;
        let path = root.join(ACTIVITY_LOCK_FILE_NAME);
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .open(&path)
            .map_err(|error| activity_io("open lock", &path, error))?;
        file.lock_exclusive()
            .map_err(|error| activity_io("acquire lock", &path, error))?;
        Ok(Self(file))
    }
}

impl Drop for ActivityFileLock {
    fn drop(&mut self) {
        let _ = self.0.unlock();
    }
}

fn activity_io(
    action: &'static str,
    path: impl Into<PathBuf>,
    source: std::io::Error,
) -> StorageError {
    StorageError::Io {
        action,
        path: path.into(),
        source,
    }
}
