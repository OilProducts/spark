use std::fs;
use std::path::Path;

use spark_common::project::sha256_hex;
use spark_storage::environment::{
    ensure_environment_id, DelegationOut, EnvironmentLink, EnvironmentRepository,
    ENVIRONMENT_ID_FILE_NAME, PAIRING_CODE_TTL,
};
use spark_storage::ConversationStore;

fn state_database(home: &Path) -> rusqlite::Connection {
    rusqlite::Connection::open(home.join("state.sqlite3")).expect("state database")
}

#[test]
fn environment_id_is_written_once_and_survives_reopen() {
    let temp = tempfile::tempdir().expect("tempdir");
    let home = temp.path().join("spark-home");

    let first = ConversationStore::open(&home)
        .expect("store")
        .environment_id()
        .to_string();
    uuid::Uuid::parse_str(&first).expect("environment ID is a UUID");
    assert_eq!(
        fs::read_to_string(home.join(ENVIRONMENT_ID_FILE_NAME)).expect("id file"),
        first
    );

    let reopened = ConversationStore::open(&home).expect("reopen");
    assert_eq!(reopened.environment_id(), first);
    drop(reopened);
    assert_eq!(ensure_environment_id(&home).expect("id"), first);
}

#[test]
fn malformed_environment_id_fails_open_and_is_kept() {
    let temp = tempfile::tempdir().expect("tempdir");
    let home = temp.path().join("spark-home");
    fs::create_dir_all(&home).expect("home");
    let path = home.join(ENVIRONMENT_ID_FILE_NAME);
    fs::write(&path, "not-a-uuid\n").expect("malformed id");

    let error = ConversationStore::open(&home)
        .err()
        .expect("a malformed ID fails startup");
    assert!(error.to_string().contains("malformed"), "{error}");
    assert!(ensure_environment_id(&home).is_err());
    assert_eq!(fs::read_to_string(&path).expect("id file"), "not-a-uuid\n");
}

#[test]
fn pairing_code_is_single_use_and_stored_only_as_a_hash() {
    let temp = tempfile::tempdir().expect("tempdir");
    let home = temp.path().join("spark-home");
    let _store = ConversationStore::open(&home).expect("store");
    let repository = EnvironmentRepository::new(&home);

    let (code, expires_at) = repository.create_pairing_code("laptop").expect("code");
    // Fixed-width UTC text, ten minutes ahead.
    assert_eq!(expires_at.len(), "YYYY-MM-DDTHH:MM:SSZ".len());
    let expires =
        time::OffsetDateTime::parse(&expires_at, &time::format_description::well_known::Rfc3339)
            .expect("expiry time");
    let ahead = expires - time::OffsetDateTime::now_utc();
    assert!(
        ahead <= PAIRING_CODE_TTL && ahead > PAIRING_CODE_TTL - time::Duration::seconds(30),
        "{ahead}"
    );
    let stored_code: String = state_database(&home)
        .query_row("SELECT code_sha256 FROM pairing_codes", [], |row| {
            row.get(0)
        })
        .expect("stored code");
    assert_ne!(stored_code, code);
    assert_eq!(stored_code, sha256_hex(code.as_bytes()));

    assert!(repository
        .pair("wrong-code", "peer")
        .expect("pair")
        .is_none());
    let (token, secret) = repository
        .pair(&code, "ignored")
        .expect("pair")
        .expect("first use issues a token");
    // The code's label wins over the one the peer sends.
    assert_eq!(token.label, "laptop");
    assert!(repository.pair(&code, "again").expect("pair").is_none());

    // A code without a label takes the peer's.
    let (unlabeled, _) = repository.create_pairing_code("").expect("code");
    let (peer_labeled, _) = repository
        .pair(&unlabeled, " desk ")
        .expect("pair")
        .expect("token");
    assert_eq!(peer_labeled.label, "desk");

    let stored_token: String = state_database(&home)
        .query_row(
            "SELECT token_sha256 FROM peer_tokens WHERE token_id = ?1",
            [&token.token_id],
            |row| row.get(0),
        )
        .expect("stored token");
    assert_ne!(stored_token, secret);
    assert_eq!(stored_token, sha256_hex(secret.as_bytes()));
    let listed = serde_json::to_string(&repository.tokens().expect("tokens")).expect("json");
    assert!(listed.contains(&token.token_id));
    assert!(!listed.contains(&secret));
    assert!(!listed.contains(&stored_token));
    assert_eq!(
        repository.token_for_secret(&secret).expect("lookup"),
        Some(token.token_id)
    );
}

#[test]
fn expired_pairing_code_issues_nothing() {
    let temp = tempfile::tempdir().expect("tempdir");
    let home = temp.path().join("spark-home");
    let _store = ConversationStore::open(&home).expect("store");
    let repository = EnvironmentRepository::new(&home);
    let (expired, _) = repository.create_pairing_code("old").expect("code");
    let (fresh, _) = repository.create_pairing_code("new").expect("code");

    state_database(&home)
        .execute(
            "UPDATE pairing_codes SET expires_at = '2000-01-01T00:00:00Z' \
             WHERE code_sha256 = ?1",
            [sha256_hex(expired.as_bytes())],
        )
        .expect("expire code");

    assert!(repository.pair(&expired, "peer").expect("pair").is_none());
    assert!(repository.tokens().expect("tokens").is_empty());
    assert!(repository.pair(&fresh, "peer").expect("pair").is_some());
}

