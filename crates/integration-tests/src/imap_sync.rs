//! End-to-end MK-7 acceptance tests for the IMAP sync engine against a real
//! greenmail IMAP server running in colima/docker.
//!
//! Unlike `greenmail.rs` (which only exercises connectivity + LIST through nop
//! sync services), these tests wire the **real** sync pipeline:
//!
//! ```text
//! ImapAdapter (sync engine: connect → SELECT → UID FETCH → IDLE)
//!   → IngestService  → encrypted filesystem storage (tempdir) + ParseMessageJob
//!   → job worker (core subsystem) → MessageService → sqlite::memory: DB rows
//! ```
//!
//! The adapter is driven directly via `ImapPort::start_account` with an
//! `ImapConnectionParams` (the plan explicitly permits driving the adapter
//! directly rather than going through `ImapAccountService`, which would also
//! require credential decryption and account loading that add nothing to the
//! sync-engine assertions).
//!
//! ## Message-row-vs-parse-job seam
//!
//! `IngestService::ingest_raw` stores the encrypted raw blob and enqueues a
//! `ParseMessageJob`; the `Message` rows are produced asynchronously by the
//! parser handler running inside the core job worker. We therefore run the core
//! subsystem (`run_core`) so the worker drains those jobs, exactly as the MK-5
//! ingest integration test (`ingest.rs`) does, and **poll the DB** for the
//! expected `Message` rows. This proves the full pipeline reaches DB rows +
//! on-disk ciphertext, not merely the enqueue seam.
//!
//! Compiled only under the `greenmail` feature and `#[ignore]`d, so the default
//! `sqlite` run never touches them. Run with `just imap-integration-tests`.

use std::{sync::Arc, time::Duration};

use chrono::Utc;
use mk_core::{
    account::{AccountId, CreateAccountParams},
    folder::{Folder, FolderId, NewFolderRequest, SpecialUse},
    imap::{FolderConfig, ImapConnectionParams, ImapCredentials, ImapPort, ImapServerConfig, SyncState},
    message::MessageId,
    repository::{RepositoryService, transaction},
    test_support::{default_external_services_builder, test_cipher_service},
    types::EmailAddress,
    user::{NewUser, User},
};
use mk_database::create_repository_service;
use mk_imap::ImapAdapter;
use sea_orm::Database;
use secrecy::SecretString;
use tempfile::TempDir;

use crate::{
    context::TestContext,
    greenmail_support::{
        ACCOUNT_TIMEOUT, Control, Greenmail, PASSWORD, USERNAME, assert_ciphertext_on_disk, insecure_tls, list_messages, run_core, wait_for_messages,
    },
};

// ─── System-under-test harness: real ingest/storage/parser pipeline
// ───────────

/// Build the real core pipeline (sqlite::memory: DB + encrypted tempdir storage
/// + parser handlers) behind a `TestContext`. Mirrors `ingest.rs::setup_fs`.
async fn setup_pipeline() -> TestContext {
    let dir = TempDir::new().unwrap();
    let cipher = test_cipher_service();
    let storage = mk_storage::create_filesystem_storage(dir.path(), cipher.clone()).await.unwrap();

    let db = Database::connect("sqlite::memory:").await.unwrap();
    let repository_service = create_repository_service(db).await.unwrap();

    let core = mk_core::create_services(
        default_external_services_builder()
            .repository_service(repository_service.clone())
            .cipher_service(cipher)
            .raw_storage_service(storage.raw_storage_service)
            .attachment_storage_service(storage.attachment_storage_service)
            .build()
            .unwrap(),
    )
    .unwrap();

    mk_parser::register_handlers(&core);

    TestContext::new(core, repository_service, dir)
}

