//! Integration tests for folder + message cascade semantics, UIDVALIDITY
//! rollover, and the two-folder same-message case.
//!
//! These tests run against a real sqlite::memory: database (full schema +
//! migrations) so they exercise the actual FK cascades and unique constraints
//! defined by the MK-4 migrations — not mocks.

use std::{collections::HashSet, sync::Arc};

use async_trait::async_trait;
use chrono::Utc;
use mk_core::{
    Error, ExternalServicesBuilder,
    account::{AccountId, CreateAccountParams},
    folder::{Folder, NewFolderRequest, SpecialUse},
    imap::{ImapServerConfig, TlsMode},
    message::{MessageFlags, ParsedAttachment, ParsedMessage},
    repository::{RepositoryService, transaction},
    storage::{AttachmentStorageService, RawStorageService},
    test_support::test_cipher_service,
    types::{ContentHash, EmailAddress},
    user::{NewUser, User},
};
use mk_database::create_repository_service;
use sea_orm::Database;
use secrecy::SecretString;

use crate::context::TestContext;

// ─── Local test harness: storage stubs that return Ok ─────────────────────
//
// The default `test_support::default_external_services_builder()` wires
// `NopRawStorage` / `NopAttachmentStorage` which `unimplemented!()` on every
// call. `AccountService::delete_account` calls `delete_account` on both
// storage services after the DB commit, so we need stubs that succeed.

struct OkRawStorage;

#[async_trait]
impl RawStorageService for OkRawStorage {
    async fn put_if_absent(&self, _account_id: AccountId, _plaintext: &[u8]) -> Result<ContentHash, Error> {
        unimplemented!("OkRawStorage: only delete_account is exercised by these tests")
    }
    async fn get(&self, _account_id: AccountId, _key: &ContentHash) -> Result<Vec<u8>, Error> {
        unimplemented!("OkRawStorage: only delete_account is exercised by these tests")
    }
    async fn exists(&self, _account_id: AccountId, _key: &ContentHash) -> Result<bool, Error> {
        unimplemented!("OkRawStorage: only delete_account is exercised by these tests")
    }
    async fn delete_account(&self, _account_id: AccountId) -> Result<(), Error> {
        Ok(())
    }
}

struct OkAttachmentStorage;

#[async_trait]
impl AttachmentStorageService for OkAttachmentStorage {
    async fn put_if_absent(&self, _account_id: AccountId, _plaintext: &[u8]) -> Result<ContentHash, Error> {
        unimplemented!("OkAttachmentStorage: only delete_account is exercised by these tests")
    }
    async fn get(&self, _account_id: AccountId, _key: &ContentHash) -> Result<Vec<u8>, Error> {
        unimplemented!("OkAttachmentStorage: only delete_account is exercised by these tests")
    }
    async fn exists(&self, _account_id: AccountId, _key: &ContentHash) -> Result<bool, Error> {
        unimplemented!("OkAttachmentStorage: only delete_account is exercised by these tests")
    }
    async fn delete_account(&self, _account_id: AccountId) -> Result<(), Error> {
        Ok(())
    }
}

/// Same shape as `crate::sqlite::setup()` but uses the Ok storage stubs so
/// `AccountService::delete_account` doesn't panic post-commit.
async fn setup() -> TestContext {
    let db = Database::connect("sqlite::memory:").await.unwrap();
    let repository_service = create_repository_service(db).await.unwrap();
    let core_services = mk_core::create_services(
        ExternalServicesBuilder::default()
            .repository_service(repository_service.clone())
            .cipher_service(test_cipher_service())
            .raw_storage_service(Arc::new(OkRawStorage) as Arc<dyn RawStorageService>)
            .attachment_storage_service(Arc::new(OkAttachmentStorage) as Arc<dyn AttachmentStorageService>)
            .job_concurrency(1)
            .build()
            .unwrap(),
    )
    .unwrap();

    TestContext::new(core_services, repository_service, ())
}

// ─── Fixtures ──────────────────────────────────────────────────────────────

async fn make_user(ctx: &TestContext, username: &str, email: &str) -> User {
    let new_user = NewUser::new(username, "password-hash", email, HashSet::new(), "Test User", false).unwrap();
    ctx.services.user_service.add_user(new_user).await.unwrap()
}

async fn make_account(ctx: &TestContext, user_id: u64, host: &str) -> AccountId {
    let params = CreateAccountParams {
        user_id,
        display_name: format!("{host} Account"),
        email_address: EmailAddress::new(format!("user@{host}")).unwrap(),
        server: ImapServerConfig {
            host: host.to_string(),
            port: 993,
            tls: TlsMode::Tls,
        },
        username: format!("user@{host}"),
        password: SecretString::from("hunter2".to_string()),
    };
    ctx.services.account_service.create_account(params).await.unwrap().id
}

