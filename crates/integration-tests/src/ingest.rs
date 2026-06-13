//! End-to-end integration tests for the ingest pipeline.
//!
//! These tests wire real filesystem storage + sqlite::memory: + the real job
//! worker to prove the full raw-.eml → encrypted storage → DB rows path.

use std::{sync::Arc, time::Duration};

use bytes::Bytes;
use chrono::Utc;
use mk_core::{
    ExternalServicesBuilder,
    account::{AccountId, CreateAccountParams},
    create_core_subsystem,
    folder::{Folder, NewFolderRequest, SpecialUse},
    imap::{ImapServerConfig, TlsMode},
    ingest::IngestRequest,
    message::{MessageFlags, MessageId},
    repository::{RepositoryService, transaction},
    test_support::test_cipher_service,
    types::{ContentHash, EmailAddress},
    user::{NewUser, User},
};
use mk_database::create_repository_service;
use sea_orm::Database;
use secrecy::SecretString;
use tempfile::TempDir;
use tokio_graceful_shutdown::{IntoSubsystem, SubsystemBuilder, SubsystemHandle, Toplevel};

use crate::context::TestContext;

// ─── Harness ───────────────────────────────────────────────────────────────

async fn setup_fs() -> TestContext {
    let dir = TempDir::new().unwrap();
    let cipher = test_cipher_service();
    let storage = mk_storage::create_filesystem_storage(dir.path(), cipher.clone()).await.unwrap();

    let db = Database::connect("sqlite::memory:").await.unwrap();
    let repository_service = create_repository_service(db).await.unwrap();

    let core = mk_core::create_services(
        ExternalServicesBuilder::default()
            .repository_service(repository_service.clone())
            .cipher_service(cipher)
            .raw_storage_service(storage.raw_storage_service)
            .attachment_storage_service(storage.attachment_storage_service)
            .job_concurrency(1)
            .build()
            .unwrap(),
    )
    .unwrap();

    mk_parser::register_handlers(&core);

    // `dir` is stored in the handle slot to keep the temp directory alive.
    TestContext::new(core, repository_service, dir)
}

// ─── Fixture helpers ────────────────────────────────────────────────────────

async fn make_user(ctx: &TestContext) -> User {
    let new_user = NewUser::new("alice", "password-hash", "alice@example.com", std::collections::HashSet::new(), "Alice", false).unwrap();
    ctx.services.user_service.add_user(new_user).await.unwrap()
}

async fn make_account(ctx: &TestContext, user_id: u64) -> AccountId {
    let params = CreateAccountParams {
        user_id,
        display_name: "Test Account".into(),
        email_address: EmailAddress::new("user@example.com").unwrap(),
        server: ImapServerConfig {
            host: "example.com".into(),
            port: 993,
            tls: TlsMode::Tls,
        },
        username: "user@example.com".into(),
        password: SecretString::from("hunter2".to_string()),
    };
    ctx.services.account_service.create_account(params).await.unwrap().id
}

async fn make_folder(ctx: &TestContext, account_id: AccountId, path: &str, special_use: Option<SpecialUse>) -> Folder {
    ctx.services
        .folder_service
        .create_folders_for_account(
            account_id,
            vec![NewFolderRequest {
                path: path.into(),
                display_name: None,
                special_use,
                uidvalidity: Some(1000),
            }],
        )
        .await
        .unwrap()
        .into_iter()
        .next()
        .unwrap()
}

fn run_core(ctx: &TestContext) -> tokio::task::JoinHandle<()> {
    let core = ctx.services.clone();
    tokio::spawn(async move {
        Toplevel::new(async move |s: &mut SubsystemHandle| {
            s.start(SubsystemBuilder::new("Core", create_core_subsystem(&core).into_subsystem()));
        })
        .handle_shutdown_requests(Duration::from_secs(5))
        .await
        .unwrap();
    })
}

// ─── DB query helpers ───────────────────────────────────────────────────────

