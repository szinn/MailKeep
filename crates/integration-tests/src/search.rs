//! End-to-end integration tests for the MK-20 full-text search backend.
//!
//! Unlike the `mk-search` component tests (which drive the index directly with
//! a fake store), these wire the REAL stack together — a real sqlite
//! `CoreServices`, a real filesystem `RawStorageService` (so bodies are
//! encrypted at rest and decrypted on demand), a real Tantivy index, and the
//! real `SearchSubsystem` indexer — and exercise the whole archive → index →
//! search path. Messages are seeded exactly as the ingest pipeline does: the
//! raw `.eml` bytes are stored via `raw_storage_service.put_if_absent` and the
//! metadata row is recorded via `record_parsed_message`, so the indexer's
//! decrypt-on-demand body extraction runs for real.
//!
//! sqlite + tempdirs only — no docker.

use std::{collections::HashSet, path::PathBuf, sync::Arc};

use chrono::Utc;
use mk_core::{
    CoreServices,
    account::{AccountId, CreateAccountParams},
    crypto::create_cipher_service,
    folder::{Folder, NewFolderRequest},
    imap::{ImapServerConfig, TlsMode},
    message::{MessageFlags, ParsedMessage},
    repository::{RepositoryService, transaction},
    test_support::default_external_services_builder,
    types::EmailAddress,
    user::{NewUser, User},
};
use mk_database::create_repository_service;
use mk_search::{SCHEMA_VERSION, SearchSubsystem, create_search_service, create_search_subsystem, open_search_index, read_version};
use mk_storage::create_filesystem_storage;
use sea_orm::Database;
use secrecy::SecretString;
use tempfile::TempDir;

/// A fully-wired search stack: real `CoreServices` (with a real search service
/// over a temp-dir Tantivy index), the matching indexer subsystem, and the
/// index directory path (so tests can inspect / corrupt the version sidecar).
struct SearchCtx {
    services: Arc<CoreServices>,
    repos: Arc<RepositoryService>,
    subsystem: SearchSubsystem,
    index_dir: PathBuf,
    // Kept alive for the test's duration: dropping these deletes the temp dirs.
    _storage_dir: TempDir,
    _index_dir: TempDir,
}

async fn setup() -> SearchCtx {
    let db = Database::connect("sqlite::memory:").await.unwrap();
    let repository_service = create_repository_service(db).await.unwrap();

    // Real filesystem storage so bodies are encrypted at rest and the indexer's
    // decrypt-on-demand read path runs for real.
    let cipher = create_cipher_service("integration-search-secret");
    let storage_dir = TempDir::new().unwrap();
    let storage = create_filesystem_storage(storage_dir.path(), cipher.clone()).await.unwrap();

    // Real Tantivy index + read service sharing one Arc.
    let index_dir = TempDir::new().unwrap();
    let index_path = index_dir.path().to_path_buf();
    let index = open_search_index(index_dir.path()).unwrap();
    let search_service = create_search_service(index.clone(), repository_service.clone());

    let core_services = mk_core::create_services(
        default_external_services_builder()
            .repository_service(repository_service.clone())
            .cipher_service(cipher)
            .raw_storage_service(storage.raw_storage_service.clone())
            .attachment_storage_service(storage.attachment_storage_service.clone())
            .search_service(search_service)
            .build()
            .unwrap(),
    )
    .unwrap();

    let subsystem = create_search_subsystem(&core_services, index.clone());

    SearchCtx {
        services: core_services,
        repos: repository_service,
        subsystem,
        index_dir: index_path,
        _storage_dir: storage_dir,
        _index_dir: index_dir,
    }
}

// ─── Fixtures ──────────────────────────────────────────────────────────────

async fn make_user(ctx: &SearchCtx, username: &str, email: &str) -> User {
    let new_user = NewUser::new(username, "password-hash", email, HashSet::new(), "Test User", false).unwrap();
    ctx.services.user_service.add_user(new_user).await.unwrap()
}