/// Build the system-under-test adapter from the real pipeline services with the
/// insecure (greenmail-trusting) TLS config.
fn make_adapter(ctx: &TestContext, poll_interval: Duration) -> ImapAdapter {
    ImapAdapter::with_tls_config(
        ctx.services.ingest_service.clone(),
        ctx.services.folder_service.clone(),
        ctx.services.message_service.clone(),
        poll_interval,
        insecure_tls(),
    )
}

// ─── Fixtures
// ─────────────────────────────────────────────────────────────────

async fn make_user(ctx: &TestContext) -> User {
    let new_user = NewUser::new("alice", "password-hash", "alice@example.com", std::collections::HashSet::new(), "Alice", false).unwrap();
    ctx.services.user_service.add_user(new_user).await.unwrap()
}

async fn make_account(ctx: &TestContext, user_id: u64, server: ImapServerConfig) -> AccountId {
    let params = CreateAccountParams {
        user_id,
        display_name: "Greenmail".into(),
        email_address: EmailAddress::new("alice@example.com").unwrap(),
        server,
        username: USERNAME.into(),
        password: SecretString::from(PASSWORD.to_string()),
    };
    ctx.services.account_service.create_account(params).await.unwrap().id
}

/// Create an enabled INBOX folder. `idle` selects whether the engine drives it
/// via the dedicated IDLE connection (true) or the poll loop (false).
async fn make_inbox(ctx: &TestContext, account_id: AccountId, idle: bool) -> Folder {
    let folder = ctx
        .services
        .folder_service
        .create_folders_for_account(
            account_id,
            vec![NewFolderRequest {
                path: "INBOX".into(),
                display_name: None,
                special_use: Some(SpecialUse::Inbox),
                // Start with no known UIDVALIDITY so the first SELECT records the
                // server's value (the rollover path triggers only on a *change*).
                uidvalidity: None,
            }],
        )
        .await
        .unwrap()
        .into_iter()
        .next()
        .unwrap();
    if idle {
        ctx.services.folder_service.set_idle_enabled(folder.id, true).await.unwrap();
    }
    folder
}

/// Create an enabled, polled (non-IDLE) folder at an arbitrary path.
async fn make_folder(ctx: &TestContext, account_id: AccountId, path: &str, special_use: Option<SpecialUse>) -> Folder {
    ctx.services
        .folder_service
        .create_folders_for_account(
            account_id,
            vec![NewFolderRequest {
                path: path.into(),
                display_name: None,
                special_use,
                uidvalidity: None,
            }],
        )
        .await
        .unwrap()
        .into_iter()
        .next()
        .unwrap()
}

fn creds() -> ImapCredentials {
    ImapCredentials {
        username: USERNAME.into(),
        password: SecretString::from(PASSWORD),
    }
}

fn params_for(server: ImapServerConfig, folders: &[&Folder]) -> ImapConnectionParams {
    ImapConnectionParams {
        server,
        credentials: creds(),
        folders: folders
            .iter()
            .map(|f| FolderConfig {
                id: f.id,
                path: f.path.clone(),
                idle_enabled: f.idle_enabled,
                uidvalidity: f.uidvalidity,
                last_uid: f.last_uid,
            })
            .collect(),
    }
}

// ─── DB query helpers
// ─────────────────────────────────────────────────────────

async fn message_count(repos: &Arc<RepositoryService>, account_id: AccountId) -> usize {
    list_messages(repos, account_id).await.len()
}

async fn location_exists(repos: &Arc<RepositoryService>, message_id: MessageId, folder_id: FolderId) -> bool {
    transaction(&**repos.repository(), |tx| {
        let r = repos.message_location_repository().clone();
        Box::pin(async move { r.find_by_message_and_folder(tx, message_id, folder_id).await })
    })
    .await
    .unwrap()
    .is_some()
}

/// Re-read a folder row to inspect the sync cursor (uidvalidity / last_uid).
async fn folder_row(repos: &Arc<RepositoryService>, account_id: AccountId, folder_id: FolderId) -> Folder {
    transaction(&**repos.repository(), |tx| {
        let r = repos.folder_repository().clone();
        Box::pin(async move { r.find_by_id_for_account(tx, account_id, folder_id).await })
    })
    .await
    .unwrap()
    .expect("folder row exists")
}

