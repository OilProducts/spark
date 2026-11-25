use std::fs;
use std::path::Path;

use spark_common::project::sha256_hex;
use spark_storage::environment::{
    ensure_environment_id, DelegationListFilter, DelegationOut, EnvironmentLink,
    EnvironmentRepository, ENVIRONMENT_ID_FILE_NAME, PAIRING_CODE_TTL,
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
    created_out_on(repository, "environment-b", request_key)
}

fn created_out_on(
    repository: &EnvironmentRepository,
    environment_id: &str,
    request_key: &str,
) -> String {
    repository
        .create_out(&DelegationOut {
            delegation_id: uuid::Uuid::new_v4().to_string(),
            environment_id: environment_id.into(),
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

#[test]
fn delegation_lists_filter_in_sql_newest_first() {
    let temp = tempfile::tempdir().expect("tempdir");
    let home = temp.path().join("spark-home");
    let _store = ConversationStore::open(&home).expect("store");
    let repository = EnvironmentRepository::new(&home);
    // Oldest first; each row is named by its request key, and each filter
    // keeps a different set of them.
    //   b-created   environment-b  created
    //   b-accepted  environment-b  accepted  paused
    //   b-sealed    environment-b  sealed
    //   c-created   environment-c  created   paused
    //   c-accepted  environment-c  accepted
    //   c-sealed    environment-c  sealed
    let mut ids = Vec::new();
    for (second, (environment, phase)) in ["b", "c"]
        .into_iter()
        .flat_map(|env| ["created", "accepted", "sealed"].map(|phase| (env, phase)))
        .enumerate()
    {
        let id = created_out_on(
            &repository,
            &format!("environment-{environment}"),
            &format!("{environment}-{phase}"),
        );
        if phase != "created" {
            assert!(repository.accept_out(&id, "target").expect("accept"));
        }
        if phase == "sealed" {
            let completed = serde_json::json!({"status": "completed"});
            assert!(repository.seal_out(&id, &completed, false).expect("seal"));
        }
        state_database(&home)
            .execute(
                "UPDATE delegations_out SET created_at = ?2 WHERE delegation_id = ?1",
                [&id, &format!("2026-01-01T00:00:0{second}Z")],
            )
            .expect("created at");
        ids.push(id);
    }
    for paused in [&ids[1], &ids[3]] {
        assert!(repository.pause_out(paused, "Paused").expect("pause"));
    }
    let filter =
        |environment: Option<&str>, phase: Option<&str>, open, paused| DelegationListFilter {
            environment_id: environment.map(|env| format!("environment-{env}")),
            phase: phase.map(str::to_string),
            open,
            paused,
        };
    let list = |filter: DelegationListFilter| -> Vec<String> {
        repository
            .list_out(&filter)
            .expect("list")
            .into_iter()
            .map(|record| record.request_key)
            .collect()
    };

    let all = filter(None, None, false, false);
    assert_eq!(
        list(all.clone()),
        [
            "c-sealed",
            "c-accepted",
            "c-created",
            "b-sealed",
            "b-accepted",
            "b-created"
        ]
    );
    for (filter, expected) in [
        (
            filter(Some("b"), None, false, false),
            &["b-sealed", "b-accepted", "b-created"][..],
        ),
        (
            filter(Some("c"), None, false, false),
            &["c-sealed", "c-accepted", "c-created"],
        ),
        (
            filter(None, Some("created"), false, false),
            &["c-created", "b-created"],
        ),
        (
            filter(None, Some("accepted"), false, false),
            &["c-accepted", "b-accepted"],
        ),
        (
            filter(None, Some("sealed"), false, false),
            &["c-sealed", "b-sealed"],
        ),
        (
            filter(None, None, true, false),
            &["c-accepted", "c-created", "b-accepted", "b-created"],
        ),
        (
            filter(None, None, false, true),
            &["c-created", "b-accepted"],
        ),
        (
            filter(Some("b"), None, true, false),
            &["b-accepted", "b-created"],
        ),
        (filter(Some("c"), None, false, true), &["c-created"]),
        (filter(None, Some("accepted"), false, true), &["b-accepted"]),
        (
            filter(Some("c"), Some("sealed"), false, false),
            &["c-sealed"],
        ),
        (filter(Some("b"), Some("created"), true, true), &[]),
        (filter(Some("b"), Some("sealed"), true, false), &[]),
        (filter(Some("missing"), None, false, false), &[]),
    ] {
        assert_eq!(list(filter.clone()), expected, "{filter:?}");
    }

    // A pause, a relink that resumes the environment's paused records, and a
    // seal each move a record between lists.
    assert!(repository.pause_out(&ids[4], "Paused").expect("pause"));
    let paused = filter(None, None, false, true);
    assert_eq!(
        list(paused.clone()),
        ["c-accepted", "c-created", "b-accepted"]
    );
    let link = EnvironmentLink {
        environment_id: "environment-c".into(),
        origin: "https://c.example".into(),
        label: "C".into(),
        created_at: String::new(),
    };
    repository.save_link(&link, "token").expect("relink");
    assert_eq!(list(paused.clone()), ["b-accepted"]);
    let completed = serde_json::json!({"status": "completed"});
    assert!(repository
        .seal_out(&ids[1], &completed, false)
        .expect("seal"));
    assert!(list(paused).is_empty());
    assert_eq!(
        list(filter(None, None, true, false)),
        ["c-accepted", "c-created", "b-created"]
    );
    assert_eq!(
        list(filter(None, Some("sealed"), false, false)),
        ["c-sealed", "b-sealed", "b-accepted"]
    );

    // Records created in the same second list by ID, highest first.
    state_database(&home)
        .execute(
            "UPDATE delegations_out SET created_at = '2026-01-01T00:00:00Z'",
            [],
        )
        .expect("same second");
    let mut by_id = ids.clone();
    by_id.sort_unstable_by(|a, b| b.cmp(a));
    let listed: Vec<_> = repository
        .list_out(&all)
        .expect("list")
        .into_iter()
        .map(|record| record.delegation_id)
        .collect();
    assert_eq!(listed, by_id);

    // The filter runs before any payload is read: a row whose request no
    // read can decode fails only the lists that include it.
    let database = state_database(&home);
    database
        .execute_batch("PRAGMA ignore_check_constraints = ON")
        .expect("pragma");
    database
        .execute(
            "UPDATE delegations_out SET request = X'00' WHERE delegation_id = ?1",
            [&ids[5]],
        )
        .expect("undecodable request");
    assert!(repository.list_out(&all).is_err());
    assert!(repository
        .list_out(&filter(None, Some("sealed"), false, false))
        .is_err());
    assert_eq!(list(filter(Some("b"), None, false, false)).len(), 3);
    assert_eq!(list(filter(None, None, true, false)).len(), 3);
    assert_eq!(
        list(filter(Some("c"), Some("created"), false, false)),
        ["c-created"]
    );
}

#[test]
fn agent_token_is_internal_and_replaced_on_each_provision() {
    let temp = tempfile::tempdir().expect("tempdir");
    let home = temp.path().join("spark-home");
    let _store = ConversationStore::open(&home).expect("store");
    let repository = EnvironmentRepository::new(&home);
    let (code, _) = repository.create_pairing_code("laptop").expect("code");
    let (peer, peer_secret) = repository.pair(&code, "").expect("pair").expect("token");

    let first = repository.provision_agent_token().expect("provision");
    let agent_id = repository
        .token_for_secret(&first)
        .expect("lookup")
        .expect("the agent token authenticates");
    assert_ne!(agent_id, peer.token_id);
    assert_eq!(repository.tokens().expect("tokens"), vec![peer.clone()]);
    assert!(!repository.revoke_token(&agent_id).expect("revoke"));
    assert!(repository
        .token_for_secret(&first)
        .expect("lookup")
        .is_some());

    // Provisioning replaces the secret and resets an earlier revocation.
    state_database(&home)
        .execute(
            "UPDATE peer_tokens SET revoked_at = '2000-01-01T00:00:00Z' WHERE token_id = ?1",
            [&agent_id],
        )
        .expect("revoke directly");
    let second = repository.provision_agent_token().expect("provision again");
    assert_ne!(second, first);
    assert_eq!(repository.token_for_secret(&first).expect("lookup"), None);
    assert_eq!(
        repository.token_for_secret(&second).expect("lookup"),
        Some(agent_id)
    );

    repository.clear_agent_token().expect("clear");
    assert_eq!(repository.token_for_secret(&second).expect("lookup"), None);
    repository.clear_agent_token().expect("clearing again");
    assert_eq!(
        repository.token_for_secret(&peer_secret).expect("lookup"),
        Some(peer.token_id.clone())
    );
    assert_eq!(repository.tokens().expect("tokens"), vec![peer]);
}

/// The state database holds bearer secrets, so its files are owner-only.
#[cfg(unix)]
mod private_state_files {
    use std::fs;
    use std::os::unix::fs::{symlink, PermissionsExt as _};
    use std::path::{Path, PathBuf};

    use rusqlite::TransactionBehavior;
    use spark_storage::environment::{EnvironmentLink, EnvironmentRepository};
    use spark_storage::{ConversationStore, StorageError};

    const SIDECARS: [&str; 3] = ["-wal", "-shm", "-journal"];
    const LOCK_PROBE_ENV: &str = "SPARK_STORAGE_TEST_LOCK_PROBE";

    fn mode(path: &Path) -> u32 {
        fs::metadata(path).expect("metadata").permissions().mode() & 0o777
    }

    fn set_mode(path: &Path, mode: u32) {
        fs::set_permissions(path, fs::Permissions::from_mode(mode)).expect("chmod");
    }

    fn sibling(database: &Path, suffix: &str) -> PathBuf {
        let mut path = database.as_os_str().to_os_string();
        path.push(suffix);
        path.into()
    }

    /// Saves a link with the token `link-secret`; the connection closes, so
    /// SQLite checkpoints and removes the WAL and shared-memory files.
    fn saved_link(repository: &EnvironmentRepository) -> EnvironmentLink {
        let link = EnvironmentLink {
            environment_id: uuid::Uuid::new_v4().to_string(),
            origin: "http://peer.example:8000".to_string(),
            label: "Peer".to_string(),
            created_at: spark_storage::environment::now(),
        };
        repository
            .save_link(&link, "link-secret")
            .expect("save link");
        link
    }

    /// A connection that keeps the WAL and shared-memory files open, as a
    /// running store does.
    fn reader(database: &Path) -> rusqlite::Connection {
        let connection = rusqlite::Connection::open(database).expect("reader");
        connection
            .query_row("SELECT count(*) FROM environment_links", [], |row| {
                row.get::<_, i64>(0)
            })
            .expect("read");
        connection
    }

    #[test]
    fn a_new_database_and_its_sidecars_start_private() {
        let temp = tempfile::tempdir().expect("tempdir");
        let home = temp.path().join("spark-home");
        let _store = ConversationStore::open(&home).expect("store");
        let database = home.join("state.sqlite3");
        assert_eq!(mode(&database), 0o600);

        saved_link(&EnvironmentRepository::new(&home));
        assert_eq!(mode(&database), 0o600);
        for suffix in ["-wal", "-shm"] {
            assert_eq!(mode(&sibling(&database, suffix)), 0o600, "{suffix}");
        }
    }

    #[test]
    fn broadened_files_are_tightened_and_keep_their_links() {
        let temp = tempfile::tempdir().expect("tempdir");
        let home = temp.path().join("spark-home");
        let repository = EnvironmentRepository::new(&home);
        let link = saved_link(&repository);
        let database = home.join("state.sqlite3");
        let _reader = reader(&database);
        fs::write(sibling(&database, "-journal"), b"").expect("journal");
        set_mode(&database, 0o644);
        for suffix in SIDECARS {
            set_mode(&sibling(&database, suffix), 0o644);
        }

        assert_eq!(
            repository.link(&link.environment_id).expect("link"),
            Some((link, "link-secret".to_string()))
        );
        assert_eq!(mode(&database), 0o600);
        for suffix in SIDECARS {
            assert_eq!(mode(&sibling(&database, suffix)), 0o600, "{suffix}");
        }
    }

    #[test]
    fn a_linked_database_keeps_its_link_and_its_real_files_are_tightened() {
        let temp = tempfile::tempdir().expect("tempdir");
        let elsewhere = temp.path().join("elsewhere");
        let link = saved_link(&EnvironmentRepository::new(&elsewhere));
        let real = elsewhere.join("relocated.sqlite3");
        fs::rename(elsewhere.join("state.sqlite3"), &real).expect("relocate");
        let home = temp.path().join("spark-home");
        fs::create_dir_all(&home).expect("home");
        symlink(&real, home.join("state.sqlite3")).expect("link database");
        let _reader = reader(&real);
        for path in [&real, &sibling(&real, "-wal"), &sibling(&real, "-shm")] {
            set_mode(path, 0o644);
        }

        assert_eq!(
            EnvironmentRepository::new(&home)
                .link(&link.environment_id)
                .expect("link"),
            Some((link, "link-secret".to_string()))
        );
        assert_eq!(
            fs::read_link(home.join("state.sqlite3")).expect("still a link"),
            real
        );
        for path in [&real, &sibling(&real, "-wal"), &sibling(&real, "-shm")] {
            assert_eq!(mode(path), 0o600, "{}", path.display());
        }
        assert!(!home.join("state.sqlite3-wal").exists());
    }

    #[test]
    fn a_linked_sidecar_fails_the_open_and_its_target_is_untouched() {
        let temp = tempfile::tempdir().expect("tempdir");
        let home = temp.path().join("spark-home");
        let repository = EnvironmentRepository::new(&home);
        let link = saved_link(&repository);
        let database = fs::canonicalize(home.join("state.sqlite3")).expect("resolve");
        for suffix in SIDECARS {
            let target = temp.path().join(format!("target{suffix}"));
            fs::write(&target, b"keep").expect("target");
            set_mode(&target, 0o644);
            let sidecar = sibling(&database, suffix);
            symlink(&target, &sidecar).expect("link sidecar");

            let error = repository.links().expect_err("a linked sidecar fails");
            let message = error.to_string();
            assert!(
                message.contains(&sidecar.display().to_string())
                    && message.contains("not a regular file"),
                "{message}"
            );
            assert_eq!(mode(&target), 0o644);
            assert_eq!(fs::read(&target).expect("target"), b"keep");
            fs::remove_file(&sidecar).expect("unlink sidecar");
        }
        assert_eq!(repository.links().expect("links"), vec![link]);
    }

    #[test]
    fn concurrent_first_opens_keep_the_winning_database() {
        let temp = tempfile::tempdir().expect("tempdir");
        let home = temp.path().join("spark-home");
        let barrier = std::sync::Barrier::new(8);
        let written = std::thread::scope(|scope| {
            let openers: Vec<_> = (0..8)
                .map(|i| {
                    let (home, barrier) = (&home, &barrier);
                    scope.spawn(move || {
                        barrier.wait();
                        match EnvironmentRepository::new(home)
                            .create_pairing_code(&format!("peer-{i}"))
                        {
                            Ok(_) => true,
                            // SQLite reports BUSY to openers racing the
                            // switch of a new file to WAL.
                            Err(StorageError::ConversationDatabase(error))
                                if error.sqlite_error_code()
                                    == Some(rusqlite::ErrorCode::DatabaseBusy) =>
                            {
                                false
                            }
                            Err(error) => panic!("{error}"),
                        }
                    })
                })
                .collect();
            openers
                .into_iter()
                .map(|opener| opener.join().expect("opener"))
                .filter(|written| *written)
                .count()
        });
        let codes: i64 = super::state_database(&home)
            .query_row("SELECT count(*) FROM pairing_codes", [], |row| row.get(0))
            .expect("count");
        assert!(written > 0);
        assert_eq!(codes, written as i64, "every acknowledged write is kept");
        assert_eq!(mode(&home.join("state.sqlite3")), 0o600);
    }

    /// Tightening while another connection holds the write lock must not
    /// release it: closing an ordinary descriptor on a SQLite file drops the
    /// process's POSIX locks on it. Only another process sees that.
    #[test]
    fn a_short_lived_open_keeps_another_connections_write_lock() {
        if let Some(database) = std::env::var_os(LOCK_PROBE_ENV) {
            let writer = rusqlite::Connection::open(database).expect("writer");
            writer
                .busy_timeout(std::time::Duration::ZERO)
                .expect("timeout");
            let error = writer
                .execute_batch("BEGIN IMMEDIATE")
                .expect_err("the write lock is held");
            assert_eq!(
                error.sqlite_error_code(),
                Some(rusqlite::ErrorCode::DatabaseBusy),
                "{error}"
            );
            return;
        }
        let temp = tempfile::tempdir().expect("tempdir");
        let home = temp.path().join("spark-home");
        let repository = EnvironmentRepository::new(&home);
        let link = saved_link(&repository);
        let database = home.join("state.sqlite3");
        let mut holder = rusqlite::Connection::open(&database).expect("holder");
        let tx = holder
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .expect("write transaction");
        tx.execute("DELETE FROM environment_links", [])
            .expect("delete");
        // Broadened modes make the opener change every file under the lock.
        let files = [
            &database,
            &sibling(&database, "-wal"),
            &sibling(&database, "-shm"),
        ];
        for path in files {
            set_mode(path, 0o644);
        }

        assert_eq!(repository.links().expect("links"), vec![link]);
        for path in files {
            assert_eq!(mode(path), 0o600, "{}", path.display());
        }
        let test = "environment_contracts::private_state_files::\
                    a_short_lived_open_keeps_another_connections_write_lock";
        let probe = std::process::Command::new(std::env::current_exe().expect("test binary"))
            .args(["--exact", test, "--nocapture"])
            .env(LOCK_PROBE_ENV, &database)
            .output()
            .expect("probe");
        let stdout = String::from_utf8_lossy(&probe.stdout);
        assert!(
            probe.status.success() && stdout.contains("1 passed"),
            "{stdout}\n{}",
            String::from_utf8_lossy(&probe.stderr)
        );
        tx.commit().expect("the held transaction still commits");
        assert!(repository.links().expect("links").is_empty());
    }
}