async fn make_account(ctx: &SearchCtx, user_id: u64, display_name: &str) -> AccountId {
    let params = CreateAccountParams {
        user_id,
        display_name: display_name.to_string(),
        email_address: EmailAddress::new(format!("{user_id}@example.com")).unwrap(),
        server: ImapServerConfig {
            host: "imap.example.com".to_string(),
            port: 993,
            tls: TlsMode::Tls,
        },
        username: format!("{user_id}@example.com"),
        password: SecretString::from("hunter2".to_string()),
    };
    ctx.services.account_service.create_account(params).await.unwrap().id
}

async fn make_folder(ctx: &SearchCtx, account_id: AccountId, path: &str) -> Folder {
    let req = NewFolderRequest {
        path: path.to_string(),
        display_name: None,
        special_use: None,
        uidvalidity: Some(1000),
    };
    ctx.services
        .folder_service
        .create_folders_for_account(account_id, vec![req])
        .await
        .unwrap()
        .into_iter()
        .next()
        .unwrap()
}

/// Store the raw `.eml` bytes (encrypted) and record the message row exactly as
/// the ingest pipeline would, so the indexer can later decrypt + parse it.
/// The subject is indexed from the DB row; the body is extracted from the
/// stored `.eml` by the indexer's mail-parser pass. Returns the message id.
async fn archive_message(ctx: &SearchCtx, account_id: AccountId, folder_id: u64, uid: u32, subject: &str, body: &str) -> u64 {
    let eml = format!("Subject: {subject}\r\nFrom: alice@example.com\r\nTo: bob@example.com\r\n\r\n{body}\r\n");
    let content_hash = ctx.services.raw_storage_service.put_if_absent(account_id, eml.as_bytes()).await.unwrap();

    let parsed = ParsedMessage {
        rfc822_message_id: format!("<{account_id}-{uid}@example.com>"),
        content_hash,
        subject: Some(subject.to_string()),
        from_address: EmailAddress::new("alice@example.com").unwrap(),
        from_name: Some("Alice".into()),
        to_addresses: vec![],
        cc_addresses: vec![],
        bcc_addresses: vec![],
        reply_to_addresses: vec![],
        sent_date: Some(Utc::now()),
        in_reply_to: None,
        references: vec![],
        snippet: body.chars().take(60).collect(),
        size_bytes: eml.len() as i64,
        attachments: vec![],
    };
    ctx.services
        .message_service
        .record_parsed_message(account_id, folder_id, uid, 1000, Utc::now(), MessageFlags::default(), parsed)
        .await
        .unwrap()
        .message_id
}

async fn unindexed_count(ctx: &SearchCtx) -> usize {
    transaction(&**ctx.repos.repository(), |tx| {
        let r = ctx.repos.message_repository().clone();
        Box::pin(async move { r.list_unindexed(tx, 1000).await })
    })
    .await
    .unwrap()
    .len()
}

async fn reset_all_indexed(ctx: &SearchCtx) {
    transaction(&**ctx.repos.repository(), |tx| {
        let r = ctx.repos.message_repository().clone();
        Box::pin(async move { r.reset_all_indexed(tx).await })
    })
    .await
    .unwrap();
}

// ─── Test 1: end-to-end archive → index → search ──────────────────────────

#[tokio::test]
async fn end_to_end_archive_index_search() {
    let ctx = setup().await;
    let user = make_user(&ctx, "alice", "alice@example.com").await;
    let account = make_account(&ctx, user.id, "Primary").await;
    let folder = make_folder(&ctx, account, "INBOX").await;

    let invoice = archive_message(&ctx, account, folder.id, 1, "Invoice for March", "payment due next week").await;
    let news = archive_message(&ctx, account, folder.id, 2, "Weekly newsletter", "gardening tips and recipes").await;

    // Nothing indexed yet; then drive the real indexer to convergence.
    assert_eq!(unindexed_count(&ctx).await, 2, "both messages start unindexed");
    ctx.subsystem.reindex_to_idle().await.unwrap();
    assert_eq!(unindexed_count(&ctx).await, 0, "drain flags every message indexed");

    // Subject term.
    let by_subject = ctx.services.search_service.search(user.id, "invoice", 10, 0).await.unwrap();
    assert_eq!(by_subject.hits.iter().map(|h| h.message_id).collect::<Vec<_>>(), vec![invoice]);

    // Body term (proves decrypt-on-demand + mail-parser body extraction ran).
    let by_body = ctx.services.search_service.search(user.id, "gardening", 10, 0).await.unwrap();
    assert_eq!(by_body.hits.len(), 1, "body-only term matches exactly the newsletter");
    assert_eq!(by_body.hits[0].message_id, news);
}

