//! A Spark home among environments: its identity, the peer tokens and
//! pairing codes it issues, its links to other homes, and the delegations it
//! sends (`delegations_out`) and admits (`delegations_in`). Rows live in the
//! state database and use short-lived connections, like the other private
//! records.
use std::io::Write as _;
use std::path::{Path, PathBuf};

use rusqlite::{params, Connection, OptionalExtension, Row, TransactionBehavior};
use serde::Serialize;
use serde_json::Value;

use crate::conversation::{open_state_database, STATE_DATABASE_FILE_NAME};
use crate::error::{Result, StorageError};

/// The home's environment ID, written once.
pub const ENVIRONMENT_ID_FILE_NAME: &str = "environment_id";
/// Peers require an exact match.
pub const PROTOCOL_VERSION: u32 = 1;
/// Frontend↔backend HTTP/SSE contract version for remote UI views; bump on
/// breaking workspace/attractor API or SSE envelope changes.
pub const UI_API_VERSION: u32 = 2;
/// How long a pairing code stays valid.
pub const PAIRING_CODE_TTL: time::Duration = time::Duration::minutes(10);

/// The home's environment ID, created on first use by exclusive creation of a
/// complete file. The caller holds the home lock. A file that is not a UUID
/// fails rather than being replaced.
pub fn ensure_environment_id(home: &Path) -> Result<String> {
    let path = home.join(ENVIRONMENT_ID_FILE_NAME);
    if !path.exists() {
        let mut temp = tempfile::NamedTempFile::new_in(home)
            .map_err(|source| StorageError::io("create", home, source))?;
        temp.write_all(uuid::Uuid::new_v4().to_string().as_bytes())
            .and_then(|()| temp.as_file().sync_all())
            .map_err(|source| StorageError::io("write", temp.path().to_path_buf(), source))?;
        // An existing file wins: it is read and checked below.
        if let Err(error) = temp.persist_noclobber(&path) {
            if error.error.kind() != std::io::ErrorKind::AlreadyExists {
                return Err(StorageError::io("create", &path, error.error));
            }
        }
    }
    let text =
        std::fs::read_to_string(&path).map_err(|source| StorageError::io("read", &path, source))?;
    uuid::Uuid::parse_str(text.trim())
        .map(|id| id.to_string())
        .map_err(|_| StorageError::InvalidConversationState {
            path: path.clone(),
            reason: format!(
                "The environment ID file {} is malformed; it must hold one UUID.",
                path.display()
            ),
        })
}

/// A token this home issued. Its secret is never stored or listed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PeerToken {
    pub token_id: String,
    pub label: String,
    pub created_at: String,
    pub revoked_at: Option<String>,
}

/// A saved link to another environment. Its token is not part of it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EnvironmentLink {
    pub environment_id: String,
    pub origin: String,
    pub label: String,
    pub created_at: String,
}

/// A delegation this home sent.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DelegationOut {
    pub delegation_id: String,
    pub environment_id: String,
    pub project_path: String,
    pub kind: String,
    pub request: Value,
    pub request_sha256: String,
    pub caller: String,
    pub request_key: String,
    pub owner_conversation_id: Option<String>,
    pub phase: String,
    pub target_id: Option<String>,
    pub last_state: Option<Value>,
    pub result: Option<Value>,
    pub paused: Option<String>,
    pub cancel_requested_at: Option<String>,
    pub cancel_acknowledged_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

/// A delegation this home admitted, or a cancellation tombstone.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DelegationIn {
    pub delegation_id: String,
    pub origin_environment_id: String,
    pub project_path: String,
    pub kind: String,
    pub request: Value,
    pub request_sha256: String,
    pub target_kind: String,
    pub target_id: String,
    pub phase: String,
    pub result: Option<Value>,
    pub created_at: String,
    pub sealed_at: Option<String>,
    /// When a cancel reached this home.
    pub canceled_at: Option<String>,
}

/// A fresh secret: 32 random bytes, hex encoded.
fn secret() -> String {
    let bytes: [u8; 32] = rand::random();
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn hash(secret: &str) -> String {
    spark_common::project::sha256_hex(secret.as_bytes())
}

/// A fixed-width UTC time `offset` from now, so times compare as text.
fn utc(offset: time::Duration) -> String {
    let at = time::OffsetDateTime::now_utc() + offset;
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        at.year(),
        u8::from(at.month()),
        at.day(),
        at.hour(),
        at.minute(),
        at.second()
    )
}

/// Now, in the fixed-width form these tables store.
pub fn now() -> String {
    utc(time::Duration::ZERO)
}