async fn message_count(repos: &Arc<RepositoryService>, account_id: AccountId) -> usize {
    transaction(&**repos.repository(), |tx| {
        let r = repos.message_repository().clone();
        Box::pin(async move { r.list_for_account(tx, account_id, 1000, 0).await })
    })
    .await
    .unwrap()
    .len()
}

async fn list_messages(repos: &Arc<RepositoryService>, account_id: AccountId) -> Vec<mk_core::message::Message> {
    transaction(&**repos.repository(), |tx| {
        let r = repos.message_repository().clone();
        Box::pin(async move { r.list_for_account(tx, account_id, 1000, 0).await })
    })
    .await
    .unwrap()
}

async fn list_attachments(repos: &Arc<RepositoryService>, message_id: MessageId) -> Vec<mk_core::message::MessageAttachment> {
    transaction(&**repos.repository(), |tx| {
        let r = repos.message_attachment_repository().clone();
        Box::pin(async move { r.list_for_message(tx, message_id).await })
    })
    .await
    .unwrap()
}

async fn location_exists(repos: &Arc<RepositoryService>, message_id: MessageId, folder_id: u64) -> bool {
    transaction(&**repos.repository(), |tx| {
        let r = repos.message_location_repository().clone();
        Box::pin(async move { r.find_by_message_and_folder(tx, message_id, folder_id).await })
    })
    .await
    .unwrap()
    .is_some()
}