// ─── Test 2: BM25 ranking — subject boost ─────────────────────────────────

#[tokio::test]
async fn subject_match_outranks_body_match() {
    let ctx = setup().await;
    let user = make_user(&ctx, "alice", "alice@example.com").await;
    let account = make_account(&ctx, user.id, "Primary").await;
    let folder = make_folder(&ctx, account, "INBOX").await;

    let in_subject = archive_message(&ctx, account, folder.id, 1, "Quarterly running report", "unrelated content").await;
    let in_body = archive_message(&ctx, account, folder.id, 2, "Weekly digest", "the running totals are attached").await;

    ctx.subsystem.reindex_to_idle().await.unwrap();

    let results = ctx.services.search_service.search(user.id, "running", 10, 0).await.unwrap();
    assert_eq!(results.hits.len(), 2, "both match the bare term");
    assert_eq!(results.hits[0].message_id, in_subject, "subject match ranks first");
    assert_eq!(results.hits[1].message_id, in_body);
    assert!(results.hits[0].score > results.hits[1].score, "subject boost raises the score");
}

// ─── Test 3: per-user scoping isolation ───────────────────────────────────

#[tokio::test]
async fn search_is_isolated_per_user() {
    let ctx = setup().await;
    let alice = make_user(&ctx, "alice", "alice@example.com").await;
    let bob = make_user(&ctx, "bob", "bob@example.com").await;
    // Colliding account display name across the two users.
    let alice_account = make_account(&ctx, alice.id, "Mail").await;
    let bob_account = make_account(&ctx, bob.id, "Mail").await;
    let alice_folder = make_folder(&ctx, alice_account, "INBOX").await;
    let bob_folder = make_folder(&ctx, bob_account, "INBOX").await;

    // Byte-identical messages under each user (same content_hash, different
    // account) — so the isolation cannot be an artifact of differing content.
    let alice_msg = archive_message(&ctx, alice_account, alice_folder.id, 1, "secret plan", "confidential body").await;
    let bob_msg = archive_message(&ctx, bob_account, bob_folder.id, 1, "secret plan", "confidential body").await;

    ctx.subsystem.reindex_to_idle().await.unwrap();

    // Alice's bare search returns only Alice's message.
    let alice_view = ctx.services.search_service.search(alice.id, "secret", 10, 0).await.unwrap();
    assert_eq!(alice_view.hits.iter().map(|h| h.message_id).collect::<Vec<_>>(), vec![alice_msg]);
    assert!(alice_view.hits.iter().all(|h| h.account_id == alice_account));

    // Both directions: Bob's identical message WAS indexed and is findable by
    // Bob — proving the isolation above is a real scope boundary, not Bob's row
    // silently failing to index.
    let bob_view = ctx.services.search_service.search(bob.id, "secret", 10, 0).await.unwrap();
    assert_eq!(bob_view.hits.iter().map(|h| h.message_id).collect::<Vec<_>>(), vec![bob_msg]);
    assert!(bob_view.hits.iter().all(|h| h.account_id == bob_account));

    // Naming the account "Mail" resolves within Alice's own accounts only — it
    // can never reach Bob's identically-named account.
    let hinted = ctx.services.search_service.search(alice.id, "account:Mail secret", 10, 0).await.unwrap();
    assert_eq!(hinted.hits.iter().map(|h| h.message_id).collect::<Vec<_>>(), vec![alice_msg]);
}

// ─── Test 4: folder: post-filter ──────────────────────────────────────────