const OUT_COLUMNS: &str = "delegation_id, environment_id, project_path, kind, request, \
    request_sha256, caller, request_key, owner_conversation_id, phase, target_id, last_state, \
    result, paused, cancel_requested_at, cancel_acknowledged_at, created_at, updated_at";
const IN_COLUMNS: &str = "delegation_id, origin_environment_id, project_path, kind, request, \
    request_sha256, target_kind, target_id, phase, result, created_at, sealed_at, canceled_at";

fn json_column(row: &Row<'_>, index: usize) -> rusqlite::Result<Option<Value>> {
    Ok(row
        .get::<_, Option<String>>(index)?
        .and_then(|text| serde_json::from_str(&text).ok()))
}

fn out_row(row: &Row<'_>) -> rusqlite::Result<DelegationOut> {
    Ok(DelegationOut {
        delegation_id: row.get(0)?,
        environment_id: row.get(1)?,
        project_path: row.get(2)?,
        kind: row.get(3)?,
        request: json_column(row, 4)?.unwrap_or_default(),
        request_sha256: row.get(5)?,
        caller: row.get(6)?,
        request_key: row.get(7)?,
        owner_conversation_id: row.get(8)?,
        phase: row.get(9)?,
        target_id: row.get(10)?,
        last_state: json_column(row, 11)?,
        result: json_column(row, 12)?,
        paused: row.get(13)?,
        cancel_requested_at: row.get(14)?,
        cancel_acknowledged_at: row.get(15)?,
        created_at: row.get(16)?,
        updated_at: row.get(17)?,
    })
}

fn in_row(row: &Row<'_>) -> rusqlite::Result<DelegationIn> {
    Ok(DelegationIn {
        delegation_id: row.get(0)?,
        origin_environment_id: row.get(1)?,
        project_path: row.get(2)?,
        kind: row.get(3)?,
        request: json_column(row, 4)?.unwrap_or_default(),
        request_sha256: row.get(5)?,
        target_kind: row.get(6)?,
        target_id: row.get(7)?,
        phase: row.get(8)?,
        result: json_column(row, 9)?,
        created_at: row.get(10)?,
        sealed_at: row.get(11)?,
        canceled_at: row.get(12)?,
    })
}

/// Seals an outgoing delegation on `connection`, inside the caller's
/// transaction, so the seal and its inbox event commit together. A sealed
/// record keeps its first result.
pub fn seal_out_on(connection: &Connection, delegation_id: &str, result: &Value) -> Result<()> {
    connection.execute(
        "UPDATE delegations_out SET phase = 'sealed', result = ?2, paused = NULL, \
         updated_at = ?3 WHERE delegation_id = ?1 AND phase != 'sealed'",
        params![delegation_id, result.to_string(), now()],
    )?;
    Ok(())
}

/// The environment tables of one Spark home.
pub struct EnvironmentRepository {
    home: PathBuf,
}

impl EnvironmentRepository {
    pub fn new(home: impl Into<PathBuf>) -> Self {
        Self { home: home.into() }
    }

    fn connection(&self) -> Result<Connection> {
        open_state_database(&self.home)
    }

    fn database_path(&self) -> PathBuf {
        self.home.join(STATE_DATABASE_FILE_NAME)
    }

    // ---- Pairing and tokens ----------------------------------------------

    /// Stores a new single-use pairing code and returns it with its expiry.
    pub fn create_pairing_code(&self, label: &str) -> Result<(String, String)> {
        let code = secret();
        let expires_at = utc(PAIRING_CODE_TTL);
        self.connection()?.execute(
            "INSERT INTO pairing_codes (code_sha256, label, expires_at) VALUES (?1, ?2, ?3)",
            params![hash(&code), label, expires_at],
        )?;
        Ok((code, expires_at))
    }

    /// Consumes an unexpired, unused code and issues a token in one
    /// transaction. Returns the token and its secret, or `None` for a code
    /// that is unknown, used or expired.
    pub fn pair(&self, code: &str, label: &str) -> Result<Option<(PeerToken, String)>> {
        let mut connection = self.connection()?;
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let now = now();
        let code_label: Option<String> = tx
            .query_row(
                "UPDATE pairing_codes SET consumed_at = ?2 WHERE code_sha256 = ?1 \
                 AND consumed_at IS NULL AND expires_at > ?2 RETURNING label",
                params![hash(code), now],
                |row| row.get(0),
            )
            .optional()?;
        let Some(code_label) = code_label else {
            return Ok(None);
        };
        let token = PeerToken {
            token_id: uuid::Uuid::new_v4().to_string(),
            label: if code_label.trim().is_empty() {
                label.trim().to_string()
            } else {
                code_label
            },
            created_at: now,
            revoked_at: None,
        };
        let secret = secret();
        tx.execute(
            "INSERT INTO peer_tokens (token_id, label, token_sha256, created_at) \
             VALUES (?1, ?2, ?3, ?4)",
            params![token.token_id, token.label, hash(&secret), token.created_at],
        )?;
        tx.commit()?;
        Ok(Some((token, secret)))
    }

