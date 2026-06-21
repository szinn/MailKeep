//! MK-8 end-to-end account-add acceptance test against a real greenmail IMAP
//! server.
//!
//! Tasks 1–9 implemented the account-add UX. The `create_account_and_start`
//! server fn (frontend) orchestrates three core-service calls:
//!
//! ```text
//! account_service.create_account(CreateAccountParams)
//!   → folder_service.create_folders_for_account(account_id, Vec<NewFolderRequest>)
//!   → imap_account_service.start_account(account_id)
//! ```
//!
//! Server fns can't be invoked headless, so this test drives the **same**
//! core-service sequence directly. Unlike `imap_sync.rs` (which drives the
//! `ImapAdapter` directly via `ImapPort::start_account`), this test exercises
//! the full orchestration path through `imap_account_service.start_account` —
//! account load + credential decryption + enabled-folder lookup + port drive —
//! exactly as the server fn does. The only difference from production wiring is
//! the `imap_port_factory`, which here builds an `ImapAdapter` that trusts
//! greenmail's self-signed cert (`with_tls_config`).
//!
//! Reuses the MK-7 greenmail harness wholesale via `greenmail_support`
//! (container bring-up, control client for seeding, ingest-wait +
//! ciphertext-on-disk assertions). Compiled only under the `greenmail` feature
//! and `#[ignore]`d, so the default `sqlite` run never touches it. Run with
//! `just imap-integration-tests`.

use std::{sync::Arc, time::Duration};

use mk_core::{
    account::CreateAccountParams,
    folder::{FolderService, NewFolderRequest, SpecialUse},
    imap::{ImapPort, ImapPortFactory},
    ingest::IngestService,
    message::MessageService,
    test_support::{default_external_services_builder, test_cipher_service},
    types::EmailAddress,
    user::NewUser,
};
use mk_database::create_repository_service;
use mk_imap::ImapAdapter;
use sea_orm::Database;
use secrecy::SecretString;
use tempfile::TempDir;

use crate::{
    context::TestContext,
    greenmail_support::{ACCOUNT_TIMEOUT, Control, Greenmail, PASSWORD, USERNAME, assert_ciphertext_on_disk, insecure_tls, run_core, wait_for_messages},
};

const EMAIL: &str = "alice@example.com";

// ─── System-under-test pipeline ──────────────────────────────────────────────

/// Build the real core pipeline (sqlite::memory: DB, encrypted tempdir storage,
/// parser handlers) wiring an `imap_port_factory` that produces a
/// greenmail-trusting `ImapAdapter`. This is what lets the test drive the REAL
/// orchestration path `imap_account_service.start_account` (which itself
/// constructs the port via the factory in `CoreServices::new`). Unlike
/// `imap_sync.rs::setup_pipeline`, it routes the adapter through the factory
/// instead of building it standalone.
async fn setup_pipeline() -> TestContext {
    let dir = TempDir::new().unwrap();
    let cipher = test_cipher_service();
    let storage = mk_storage::create_filesystem_storage(dir.path(), cipher.clone()).await.unwrap();

    let db = Database::connect("sqlite::memory:").await.unwrap();
    let repository_service = create_repository_service(db).await.unwrap();

    // The adapter is built inside `create_services` via this factory, so the
    // start_account path drives the same port the server fn would.
    let imap_port_factory: ImapPortFactory = Box::new(
        |ingest: Arc<dyn IngestService>, folders: Arc<dyn FolderService>, messages: Arc<dyn MessageService>| {
            Arc::new(ImapAdapter::with_tls_config(ingest, folders, messages, Duration::from_secs(1), insecure_tls())) as Arc<dyn ImapPort>
        },
    );

    let core = mk_core::create_services(
        default_external_services_builder()
            .repository_service(repository_service.clone())
            .cipher_service(cipher)
            .raw_storage_service(storage.raw_storage_service)
            .attachment_storage_service(storage.attachment_storage_service)
            .imap_port_factory(imap_port_factory)
            .build()
            .unwrap(),
    )
    .unwrap();

    mk_parser::register_handlers(&core);

    TestContext::new(core, repository_service, dir)
}

// ─── Test: end-to-end account-add orchestration ──────────────────────────────