async fn make_folder(ctx: &TestContext, account_id: AccountId, path: &str, special_use: Option<SpecialUse>) -> Folder {
    let req = NewFolderRequest {
        path: path.to_string(),
        display_name: None,
        special_use,
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

fn sample_parsed_message() -> ParsedMessage {
    ParsedMessage {
        rfc822_message_id: "<abc@example.com>".into(),
        content_hash: ContentHash::from_hex("a".repeat(64)).unwrap(),
        subject: Some("Hello".into()),
        from_address: EmailAddress::new("alice@example.com").unwrap(),
        from_name: Some("Alice".into()),
        to_addresses: vec![],
        cc_addresses: vec![],
        bcc_addresses: vec![],
        reply_to_addresses: vec![],
        sent_date: Some(Utc::now()),
        in_reply_to: None,
        references: vec![],
        snippet: "Hello there".into(),
        size_bytes: 1024,
        attachments: vec![ParsedAttachment {
            content_hash: ContentHash::from_hex("c".repeat(64)).unwrap(),
            filename: Some("file.pdf".into()),
            content_type: "application/pdf".into(),
            size_bytes: 2048,
            is_inline: false,
            content_id: None,
        }],
    }
}

// ─── Counting helpers (use repos directly) ─────────────────────────────────

async fn count_folders_for_account(repos: &Arc<RepositoryService>, account_id: AccountId) -> usize {
    transaction(&**repos.repository(), |tx| {
        let r = repos.folder_repository().clone();
        Box::pin(async move { r.list_for_account(tx, account_id).await })
    })
    .await
    .unwrap()
    .len()
}

async fn count_messages_for_account(repos: &Arc<RepositoryService>, account_id: AccountId) -> usize {
    transaction(&**repos.repository(), |tx| {
        let r = repos.message_repository().clone();
        Box::pin(async move { r.list_for_account(tx, account_id, 1000, 0).await })
    })
    .await
    .unwrap()
    .len()
}

async fn count_attachments_for_message(repos: &Arc<RepositoryService>, message_id: u64) -> usize {
    transaction(&**repos.repository(), |tx| {
        let r = repos.message_attachment_repository().clone();
        Box::pin(async move { r.list_for_message(tx, message_id).await })
    })
    .await
    .unwrap()
    .len()
}

async fn location_exists(repos: &Arc<RepositoryService>, message_id: u64, folder_id: u64) -> bool {
    transaction(&**repos.repository(), |tx| {
        let r = repos.message_location_repository().clone();
        Box::pin(async move { r.find_by_message_and_folder(tx, message_id, folder_id).await })
    })
    .await
    .unwrap()
    .is_some()
}

// ─── Test 1: cascade on account delete ─────────────────────────────────────

#[tokio::test]
async fn cascade_on_account_delete_removes_all_child_tables() {
    let ctx = setup().await;
    let user = make_user(&ctx, "alice", "alice@example.com").await;
    let account_id = make_account(&ctx, user.id, "example.com").await;
    let folder = make_folder(&ctx, account_id, "INBOX", Some(SpecialUse::Inbox)).await;

    let recorded = ctx
        .services
        .message_service
        .record_parsed_message(account_id, folder.id, 1, 1000, Utc::now(), MessageFlags::default(), sample_parsed_message())
        .await
        .unwrap();
    assert!(recorded.created);
    let message_id = recorded.message_id;

    // Sanity: 1 folder, 1 message, 1 location, 1 attachment.
    assert_eq!(count_folders_for_account(&ctx.repos, account_id).await, 1);
    assert_eq!(count_messages_for_account(&ctx.repos, account_id).await, 1);
    assert_eq!(count_attachments_for_message(&ctx.repos, message_id).await, 1);
    assert!(location_exists(&ctx.repos, message_id, folder.id).await);

    // Delete the account.
    ctx.services.account_service.delete_account(user.id, account_id).await.unwrap();

    // All child rows must cascade.
    assert_eq!(count_folders_for_account(&ctx.repos, account_id).await, 0, "folders should cascade");
    assert_eq!(count_messages_for_account(&ctx.repos, account_id).await, 0, "messages should cascade");
    assert_eq!(count_attachments_for_message(&ctx.repos, message_id).await, 0, "attachments should cascade");
    assert!(!location_exists(&ctx.repos, message_id, folder.id).await, "locations should cascade");
}

// ─── Test 2: UIDVALIDITY rollover drops only locations ────────────────────

#[tokio::test]
async fn uidvalidity_rollover_drops_locations_only() {
    let ctx = setup().await;
    let user = make_user(&ctx, "bob", "bob@example.com").await;
    let account_id = make_account(&ctx, user.id, "example.com").await;
    let folder = make_folder(&ctx, account_id, "INBOX", Some(SpecialUse::Inbox)).await;

    // Initial ingest at uid=1, uidvalidity=1000.
    let recorded = ctx
        .services
        .message_service
        .record_parsed_message(account_id, folder.id, 1, 1000, Utc::now(), MessageFlags::default(), sample_parsed_message())
        .await
        .unwrap();
    assert!(recorded.created);
    let message_id = recorded.message_id;

    assert_eq!(count_messages_for_account(&ctx.repos, account_id).await, 1);
    assert_eq!(count_attachments_for_message(&ctx.repos, message_id).await, 1);
    assert!(location_exists(&ctx.repos, message_id, folder.id).await);

    // Drop all locations for this folder (simulates UIDVALIDITY change).
    let deleted = ctx.services.message_service.delete_locations_for_folder(folder.id).await.unwrap();
    assert_eq!(deleted, 1);

    // Message + attachment remain; only the location is gone.
    assert_eq!(count_messages_for_account(&ctx.repos, account_id).await, 1, "message must remain");
    assert_eq!(count_attachments_for_message(&ctx.repos, message_id).await, 1, "attachment must remain");
    assert!(!location_exists(&ctx.repos, message_id, folder.id).await, "location should be gone");

    // Re-ingest the SAME ParsedMessage at the NEW uid + uidvalidity.
    let recorded2 = ctx
        .services
        .message_service
        .record_parsed_message(account_id, folder.id, 999, 2000, Utc::now(), MessageFlags::default(), sample_parsed_message())
        .await
        .unwrap();
    // Message row exists (idempotency by rfc822_message_id), so created==false.
    assert!(!recorded2.created, "idempotent re-ingest after location reset");
    assert_eq!(recorded2.message_id, message_id);

    // Still 1 message, 1 attachment (unchanged), and a fresh location with the
    // new uid + uidvalidity.
    assert_eq!(count_messages_for_account(&ctx.repos, account_id).await, 1);
    assert_eq!(count_attachments_for_message(&ctx.repos, message_id).await, 1);

    let new_location = transaction(&**ctx.repos.repository(), |tx| {
        let r = ctx.repos.message_location_repository().clone();
        Box::pin(async move { r.find_by_message_and_folder(tx, message_id, folder.id).await })
    })
    .await
    .unwrap()
    .expect("location should exist after re-ingest");

    assert_eq!(new_location.uid, 999);
    assert_eq!(new_location.uidvalidity, 2000);
}

// ─── Test 3: two folders share the same message ───────────────────────────

#[tokio::test]
async fn two_folders_share_message() {
    let ctx = setup().await;
    let user = make_user(&ctx, "carol", "carol@example.com").await;
    let account_id = make_account(&ctx, user.id, "example.com").await;
    let inbox = make_folder(&ctx, account_id, "INBOX", Some(SpecialUse::Inbox)).await;
    let archive = make_folder(&ctx, account_id, "Archive", Some(SpecialUse::Archive)).await;

    let parsed = sample_parsed_message();
    let expected_attachments = parsed.attachments.len();

    // First ingest into INBOX → fresh insert.
    let r1 = ctx
        .services
        .message_service
        .record_parsed_message(account_id, inbox.id, 1, 1000, Utc::now(), MessageFlags::default(), parsed.clone())
        .await
        .unwrap();
    assert!(r1.created, "first ingest should create the message");

    // Same message into Archive → existing-message path, just adds a location.
    let r2 = ctx
        .services
        .message_service
        .record_parsed_message(account_id, archive.id, 2, 1000, Utc::now(), MessageFlags::default(), parsed)
        .await
        .unwrap();
    assert!(!r2.created, "second ingest must reuse the existing message");
    assert_eq!(r1.message_id, r2.message_id, "both ingests should share message_id");

    let message_id = r1.message_id;

    // 1 message, 2 locations (one per folder), attachments inserted only on the
    // create path (so == parsed.attachments.len(), not 2x).
    assert_eq!(count_messages_for_account(&ctx.repos, account_id).await, 1);
    assert!(location_exists(&ctx.repos, message_id, inbox.id).await, "INBOX location");
    assert!(location_exists(&ctx.repos, message_id, archive.id).await, "Archive location");
    assert_eq!(
        count_attachments_for_message(&ctx.repos, message_id).await,
        expected_attachments,
        "attachments are inserted only on the create path"
    );
}