    pub fn tokens(&self) -> Result<Vec<PeerToken>> {
        let connection = self.connection()?;
        let mut statement = connection.prepare(
            "SELECT token_id, label, created_at, revoked_at FROM peer_tokens ORDER BY created_at",
        )?;
        let tokens = statement
            .query_map([], |row| {
                Ok(PeerToken {
                    token_id: row.get(0)?,
                    label: row.get(1)?,
                    created_at: row.get(2)?,
                    revoked_at: row.get(3)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(tokens)
    }

    /// Returns whether a live token was revoked.
    pub fn revoke_token(&self, token_id: &str) -> Result<bool> {
        Ok(self.connection()?.execute(
            "UPDATE peer_tokens SET revoked_at = ?2 WHERE token_id = ?1 AND revoked_at IS NULL",
            params![token_id, now()],
        )? > 0)
    }

    /// The ID of the unrevoked token whose secret is `secret`.
    pub fn token_for_secret(&self, secret: &str) -> Result<Option<String>> {
        Ok(self
            .connection()?
            .query_row(
                "SELECT token_id FROM peer_tokens WHERE token_sha256 = ?1 AND revoked_at IS NULL",
                [hash(secret)],
                |row| row.get(0),
            )
            .optional()?)
    }

    // ---- Links -----------------------------------------------------------

    /// Saves the link to `link.environment_id`, replacing any earlier one,
    /// and resumes that environment's paused delegations.
    pub fn save_link(&self, link: &EnvironmentLink, token: &str) -> Result<()> {
        let mut connection = self.connection()?;
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute(
            "INSERT INTO environment_links (environment_id, origin, label, token, created_at) \
             VALUES (?1, ?2, ?3, ?4, ?5) ON CONFLICT (environment_id) DO UPDATE SET \
             origin = excluded.origin, label = excluded.label, token = excluded.token, \
             created_at = excluded.created_at",
            params![
                link.environment_id,
                link.origin,
                link.label,
                token,
                link.created_at
            ],
        )?;
        tx.execute(
            "UPDATE delegations_out SET paused = NULL, updated_at = ?2 \
             WHERE environment_id = ?1 AND phase != 'sealed' AND paused IS NOT NULL",
            params![link.environment_id, now()],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn links(&self) -> Result<Vec<EnvironmentLink>> {
        let connection = self.connection()?;
        let mut statement = connection.prepare(
            "SELECT environment_id, origin, label, created_at FROM environment_links \
             ORDER BY created_at",
        )?;
        let links = statement
            .query_map([], |row| {
                Ok(EnvironmentLink {
                    environment_id: row.get(0)?,
                    origin: row.get(1)?,
                    label: row.get(2)?,
                    created_at: row.get(3)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(links)
    }

    /// The link to `environment_id` and the token it presents.
    pub fn link(&self, environment_id: &str) -> Result<Option<(EnvironmentLink, String)>> {
        Ok(self
            .connection()?
            .query_row(
                "SELECT environment_id, origin, label, created_at, token FROM environment_links \
                 WHERE environment_id = ?1",
                [environment_id],
                |row| {
                    Ok((
                        EnvironmentLink {
                            environment_id: row.get(0)?,
                            origin: row.get(1)?,
                            label: row.get(2)?,
                            created_at: row.get(3)?,
                        },
                        row.get(4)?,
                    ))
                },
            )
            .optional()?)
    }

    /// Deletes the link and its token, pausing the environment's open
    /// delegations. Returns whether a link existed.
    pub fn remove_link(&self, environment_id: &str) -> Result<bool> {
        let mut connection = self.connection()?;
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let removed = tx.execute(
            "DELETE FROM environment_links WHERE environment_id = ?1",
            [environment_id],
        )? > 0;
        tx.execute(
            "UPDATE delegations_out SET paused = 'The link to this environment was removed.', \
             updated_at = ?2 WHERE environment_id = ?1 AND phase != 'sealed' AND paused IS NULL",
            params![environment_id, now()],
        )?;
        tx.commit()?;
        Ok(removed)
    }

    // ---- Outgoing delegations ----------------------------------------------

    /// Records `record` unless its caller already used its request key, and
    /// returns the record stored under that key: the new one or the earlier.
    pub fn create_out(&self, record: &DelegationOut) -> Result<DelegationOut> {
        let mut connection = self.connection()?;
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute(
            &format!(
                "INSERT INTO delegations_out ({OUT_COLUMNS}) VALUES \
                 (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 'created', NULL, NULL, NULL, NULL, NULL, \
                 NULL, ?10, ?10) ON CONFLICT (caller, request_key) DO NOTHING"
            ),
            params![
                record.delegation_id,
                record.environment_id,
                record.project_path,
                record.kind,
                record.request.to_string(),
                record.request_sha256,
                record.caller,
                record.request_key,
                record.owner_conversation_id,
                now(),
            ],
        )?;
        let stored = tx.query_row(
            &format!(
                "SELECT {OUT_COLUMNS} FROM delegations_out WHERE caller = ?1 AND request_key = ?2"
            ),
            params![record.caller, record.request_key],
            out_row,
        )?;
        tx.commit()?;
        Ok(stored)
    }

    /// The record `caller` keeps under `request_key`, if any.
    pub fn out_by_key(&self, caller: &str, request_key: &str) -> Result<Option<DelegationOut>> {
        Ok(self
            .connection()?
            .query_row(
                &format!(
                    "SELECT {OUT_COLUMNS} FROM delegations_out WHERE caller = ?1 \
                     AND request_key = ?2"
                ),
                [caller, request_key],
                out_row,
            )
            .optional()?)
    }

    pub fn out(&self, delegation_id: &str) -> Result<Option<DelegationOut>> {
        Ok(self
            .connection()?
            .query_row(
                &format!("SELECT {OUT_COLUMNS} FROM delegations_out WHERE delegation_id = ?1"),
                [delegation_id],
                out_row,
            )
            .optional()?)
    }

    /// Every outgoing delegation, newest first.
    pub fn list_out(&self) -> Result<Vec<DelegationOut>> {
        let connection = self.connection()?;
        let mut statement = connection.prepare(&format!(
            "SELECT {OUT_COLUMNS} FROM delegations_out ORDER BY created_at DESC, delegation_id DESC"
        ))?;
        let records = statement
            .query_map([], out_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(records)
    }

    /// Unsealed records that are not paused, oldest first.
    pub fn followed_out(&self) -> Result<Vec<DelegationOut>> {
        let connection = self.connection()?;
        let mut statement = connection.prepare(&format!(
            "SELECT {OUT_COLUMNS} FROM delegations_out WHERE phase != 'sealed' \
             AND paused IS NULL ORDER BY created_at, delegation_id"
        ))?;
        let records = statement
            .query_map([], out_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(records)
    }

    /// Runs one update with `?1` the delegation, `?2` the value and `?3`
    /// now; returns whether a row changed.
    fn update_out(&self, sql: &str, delegation_id: &str, value: &str) -> Result<bool> {
        Ok(self
            .connection()?
            .execute(sql, params![delegation_id, value, now()])?
            > 0)
    }

    /// B acknowledged the record under `target_id`.
    pub fn accept_out(&self, delegation_id: &str, target_id: &str) -> Result<bool> {
        self.update_out(
            "UPDATE delegations_out SET phase = 'accepted', target_id = ?2, updated_at = ?3 \
             WHERE delegation_id = ?1 AND phase = 'created'",
            delegation_id,
            target_id,
        )
    }

    /// Stores B's latest view; returns whether it changed.
    pub fn set_out_state(&self, delegation_id: &str, state: &Value) -> Result<bool> {
        self.update_out(
            "UPDATE delegations_out SET last_state = ?2, updated_at = ?3 \
             WHERE delegation_id = ?1 AND phase != 'sealed' AND last_state IS NOT ?2",
            delegation_id,
            &state.to_string(),
        )
    }

    pub fn pause_out(&self, delegation_id: &str, reason: &str) -> Result<bool> {
        self.update_out(
            "UPDATE delegations_out SET paused = ?2, updated_at = ?3 \
             WHERE delegation_id = ?1 AND phase != 'sealed'",
            delegation_id,
            reason,
        )
    }

    /// Records the owner's cancellation once.
    pub fn request_cancel_out(&self, delegation_id: &str) -> Result<bool> {
        Ok(self.connection()?.execute(
            "UPDATE delegations_out SET cancel_requested_at = ?2, updated_at = ?2 \
             WHERE delegation_id = ?1 AND phase != 'sealed' AND cancel_requested_at IS NULL",
            params![delegation_id, now()],
        )? > 0)
    }

    pub fn acknowledge_cancel_out(&self, delegation_id: &str) -> Result<bool> {
        Ok(self.connection()?.execute(
            "UPDATE delegations_out SET cancel_acknowledged_at = ?2, updated_at = ?2 \
             WHERE delegation_id = ?1 AND cancel_acknowledged_at IS NULL",
            params![delegation_id, now()],
        )? > 0)
    }

    /// Seals a record that has no owner conversation to tell.
    pub fn seal_out(&self, delegation_id: &str, result: &Value) -> Result<()> {
        seal_out_on(&self.connection()?, delegation_id, result)
    }

    // ---- Incoming delegations ----------------------------------------------

    /// Inserts `record` unless its ID exists, and returns the stored row:
    /// the new one, an earlier admission, or a tombstone.
    pub fn admit_in(&self, record: &DelegationIn) -> Result<DelegationIn> {
        let connection = self.connection()?;
        connection.execute(
            &format!(
                "INSERT INTO delegations_in ({IN_COLUMNS}) VALUES \
                 (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13) \
                 ON CONFLICT (delegation_id) DO NOTHING"
            ),
            params![
                record.delegation_id,
                record.origin_environment_id,
                record.project_path,
                record.kind,
                record.request.to_string(),
                record.request_sha256,
                record.target_kind,
                record.target_id,
                record.phase,
                record.result.as_ref().map(Value::to_string),
                record.created_at,
                record.sealed_at,
                record.canceled_at,
            ],
        )?;
        self.in_(&record.delegation_id)?
            .ok_or_else(|| StorageError::InvalidConversationState {
                path: self.database_path(),
                reason: format!("Delegation {} was not stored.", record.delegation_id),
            })
    }

    pub fn in_(&self, delegation_id: &str) -> Result<Option<DelegationIn>> {
        Ok(self
            .connection()?
            .query_row(
                &format!("SELECT {IN_COLUMNS} FROM delegations_in WHERE delegation_id = ?1"),
                [delegation_id],
                in_row,
            )
            .optional()?)
    }

    /// Admitted rows whose target was never launched.
    pub fn accepted_in(&self) -> Result<Vec<DelegationIn>> {
        let connection = self.connection()?;
        let mut statement = connection.prepare(&format!(
            "SELECT {IN_COLUMNS} FROM delegations_in WHERE phase = 'accepted' ORDER BY created_at"
        ))?;
        let records = statement
            .query_map([], in_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(records)
    }

    pub fn mark_in_launched(&self, delegation_id: &str) -> Result<bool> {
        Ok(self.connection()?.execute(
            "UPDATE delegations_in SET phase = 'launched' WHERE delegation_id = ?1 \
             AND phase = 'accepted'",
            [delegation_id],
        )? > 0)
    }

    /// Seals the row once and returns it; an earlier seal stands.
    pub fn seal_in(&self, delegation_id: &str, result: &Value) -> Result<Option<DelegationIn>> {
        self.connection()?.execute(
            "UPDATE delegations_in SET phase = 'sealed', result = ?2, sealed_at = ?3 \
             WHERE delegation_id = ?1 AND phase != 'sealed'",
            params![delegation_id, result.to_string(), now()],
        )?;
        self.in_(delegation_id)
    }

    /// Records the first cancel that reaches an unsealed row.
    pub fn cancel_in(&self, delegation_id: &str) -> Result<bool> {
        Ok(self.connection()?.execute(
            "UPDATE delegations_in SET canceled_at = ?2 WHERE delegation_id = ?1 \
             AND phase != 'sealed' AND canceled_at IS NULL",
            params![delegation_id, now()],
        )? > 0)
    }

    /// A cancellation for an ID never admitted: a sealed `canceled` row, so
    /// a delayed start returns it without launching. Returns the stored row.
    pub fn tombstone_in(
        &self,
        delegation_id: &str,
        origin_environment_id: &str,
    ) -> Result<DelegationIn> {
        let now = now();
        self.admit_in(&DelegationIn {
            delegation_id: delegation_id.to_string(),
            origin_environment_id: origin_environment_id.to_string(),
            project_path: String::new(),
            kind: String::new(),
            request: Value::Object(Default::default()),
            request_sha256: String::new(),
            target_kind: String::new(),
            target_id: String::new(),
            phase: "sealed".to_string(),
            result: Some(serde_json::json!({"status": "canceled"})),
            created_at: now.clone(),
            sealed_at: Some(now),
            canceled_at: None,
        })
    }
}