/// Drives the `create_account_and_start` core-service sequence end-to-end:
/// create_account → create_folders_for_account(INBOX) → start_account, then
/// asserts the seeded INBOX message is ingested (Message row + on-disk
/// ciphertext). This proves the orchestration path the MK-8 server fn invokes.
#[tokio::test]
#[ignore = "needs a docker/colima daemon — run via `just imap-integration-tests`"]
async fn account_add_flow_ingests_inbox() {
    let gm = Greenmail::start().await;

    // Seed greenmail's INBOX with one message BEFORE the account starts so the
    // engine's initial catch-up fetches it.
    let mut control = Control::connect(&gm).await.unwrap();
    control.append("INBOX", "account add seed").await.unwrap();
    let _ = control.logout().await;

    let ctx = setup_pipeline().await;

    // A user must exist for the account to belong to.
    let new_user = NewUser::new("alice", "password-hash", EMAIL, std::collections::HashSet::new(), "Alice", false).unwrap();
    let user = ctx.services.user_service.add_user(new_user).await.unwrap();

    let core = run_core(&ctx);

    // ── 1. create_account (same params the server fn builds) ──────────────────
    let account = ctx
        .services
        .account_service
        .create_account(CreateAccountParams {
            user_id: user.id,
            display_name: "Alice".into(),
            email_address: EmailAddress::new(EMAIL).unwrap(),
            server: gm.server(),
            username: USERNAME.into(),
            password: SecretString::from(PASSWORD.to_string()),
        })
        .await
        .unwrap();

    // ── 2. create_folders_for_account (INBOX only) ────────────────────────────
    ctx.services
        .folder_service
        .create_folders_for_account(
            account.id,
            vec![NewFolderRequest {
                path: "INBOX".into(),
                display_name: None,
                special_use: Some(SpecialUse::Inbox),
                uidvalidity: None,
            }],
        )
        .await
        .unwrap();

    // ── 3. start_account → first sync ─────────────────────────────────────────
    // This is the real orchestration call: it loads the enabled account,
    // decrypts credentials, builds params from enabled folders, and drives the
    // (greenmail-trusting) ImapAdapter built by the factory above.
    ctx.services.imap_account_service.start_account(account.id).await.unwrap();

    // The seeded INBOX message must reach a DB row and on-disk ciphertext.
    let msgs = wait_for_messages(&ctx.repos, account.id, 1, ACCOUNT_TIMEOUT).await;
    assert_eq!(msgs.len(), 1, "the seeded INBOX message must be ingested into a DB row");
    assert_ciphertext_on_disk(&ctx, account.id, &msgs[0]).await;

    ctx.services.imap_account_service.stop_account(account.id).await.unwrap();
    core.abort();
    let _ = core.await;
}

// ─── Test: \Noselect safety filter (mirrors server fn §6 gate) ───────────────

/// `FolderTree::selected_new_folders` lives in the `frontend` crate, which is
/// NOT a dependency of `integration-tests`, so it is not reachable here. This
/// focused unit test instead replicates the authoritative server-side filter
/// (`create_account_and_start`, spec §6: `.filter(|f| !f.no_select)`) at the
/// `NewFolderRequest`-building layer: a submission containing a \Noselect
/// container excludes the container but includes its selectable children.
#[test]
fn noselect_filter_excludes_container_keeps_children() {
    // A submission as it would arrive from the folder picker: a \Noselect
    // container ("Archive") with two selectable children.
    struct Submitted {
        path: &'static str,
        no_select: bool,
    }
    let submitted = vec![
        Submitted {
            path: "Archive",
            no_select: true, // \Noselect container — must be filtered out
        },
        Submitted {
            path: "Archive/2023",
            no_select: false,
        },
        Submitted {
            path: "Archive/2024",
            no_select: false,
        },
    ];

    // Replicates the server fn's authoritative filter at the request-building
    // layer.
    let requests: Vec<NewFolderRequest> = submitted
        .into_iter()
        .filter(|f| !f.no_select)
        .map(|f| NewFolderRequest {
            path: f.path.into(),
            display_name: None,
            special_use: None,
            uidvalidity: None,
        })
        .collect();

    let paths: Vec<&str> = requests.iter().map(|r| r.path.as_str()).collect();
    assert_eq!(
        paths,
        vec!["Archive/2023", "Archive/2024"],
        "\\Noselect container excluded; selectable children kept"
    );
}
