//! A project's mission and standing-role records: JSON payload rows in the
//! state database, keyed by project and record ID. Their inboxes live in the
//! conversation tables.
use crate::conversation::{open_state_database, STATE_DATABASE_FILE_NAME};
use crate::StorageError;
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

pub struct RecordRepository {
    home: PathBuf,
    project_id: String,
    table: &'static str,
    key: &'static str,
}
impl RecordRepository {
    pub fn missions(home: &Path, project_id: &str) -> Self {
        Self::new(home, project_id, "missions", "mission_id")
    }
    pub fn roles(home: &Path, project_id: &str) -> Self {
        Self::new(home, project_id, "standing_roles", "name")
    }
    fn new(home: &Path, project_id: &str, table: &'static str, key: &'static str) -> Self {
        Self {
            home: home.to_path_buf(),
            project_id: project_id.to_string(),
            table,
            key,
        }
    }
    /// Record ids are mission ids and role file names alike.
    fn checked(&self, id: &str) -> Result<(), StorageError> {
        if !crate::conversation::is_safe_segment_file_id(id) {
            return Err(StorageError::InvalidRepositoryPath {
                path: PathBuf::from(id),
                reason: "Invalid record ID".into(),
            });
        }
        Ok(())
    }
    fn connection(&self) -> Result<Connection, StorageError> {
        open_state_database(&self.home)
    }
    fn read_in(&self, connection: &Connection, id: &str) -> Result<Option<Value>, StorageError> {
        self.checked(id)?;
        let payload: Option<String> = connection
            .query_row(
                &format!(
                    "SELECT payload FROM {} WHERE project_id = ?1 AND {} = ?2",
                    self.table, self.key
                ),
                params![self.project_id, id],
                |row| row.get(0),
            )
            .optional()?;
        payload.map(|text| self.parse(&text)).transpose()
    }
    fn write_in(
        &self,
        connection: &Connection,
        id: &str,
        value: &Value,
    ) -> Result<(), StorageError> {
        self.checked(id)?;
        connection.execute(
            &format!(
                "INSERT INTO {} (project_id, {}, payload) VALUES (?1, ?2, ?3) \
                 ON CONFLICT DO UPDATE SET payload = excluded.payload",
                self.table, self.key
            ),
            params![self.project_id, id, value.to_string()],
        )?;
        Ok(())
    }
    /// A stored payload that is not JSON is corruption, never an absent record.
    fn parse(&self, text: &str) -> Result<Value, StorageError> {
        serde_json::from_str(text).map_err(|source| StorageError::JsonRead {
            path: self.home.join(STATE_DATABASE_FILE_NAME),
            source,
        })
    }
    pub fn read(&self, id: &str) -> Result<Option<Value>, StorageError> {
        self.read_in(&self.connection()?, id)
    }
    pub fn list(&self) -> Result<Vec<Value>, StorageError> {
        let connection = self.connection()?;
        let mut statement = connection.prepare(&format!(
            "SELECT payload FROM {} WHERE project_id = ?1 ORDER BY {}",
            self.table, self.key
        ))?;
        let payloads = statement
            .query_map([&self.project_id], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        payloads.iter().map(|text| self.parse(text)).collect()
    }
    pub fn write(&self, id: &str, value: &Value) -> Result<(), StorageError> {
        let mut connection = self.connection()?;
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        self.write_in(&tx, id, value)?;
        tx.commit()?;
        Ok(())
    }
    /// Runs `work` while holding this process's lock for the project's
    /// records of this kind, so composite operations (which also write the
    /// conversation tables, and so cannot hold one transaction) serialize.
    pub fn locked<T, E: From<StorageError>>(
        &self,
        work: impl FnOnce(&Self) -> Result<T, E>,
    ) -> Result<T, E> {
        // ponytail: one in-process lock per project and kind; only the server
        // process writes records. Use per-record locks if throughput requires it.
        static LOCKS: OnceLock<Mutex<HashMap<String, Arc<Mutex<()>>>>> = OnceLock::new();
        let key = format!(
            "{}\0{}\0{}",
            self.home.display(),
            self.table,
            self.project_id
        );
        let lock = LOCKS
            .get_or_init(Default::default)
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .entry(key)
            .or_default()
            .clone();
        let _guard = lock.lock().unwrap_or_else(|poison| poison.into_inner());
        work(self)
    }
}

/// The global workflow milestone log and each run's projection progress.
/// Milestones belong to no project, so project removal keeps them.
pub struct WorkflowMilestones {
    home: PathBuf,
}
impl WorkflowMilestones {
    pub fn new(home: &Path) -> Self {
        Self {
            home: home.to_path_buf(),
        }
    }
    fn parse(&self, text: &str) -> Result<Value, StorageError> {
        serde_json::from_str(text).map_err(|source| StorageError::JsonRead {
            path: self.home.join(STATE_DATABASE_FILE_NAME),
            source,
        })
    }
    /// In one immediate transaction, hands `project` the run's stored
    /// progress and records the `(milestone id, entry)` pairs and next
    /// progress it returns. Each entry gets the next sequence number as its
    /// `seq`; an id already logged is skipped, so a retry never duplicates a
    /// milestone or advances progress without it. Returns the new entries.
    pub fn record<E: From<StorageError>>(
        &self,
        run_id: &str,
        project: impl FnOnce(Option<Value>) -> Result<(Vec<(String, Value)>, Value), E>,
    ) -> Result<Vec<Value>, E> {
        let mut connection = open_state_database(&self.home)?;
        let tx = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(StorageError::from)?;
        let progress: Option<String> = tx
            .query_row(
                "SELECT state FROM workflow_projection WHERE run_id = ?1",
                [run_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(StorageError::from)?;
        let progress = progress.map(|text| self.parse(&text)).transpose()?;
        let (milestones, next) = project(progress.clone())?;
        if milestones.is_empty() && progress.as_ref() == Some(&next) {
            return Ok(Vec::new());
        }
        let mut recorded = Vec::new();
        for (id, mut entry) in milestones {
            let seq: i64 = tx
                .query_row(
                    "SELECT coalesce(max(seq) + 1, 0) FROM workflow_milestones",
                    [],
                    |row| row.get(0),
                )
                .map_err(StorageError::from)?;
            entry["seq"] = Value::from(seq);
            let inserted = tx
                .execute(
                    "INSERT INTO workflow_milestones (seq, milestone_id, entry) VALUES (?1, ?2, ?3) \
                     ON CONFLICT (milestone_id) DO NOTHING",
                    params![seq, id, entry.to_string()],
                )
                .map_err(StorageError::from)?;
            if inserted == 1 {
                recorded.push(entry);
            }
        }
        tx.execute(
            "INSERT INTO workflow_projection (run_id, state) VALUES (?1, ?2) \
             ON CONFLICT DO UPDATE SET state = excluded.state",
            params![run_id, next.to_string()],
        )
        .map_err(StorageError::from)?;
        tx.commit().map_err(StorageError::from)?;
        Ok(recorded)
    }
    /// The newest `limit` entries, oldest first.
    pub fn tail(&self, limit: usize) -> Result<Vec<Value>, StorageError> {
        let connection = open_state_database(&self.home)?;
        let mut statement = connection.prepare(
            "SELECT entry FROM (SELECT seq, entry FROM workflow_milestones \
             ORDER BY seq DESC LIMIT ?1) ORDER BY seq",
        )?;
        let entries = statement
            .query_map([limit as i64], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        entries.iter().map(|text| self.parse(text)).collect()
    }
}