/// Poll until the job queue has no pending or running jobs (i.e. all enqueued
/// work has drained), or panic after ~5 s.
async fn wait_for_jobs_drained(repos: &Arc<RepositoryService>) {
    for _ in 0..200 {
        let pending = {
            let r = repos.job_repository().clone();
            transaction(&**repos.repository(), |tx| {
                let r = r.clone();
                Box::pin(async move { r.count_all_pending(tx).await })
            })
            .await
            .unwrap()
        };
        if pending == 0 {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    panic!("jobs did not drain within timeout");
}

// ─── Fixtures ────────────────────────────────────────────────────────────────

const SIMPLE_ATTACHMENT: &[u8] = include_bytes!("../fixtures/simple_attachment.eml");
const NO_FROM: &[u8] = include_bytes!("../fixtures/no_from.eml");
const DEDUP_A: &[u8] = include_bytes!("../fixtures/dedup_a.eml");
const DEDUP_B: &[u8] = include_bytes!("../fixtures/dedup_b.eml");

// ─── Tests ───────────────────────────────────────────────────────────────────

/// Ingest a multipart message with one attachment; assert the raw blob
/// round-trips via storage and a Message + MessageAttachment + MessageLocation
/// row appear in the DB.
#[tokio::test]
async fn ingest_parses_into_rows_and_blobs() {
    let ctx = setup_fs().await;
    let user = make_user(&ctx).await;
    let account_id = make_account(&ctx, user.id).await;
    let folder = make_folder(&ctx, account_id, "INBOX", Some(SpecialUse::Inbox)).await;

    let handle = run_core(&ctx);

    let result = ctx
        .services
        .ingest_service
        .ingest_raw(IngestRequest {
            account_id,
            folder_id: folder.id,
            uid: 1,
            uidvalidity: 1000,
            internal_date: Utc::now(),
            flags: MessageFlags::default(),
            raw_bytes: Bytes::from_static(SIMPLE_ATTACHMENT),
        })
        .await
        .unwrap();

    // Raw blob is retrievable (decrypted) using the returned content hash.
    let back = ctx.services.raw_storage_service.get(account_id, &result.content_hash).await.unwrap();
    assert_eq!(back.as_slice(), SIMPLE_ATTACHMENT);

    // Poll until the background parse job produces a message row.
    let mut recorded = false;
    for _ in 0..100 {
        if message_count(&ctx.repos, account_id).await == 1 {
            recorded = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(recorded, "parse job should produce a message row within ~5s");

    let msgs = list_messages(&ctx.repos, account_id).await;
    assert_eq!(msgs.len(), 1);
    let message_id = msgs[0].id;

    // One attachment row linked to this message.
    let attachments = list_attachments(&ctx.repos, message_id).await;
    assert_eq!(attachments.len(), 1, "simple_attachment.eml has one attachment");

    // A MessageLocation in the folder.
    assert!(location_exists(&ctx.repos, message_id, folder.id).await, "location in INBOX");

    // The attachment blob is retrievable from storage.
    let att_hash = attachments[0].content_hash;
    let att_bytes = ctx.services.attachment_storage_service.get(account_id, &att_hash).await.unwrap();
    assert!(!att_bytes.is_empty(), "attachment blob must be retrievable");

    handle.abort();
    let _ = handle.await;
}

/// Two different messages embedding the SAME attachment bytes produce one
/// shared blob (dedup via `put_if_absent`) and two `MessageAttachment` rows
/// pointing at the same `content_hash`.
#[tokio::test]
async fn dedup_shared_attachment() {
    let ctx = setup_fs().await;
    let user = make_user(&ctx).await;
    let account_id = make_account(&ctx, user.id).await;
    let folder = make_folder(&ctx, account_id, "INBOX", Some(SpecialUse::Inbox)).await;

    let handle = run_core(&ctx);

    ctx.services
        .ingest_service
        .ingest_raw(IngestRequest {
            account_id,
            folder_id: folder.id,
            uid: 1,
            uidvalidity: 1000,
            internal_date: Utc::now(),
            flags: MessageFlags::default(),
            raw_bytes: Bytes::from_static(DEDUP_A),
        })
        .await
        .unwrap();

    ctx.services
        .ingest_service
        .ingest_raw(IngestRequest {
            account_id,
            folder_id: folder.id,
            uid: 2,
            uidvalidity: 1000,
            internal_date: Utc::now(),
            flags: MessageFlags::default(),
            raw_bytes: Bytes::from_static(DEDUP_B),
        })
        .await
        .unwrap();

    // Wait for both parse jobs to complete.
    let mut ok = false;
    for _ in 0..100 {
        if message_count(&ctx.repos, account_id).await == 2 {
            ok = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(ok, "both parse jobs should complete within ~5s");

    let msgs = list_messages(&ctx.repos, account_id).await;
    assert_eq!(msgs.len(), 2);

    // Collect attachment hashes for both messages.
    let atts_a = list_attachments(&ctx.repos, msgs[0].id).await;
    let atts_b = list_attachments(&ctx.repos, msgs[1].id).await;
    assert_eq!(atts_a.len(), 1, "message A should have one attachment");
    assert_eq!(atts_b.len(), 1, "message B should have one attachment");

    let hash_a: ContentHash = atts_a[0].content_hash;
    let hash_b: ContentHash = atts_b[0].content_hash;
    assert_eq!(hash_a, hash_b, "both messages share the same attachment blob");

    // The single blob is retrievable.
    let blob = ctx.services.attachment_storage_service.get(account_id, &hash_a).await.unwrap();
    assert!(!blob.is_empty(), "shared attachment blob must be retrievable from storage");

    handle.abort();
    let _ = handle.await;
}

/// Ingesting the same raw bytes into two different folders produces one Message
/// row and one MessageLocation per folder.
#[tokio::test]
async fn two_folders_one_message() {
    let ctx = setup_fs().await;
    let user = make_user(&ctx).await;
    let account_id = make_account(&ctx, user.id).await;
    let inbox = make_folder(&ctx, account_id, "INBOX", Some(SpecialUse::Inbox)).await;
    let archive = make_folder(&ctx, account_id, "Archive", Some(SpecialUse::Archive)).await;

    let handle = run_core(&ctx);

    // Ingest the same bytes into INBOX (uid=1).
    ctx.services
        .ingest_service
        .ingest_raw(IngestRequest {
            account_id,
            folder_id: inbox.id,
            uid: 1,
            uidvalidity: 1000,
            internal_date: Utc::now(),
            flags: MessageFlags::default(),
            raw_bytes: Bytes::from_static(SIMPLE_ATTACHMENT),
        })
        .await
        .unwrap();

    // Ingest the same bytes into Archive (uid=2 — different uid, same content).
    ctx.services
        .ingest_service
        .ingest_raw(IngestRequest {
            account_id,
            folder_id: archive.id,
            uid: 2,
            uidvalidity: 1000,
            internal_date: Utc::now(),
            flags: MessageFlags::default(),
            raw_bytes: Bytes::from_static(SIMPLE_ATTACHMENT),
        })
        .await
        .unwrap();

    // Wait until both parse jobs have fully drained before asserting counts.
    wait_for_jobs_drained(&ctx.repos).await;

    assert_eq!(message_count(&ctx.repos, account_id).await, 1, "same message content → single Message row");

    let msgs = list_messages(&ctx.repos, account_id).await;
    let message_id = msgs[0].id;

    assert!(location_exists(&ctx.repos, message_id, inbox.id).await, "location in INBOX");
    assert!(location_exists(&ctx.repos, message_id, archive.id).await, "location in Archive");

    handle.abort();
    let _ = handle.await;
}

/// Ingesting the same raw bytes into the same folder twice produces exactly
/// one Message row (idempotent by `rfc822_message_id` + `content_hash`).
#[tokio::test]
async fn idempotent_reingest() {
    let ctx = setup_fs().await;
    let user = make_user(&ctx).await;
    let account_id = make_account(&ctx, user.id).await;
    let folder = make_folder(&ctx, account_id, "INBOX", Some(SpecialUse::Inbox)).await;

    let handle = run_core(&ctx);

    // Ingest twice with same uid — should be idempotent.
    ctx.services
        .ingest_service
        .ingest_raw(IngestRequest {
            account_id,
            folder_id: folder.id,
            uid: 1,
            uidvalidity: 1000,
            internal_date: Utc::now(),
            flags: MessageFlags::default(),
            raw_bytes: Bytes::from_static(SIMPLE_ATTACHMENT),
        })
        .await
        .unwrap();

    ctx.services
        .ingest_service
        .ingest_raw(IngestRequest {
            account_id,
            folder_id: folder.id,
            uid: 1,
            uidvalidity: 1000,
            internal_date: Utc::now(),
            flags: MessageFlags::default(),
            raw_bytes: Bytes::from_static(SIMPLE_ATTACHMENT),
        })
        .await
        .unwrap();

    // Wait until both jobs have fully drained before asserting the count.
    wait_for_jobs_drained(&ctx.repos).await;

    assert_eq!(
        message_count(&ctx.repos, account_id).await,
        1,
        "idempotent reingest must not create duplicate message row"
    );

    handle.abort();
    let _ = handle.await;
}

/// A message with no `From:` header is recorded with the sentinel address.
#[tokio::test]
async fn missing_from_uses_sentinel() {
    let ctx = setup_fs().await;
    let user = make_user(&ctx).await;
    let account_id = make_account(&ctx, user.id).await;
    let folder = make_folder(&ctx, account_id, "INBOX", Some(SpecialUse::Inbox)).await;

    let handle = run_core(&ctx);

    ctx.services
        .ingest_service
        .ingest_raw(IngestRequest {
            account_id,
            folder_id: folder.id,
            uid: 1,
            uidvalidity: 1000,
            internal_date: Utc::now(),
            flags: MessageFlags::default(),
            raw_bytes: Bytes::from_static(NO_FROM),
        })
        .await
        .unwrap();

    let mut ok = false;
    for _ in 0..100 {
        if message_count(&ctx.repos, account_id).await == 1 {
            ok = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(ok, "parse job should produce a message row within ~5s");

    let msgs = list_messages(&ctx.repos, account_id).await;
    assert_eq!(msgs.len(), 1);
    assert_eq!(
        msgs[0].from_address.as_str(),
        "unknown@mailkeep.invalid",
        "missing From must use the sentinel address"
    );

    handle.abort();
    let _ = handle.await;
}