/// Poll the live `SyncStatus` until `pred` holds, or panic after `timeout`.
async fn wait_for_status(
    adapter: &ImapAdapter,
    account_id: AccountId,
    timeout: Duration,
    label: &str,
    pred: impl Fn(&mk_core::imap::SyncStatus) -> bool,
) -> mk_core::imap::SyncStatus {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let status = adapter.status(account_id).await.unwrap();
        if pred(&status) {
            return status;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "status predicate '{label}' not satisfied within {timeout:?}; last = {status:?}"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

// ─── Scenario 1: initial sync
// ─────────────────────────────────────────────────

/// Seed greenmail's INBOX with N messages, start the account, and assert the
/// full pipeline lands N Message rows + on-disk ciphertext and advances the
/// folder cursor (`last_uid` / `uidvalidity`). Uses the poll loop (idle=false)
/// with a short poll interval so the initial pass runs promptly.
#[tokio::test]
#[ignore = "needs a docker/colima daemon — run via `just imap-integration-tests`"]
async fn initial_sync_ingests_seeded_messages() {
    let gm = Greenmail::start().await;
    let mut control = Control::connect(&gm).await.unwrap();

    const N: usize = 5;
    for i in 0..N {
        control.append("INBOX", &format!("seed message {i}")).await.unwrap();
    }
    let server_uidvalidity = control.select_uidvalidity("INBOX").await.unwrap();
    let _ = control.logout().await;

    let ctx = setup_pipeline().await;
    let user = make_user(&ctx).await;
    let account_id = make_account(&ctx, user.id, gm.server()).await;
    let inbox = make_inbox(&ctx, account_id, false).await;
    let core = run_core(&ctx);

    let adapter = make_adapter(&ctx, Duration::from_secs(1));
    adapter.start_account(account_id, params_for(gm.server(), &[&inbox])).await.unwrap();

    let msgs = wait_for_messages(&ctx.repos, account_id, N, ACCOUNT_TIMEOUT).await;
    assert_eq!(msgs.len(), N, "all seeded messages must be ingested into DB rows");

    // Every message has on-disk ciphertext and a location in INBOX.
    for msg in &msgs {
        assert_ciphertext_on_disk(&ctx, account_id, msg).await;
        assert!(location_exists(&ctx.repos, msg.id, inbox.id).await, "message {} located in INBOX", msg.id);
    }

    // The folder cursor advanced: uidvalidity recorded, last_uid == N (UIDs are
    // 1..=N for a freshly-seeded greenmail mailbox).
    let row = folder_row(&ctx.repos, account_id, inbox.id).await;
    assert_eq!(row.uidvalidity, Some(server_uidvalidity), "folder uidvalidity recorded from server");
    assert_eq!(row.last_uid, N as u32, "last_uid advanced to the highest fetched UID");

    adapter.stop_account(account_id).await.unwrap();
    core.abort();
    let _ = core.await;
}

// ─── Scenario 2: IDLE liveness
// ────────────────────────────────────────────────

/// With the account running on the IDLE connection and a poll interval far
/// above the test timeout (1 hour), APPEND a new message and assert it is
/// ingested within a few seconds — proving IDLE (not the poll loop) caught it.
#[tokio::test]
#[ignore = "needs a docker/colima daemon — run via `just imap-integration-tests`"]
async fn idle_ingests_new_message_promptly() {
    let gm = Greenmail::start().await;

    let ctx = setup_pipeline().await;
    let user = make_user(&ctx).await;
    let account_id = make_account(&ctx, user.id, gm.server()).await;
    // idle=true → driven by the dedicated IDLE connection.
    let inbox = make_inbox(&ctx, account_id, true).await;
    let core = run_core(&ctx);

    // Poll interval far beyond the assertion window: only IDLE can catch new mail.
    let adapter = make_adapter(&ctx, Duration::from_hours(1));
    adapter.start_account(account_id, params_for(gm.server(), &[&inbox])).await.unwrap();

    // Wait for the engine to reach Idle (initial catch-up done, parked in IDLE).
    wait_for_status(&adapter, account_id, ACCOUNT_TIMEOUT, "reach Idle", |s| s.state == SyncState::Idle).await;
    assert_eq!(message_count(&ctx.repos, account_id).await, 0, "no mail yet");

    // Deliver a new message; greenmail emits EXISTS, waking IDLE.
    let mut control = Control::connect(&gm).await.unwrap();
    control.append("INBOX", "live idle message").await.unwrap();
    let _ = control.logout().await;

    // It should be ingested well within the poll interval (IDLE-driven).
    let msgs = wait_for_messages(&ctx.repos, account_id, 1, Duration::from_secs(15)).await;
    assert_eq!(msgs.len(), 1, "IDLE must ingest the APPENDed message promptly");
    assert_ciphertext_on_disk(&ctx, account_id, &msgs[0]).await;

    adapter.stop_account(account_id).await.unwrap();
    core.abort();
    let _ = core.await;
}

// ─── Scenario 3: UIDVALIDITY rollover
// ─────────────────────────────────────────

/// Exercise the UIDVALIDITY rollover path end-to-end against the real server.
///
/// **greenmail limitation (verified empirically):** greenmail 2.1.0 assigns a
/// *stable* UIDVALIDITY to a mailbox and REUSES the same value after DELETE +
/// CREATE — so recreating a mailbox does not produce a server-side rollover,
/// and (separately) RFC 3501 forbids deleting INBOX. We therefore induce the
/// exact condition a real rollover creates — "the server's UIDVALIDITY no
/// longer matches the one we last recorded for this folder" — by recording a
/// *stale* UIDVALIDITY in the folder row before reconnecting. The engine's
/// production rollover code (`sync_folder`'s mismatch check →
/// `handle_uidvalidity_change` → `delete_locations_for_folder` → re-fetch from
/// UID 1) runs unchanged against the live greenmail mailbox. Only the trigger
/// is simulated; the cleanup + re-ingest is real.
///
/// Asserts: stale locations are dropped and then re-created (re-ingest), the
/// folder adopts the server's real UIDVALIDITY, and the deduplicated Message
/// rows are PRESERVED (same content → same rows; only locations churn).
#[tokio::test]
#[ignore = "needs a docker/colima daemon — run via `just imap-integration-tests`"]
async fn uidvalidity_rollover_drops_locations_and_reingests() {
    let gm = Greenmail::start().await;
    let mut control = Control::connect(&gm).await.unwrap();

    // Seed two messages into a regular (polled) mailbox.
    const MAILBOX: &str = "Archive";
    control.ensure_mailbox(MAILBOX).await;
    control.append(MAILBOX, "rollover message a").await.unwrap();
    control.append(MAILBOX, "rollover message b").await.unwrap();
    let real_uidvalidity = control.select_uidvalidity(MAILBOX).await.unwrap();
    let _ = control.logout().await;
    drop(control);

    let ctx = setup_pipeline().await;
    let user = make_user(&ctx).await;
    let account_id = make_account(&ctx, user.id, gm.server()).await;
    let archive = make_folder(&ctx, account_id, MAILBOX, Some(SpecialUse::Archive)).await;
    let core = run_core(&ctx);

    let adapter = make_adapter(&ctx, Duration::from_secs(1));
    adapter.start_account(account_id, params_for(gm.server(), &[&archive])).await.unwrap();

    // Initial sync: both messages ingested, located, folder records the real
    // server UIDVALIDITY.
    let msgs_before = wait_for_messages(&ctx.repos, account_id, 2, ACCOUNT_TIMEOUT).await;
    assert_eq!(msgs_before.len(), 2);
    let ids_before: std::collections::HashSet<MessageId> = msgs_before.iter().map(|m| m.id).collect();
    for msg in &msgs_before {
        assert!(location_exists(&ctx.repos, msg.id, archive.id).await, "located before rollover");
    }
    let row_before = folder_row(&ctx.repos, account_id, archive.id).await;
    assert_eq!(row_before.uidvalidity, Some(real_uidvalidity));
    assert!(row_before.last_uid >= 2, "cursor advanced before rollover");

    // Stop the engine, then poison the recorded cursor with a STALE UIDVALIDITY
    // (real + 1). This is exactly the persisted state after a real server-side
    // rollover that we have not yet observed.
    adapter.stop_account(account_id).await.unwrap();
    let stale_uidvalidity = real_uidvalidity.wrapping_add(1);
    ctx.services
        .folder_service
        .record_sync_progress(archive.id, stale_uidvalidity, row_before.last_uid, Utc::now())
        .await
        .unwrap();
    let poisoned = folder_row(&ctx.repos, account_id, archive.id).await;
    assert_eq!(poisoned.uidvalidity, Some(stale_uidvalidity), "stale uidvalidity recorded");

    // Restart with the poisoned cursor. On the next SELECT the engine sees
    // server_uidvalidity (real) != recorded (stale) → rollover cleanup + re-fetch.
    let restart_folder = FolderConfig {
        id: archive.id,
        path: archive.path.clone(),
        idle_enabled: false,
        uidvalidity: poisoned.uidvalidity,
        last_uid: poisoned.last_uid,
    };
    adapter
        .start_account(
            account_id,
            ImapConnectionParams {
                server: gm.server(),
                credentials: creds(),
                folders: vec![restart_folder],
            },
        )
        .await
        .unwrap();

    // The engine adopts the server's real UIDVALIDITY (rollover handled) and
    // resets+re-advances the cursor.
    let deadline = tokio::time::Instant::now() + ACCOUNT_TIMEOUT;
    loop {
        let row = folder_row(&ctx.repos, account_id, archive.id).await;
        if row.uidvalidity == Some(real_uidvalidity) && row.last_uid >= 2 {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "engine never adopted the server UIDVALIDITY after rollover (last row: {row:?})"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    // Re-ingest completed: every message is re-located in the folder (the stale
    // locations were dropped by `delete_locations_for_folder`, then re-created).
    let deadline = tokio::time::Instant::now() + ACCOUNT_TIMEOUT;
    loop {
        let msgs = list_messages(&ctx.repos, account_id).await;
        let mut all_located = !msgs.is_empty();
        for msg in &msgs {
            if !location_exists(&ctx.repos, msg.id, archive.id).await {
                all_located = false;
                break;
            }
        }
        if all_located {
            break;
        }
        assert!(tokio::time::Instant::now() < deadline, "messages were not re-located after rollover");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    // Message rows are PRESERVED across rollover (same content dedups to the same
    // rows — only locations were dropped and re-created, never the messages).
    let msgs_after = list_messages(&ctx.repos, account_id).await;
    let ids_after: std::collections::HashSet<MessageId> = msgs_after.iter().map(|m| m.id).collect();
    assert_eq!(ids_after, ids_before, "Message rows must be preserved across UIDVALIDITY rollover");

    adapter.stop_account(account_id).await.unwrap();
    core.abort();
    let _ = core.await;
}

// ─── Scenario 4: graceful shutdown
// ────────────────────────────────────────────

/// A running account stops cleanly on `stop_account`: tasks drain and `status`
/// reports `NotRunning`.
#[tokio::test]
#[ignore = "needs a docker/colima daemon — run via `just imap-integration-tests`"]
async fn stop_account_halts_tasks_and_reports_not_running() {
    let gm = Greenmail::start().await;
    let mut control = Control::connect(&gm).await.unwrap();
    control.append("INBOX", "shutdown message").await.unwrap();
    let _ = control.logout().await;

    let ctx = setup_pipeline().await;
    let user = make_user(&ctx).await;
    let account_id = make_account(&ctx, user.id, gm.server()).await;
    let inbox = make_inbox(&ctx, account_id, true).await;
    let core = run_core(&ctx);

    let adapter = make_adapter(&ctx, Duration::from_hours(1));
    adapter.start_account(account_id, params_for(gm.server(), &[&inbox])).await.unwrap();

    // Engine reaches Idle (proves the tasks are live before we stop them).
    wait_for_status(&adapter, account_id, ACCOUNT_TIMEOUT, "reach Idle", |s| s.state == SyncState::Idle).await;

    // stop_account cancels + drains; it must complete promptly even though the
    // IDLE task is parked in a 29-minute IDLE (cancellation sends DONE).
    tokio::time::timeout(Duration::from_secs(10), adapter.stop_account(account_id))
        .await
        .expect("stop_account must drain within 10s (IDLE cancellation sends DONE)")
        .unwrap();

    let status = adapter.status(account_id).await.unwrap();
    assert_eq!(status.state, SyncState::NotRunning, "after stop, status must be NotRunning");

    core.abort();
    let _ = core.await;
}

// ─── Scenario 5: bad credentials
// ──────────────────────────────────────────────

/// Starting with a wrong password drives the connect path to repeated auth
/// failures. The engine retries with backoff and only flips `SyncState::Error`
/// after [`FAILURE_ERROR_THRESHOLD`] (5) consecutive failures (≈75s of backoff:
/// 5+10+20+40), which is too long for a bounded test. We instead assert the
/// **first observable failure signal** the engine exposes: `last_error` becomes
/// populated (the auth rejection) and the state leaves the healthy `Idle`. This
/// is the earliest honest signal; we do NOT shorten production backoff to force
/// the full Error transition.
#[tokio::test]
#[ignore = "needs a docker/colima daemon — run via `just imap-integration-tests`"]
async fn bad_credentials_surface_failure_signal() {
    let gm = Greenmail::start().await;

    let ctx = setup_pipeline().await;
    let user = make_user(&ctx).await;
    let account_id = make_account(&ctx, user.id, gm.server()).await;
    let inbox = make_inbox(&ctx, account_id, false).await;
    let core = run_core(&ctx);

    let adapter = make_adapter(&ctx, Duration::from_secs(1));
    // Wrong password.
    let bad = ImapConnectionParams {
        server: gm.server(),
        credentials: ImapCredentials {
            username: USERNAME.into(),
            password: SecretString::from("WRONG-PASSWORD"),
        },
        folders: vec![FolderConfig {
            id: inbox.id,
            path: inbox.path.clone(),
            idle_enabled: false,
            uidvalidity: inbox.uidvalidity,
            last_uid: inbox.last_uid,
        }],
    };
    adapter.start_account(account_id, bad).await.unwrap();

    // The first failed pass records a `last_error` and the state is no longer the
    // healthy `Idle` (it is Connecting/Error during the retry/backoff cycle).
    let status = wait_for_status(&adapter, account_id, Duration::from_secs(20), "first failure recorded", |s| {
        s.last_error.is_some() && s.state != SyncState::Idle
    })
    .await;
    assert!(status.last_error.is_some(), "auth failure must populate last_error");
    assert_ne!(status.state, SyncState::Idle, "a failing account must not report healthy Idle");

    // And no messages were ingested (the seeded mailbox is empty; even if it
    // weren't, a failed login can't fetch).
    assert_eq!(message_count(&ctx.repos, account_id).await, 0, "no ingest on auth failure");

    adapter.stop_account(account_id).await.unwrap();
    core.abort();
    let _ = core.await;
}