#[test]
fn revoked_token_no_longer_authenticates() {
    let temp = tempfile::tempdir().expect("tempdir");
    let home = temp.path().join("spark-home");
    let _store = ConversationStore::open(&home).expect("store");
    let repository = EnvironmentRepository::new(&home);
    let issue = |label: &str| {
        let (code, _) = repository.create_pairing_code(label).expect("code");
        repository.pair(&code, "").expect("pair").expect("token")
    };
    let (revoked, revoked_secret) = issue("revoked");
    let (kept, kept_secret) = issue("kept");

    assert!(repository.revoke_token(&revoked.token_id).expect("revoke"));
    assert!(!repository
        .revoke_token(&revoked.token_id)
        .expect("revoke again"));
    assert!(!repository.revoke_token("unknown").expect("revoke unknown"));

    assert_eq!(
        repository
            .token_for_secret(&revoked_secret)
            .expect("lookup"),
        None
    );
    assert_eq!(
        repository.token_for_secret(&kept_secret).expect("lookup"),
        Some(kept.token_id.clone())
    );
    let tokens = repository.tokens().expect("tokens");
    let listed = |id: &str| {
        tokens
            .iter()
            .find(|token| token.token_id == id)
            .expect("listed")
    };
    assert!(listed(&revoked.token_id).revoked_at.is_some());
    assert!(listed(&kept.token_id).revoked_at.is_none());
}

fn created_out(repository: &EnvironmentRepository, request_key: &str) -> String {
    repository
        .create_out(&DelegationOut {
            delegation_id: uuid::Uuid::new_v4().to_string(),
            environment_id: "environment-b".into(),
            project_path: "/b/project".into(),
            kind: "flow".into(),
            request: serde_json::json!({}),
            request_sha256: "sha".into(),
            caller: "owner".into(),
            request_key: request_key.into(),
            owner_conversation_id: None,
            phase: String::new(),
            target_id: None,
            last_state: None,
            result: None,
            paused: None,
            cancel_requested_at: None,
            cancel_acknowledged_at: None,
            created_at: String::new(),
            updated_at: String::new(),
            submitted_at: None,
        })
        .expect("create")
        .delegation_id
}

#[test]
fn a_paused_delegation_seals_locally_only_if_never_submitted_when_it_seals() {
    let temp = tempfile::tempdir().expect("tempdir");
    let home = temp.path().join("spark-home");
    let _store = ConversationStore::open(&home).expect("store");
    let repository = EnvironmentRepository::new(&home);
    let link = EnvironmentLink {
        environment_id: "environment-b".into(),
        origin: "https://b.example".into(),
        label: "B".into(),
        created_at: String::new(),
    };
    repository.save_link(&link, "token").expect("link");
    let canceled = serde_json::json!({"status": "canceled"});
    let record = |id: &str| repository.out(id).expect("read").expect("record");

    // Paused and never submitted: B never saw it, so it seals here, and
    // neither a relink nor the follower reopens or submits it.
    let never = created_out(&repository, "never");
    repository.remove_link("environment-b").expect("unlink");
    assert!(!repository.submit_out(&never).expect("submit"));
    assert!(repository.request_cancel_out(&never).expect("cancel"));
    assert!(repository.seal_out(&never, &canceled, true).expect("seal"));
    repository.save_link(&link, "token").expect("relink");
    assert!(!repository.submit_out(&never).expect("submit"));
    let sealed = record(&never);
    assert_eq!((sealed.phase.as_str(), sealed.paused), ("sealed", None));
    assert_eq!(sealed.submitted_at, None);

    // A relink that commits between the cancel's read and its seal wins: the
    // cancel stays pending for the follower to send.
    let raced = created_out(&repository, "raced");
    repository.remove_link("environment-b").expect("unlink");
    assert!(record(&raced).paused.is_some());
    assert!(repository.request_cancel_out(&raced).expect("cancel"));
    repository.save_link(&link, "token").expect("relink");
    assert!(!repository.seal_out(&raced, &canceled, true).expect("seal"));
    let pending = record(&raced);
    assert_eq!((pending.phase.as_str(), pending.paused), ("created", None));
    assert!(pending.cancel_requested_at.is_some() && pending.cancel_acknowledged_at.is_none());
    // A pending cancel is sent instead of the submission.
    assert!(!repository.submit_out(&raced).expect("submit"));

    // Possibly submitted: B may have it, so its cancel waits for B.
    let sent = created_out(&repository, "sent");
    assert!(repository.submit_out(&sent).expect("submit"));
    let first = record(&sent).submitted_at.expect("submitted");
    assert!(repository.submit_out(&sent).expect("resubmit"));
    assert_eq!(record(&sent).submitted_at, Some(first));
    repository.remove_link("environment-b").expect("unlink");
    assert!(repository.request_cancel_out(&sent).expect("cancel"));
    assert!(!repository.seal_out(&sent, &canceled, true).expect("seal"));
    let held = record(&sent);
    assert_eq!(held.phase, "created");
    assert!(held.paused.is_some() && held.cancel_acknowledged_at.is_none());
    // B's answer seals it, once.
    assert!(repository.seal_out(&sent, &canceled, false).expect("seal"));
    assert!(!repository
        .seal_out(&sent, &canceled, false)
        .expect("seal again"));
}