#[tokio::test]
async fn folder_filter_includes_and_excludes() {
    let ctx = setup().await;
    let user = make_user(&ctx, "alice", "alice@example.com").await;
    let account = make_account(&ctx, user.id, "Primary").await;
    let inbox = make_folder(&ctx, account, "INBOX").await;
    let archive = make_folder(&ctx, account, "Archive").await;

    let inbox_msg = archive_message(&ctx, account, inbox.id, 1, "meeting notes", "agenda for monday").await;
    let archive_msg = archive_message(&ctx, account, archive.id, 2, "meeting notes", "old agenda archived").await;

    ctx.subsystem.reindex_to_idle().await.unwrap();

    // Both match without a folder constraint.
    let all = ctx.services.search_service.search(user.id, "meeting", 10, 0).await.unwrap();
    assert_eq!(all.hits.len(), 2);

    // folder:INBOX keeps only the in-folder message.
    let only_inbox = ctx.services.search_service.search(user.id, "meeting folder:INBOX", 10, 0).await.unwrap();
    assert_eq!(only_inbox.hits.iter().map(|h| h.message_id).collect::<Vec<_>>(), vec![inbox_msg]);

    // Negated folder excludes it.
    let not_inbox = ctx.services.search_service.search(user.id, "meeting !folder:INBOX", 10, 0).await.unwrap();
    assert_eq!(not_inbox.hits.iter().map(|h| h.message_id).collect::<Vec<_>>(), vec![archive_msg]);
}

// ─── Test 5: crash-idempotency (commit then re-drain) ─────────────────────

#[tokio::test]
async fn reindexing_after_torn_write_produces_no_duplicate() {
    let ctx = setup().await;
    let user = make_user(&ctx, "alice", "alice@example.com").await;
    let account = make_account(&ctx, user.id, "Primary").await;
    let folder = make_folder(&ctx, account, "INBOX").await;

    let msg = archive_message(&ctx, account, folder.id, 1, "unique subject", "unique body").await;
    ctx.subsystem.reindex_to_idle().await.unwrap();

    let first = ctx.services.search_service.search(user.id, "unique", 10, 0).await.unwrap();
    assert_eq!(first.hits.len(), 1);

    // Simulate a torn write: the doc is committed but the watermark was never
    // flipped (or a schema-independent re-drain occurs). Re-queue the row and
    // drain again — delete-before-add must yield exactly one document, not two.
    reset_all_indexed(&ctx).await;
    assert_eq!(unindexed_count(&ctx).await, 1);
    ctx.subsystem.reindex_to_idle().await.unwrap();

    let after = ctx.services.search_service.search(user.id, "unique", 10, 0).await.unwrap();
    assert_eq!(after.hits.len(), 1, "re-indexing must not duplicate the document");
    assert_eq!(after.hits[0].message_id, msg);
    assert_eq!(unindexed_count(&ctx).await, 0, "row ends indexed");
}

// ─── Test 6: schema-version rebuild ───────────────────────────────────────

#[tokio::test]
async fn stale_schema_version_triggers_full_rebuild() {
    let ctx = setup().await;
    let user = make_user(&ctx, "alice", "alice@example.com").await;
    let account = make_account(&ctx, user.id, "Primary").await;
    let folder = make_folder(&ctx, account, "INBOX").await;

    let msg = archive_message(&ctx, account, folder.id, 1, "rebuildable subject", "rebuildable body").await;
    ctx.subsystem.reindex_to_idle().await.unwrap();
    assert_eq!(read_version(&ctx.index_dir), Some(SCHEMA_VERSION), "version written after first index");

    // Corrupt the sidecar to a stale version, forcing a rebuild on the next run.
    std::fs::write(ctx.index_dir.join("schema_version"), b"0").unwrap();

    // Reconcile clears the index, re-queues every row, rebuilds from the DB, and
    // rewrites the current version.
    ctx.subsystem.reindex_to_idle().await.unwrap();

    let results = ctx.services.search_service.search(user.id, "rebuildable", 10, 0).await.unwrap();
    assert_eq!(results.hits.len(), 1, "search works after a full rebuild");
    assert_eq!(results.hits[0].message_id, msg);
    assert_eq!(read_version(&ctx.index_dir), Some(SCHEMA_VERSION), "sidecar restored to current version");
    assert_eq!(unindexed_count(&ctx).await, 0);
}
