use std::sync::Arc;

use async_trait::async_trait;
use secrecy::SecretString;

use crate::{
    Error,
    account::{Account, AccountId, AccountService, AccountStatus, Credentials},
    crypto::CipherService,
    folder::{Folder, FolderService},
    imap::{
        model::{FolderConfig, ImapConnectionParams, ImapCredentials, ImapServerConfig, RemoteFolder, SyncState, SyncStatus},
        port::ImapPort,
    },
};

/// Application-facing IMAP service. `test_connection` and `list_remote_folders`
/// forward to the port; the lifecycle methods (`start_account`, `stop_account`,
/// `status`, `start_all_enabled`, `stop_all`) orchestrate account loading,
/// credential decryption, status persistence, and the port.
/// `reconcile_statuses` maps each enabled account's live `SyncState` to a
/// persisted `AccountStatus`, and stops any account that is still tracked
/// but no longer enabled (the disable-while-running teardown backstop).
///
/// Note: `start_all_enabled`/`stop_all`/`reconcile_statuses` are
/// service-orchestration methods (they iterate over accounts) and have no 1:1
/// `ImapPort` counterpart — do not add them to the port.
#[cfg_attr(any(test, feature = "test-support"), mockall::automock)]
#[async_trait]
pub trait ImapAccountService: Send + Sync {
    async fn test_connection(&self, server: ImapServerConfig, creds: ImapCredentials) -> Result<(), Error>;
    async fn list_remote_folders(&self, server: ImapServerConfig, creds: ImapCredentials) -> Result<Vec<RemoteFolder>, Error>;
    async fn start_account(&self, account_id: AccountId) -> Result<(), Error>;
    async fn stop_account(&self, account_id: AccountId) -> Result<(), Error>;
    async fn status(&self, account_id: AccountId) -> Result<SyncStatus, Error>;
    async fn start_all_enabled(&self) -> Result<(), Error>;
    async fn stop_all(&self) -> Result<(), Error>;
    async fn reconcile_statuses(&self) -> Result<(), Error>;
}

/// [`ImapAccountService`] over an [`ImapPort`] plus the core services it needs
/// to load accounts, decrypt credentials, and persist status.
pub struct ImapAccountServiceImpl {
    port: Arc<dyn ImapPort>,
    account_service: Arc<dyn AccountService>,
    folder_service: Arc<dyn FolderService>,
    cipher_service: Arc<dyn CipherService>,
}

/// Construct an [`ImapAccountService`] from the port and its dependent core
/// services.
#[must_use]
pub fn create_imap_account_service(
    port: Arc<dyn ImapPort>,
    account_service: Arc<dyn AccountService>,
    folder_service: Arc<dyn FolderService>,
    cipher_service: Arc<dyn CipherService>,
) -> Arc<dyn ImapAccountService> {
    Arc::new(ImapAccountServiceImpl {
        port,
        account_service,
        folder_service,
        cipher_service,
    })
}

impl ImapAccountServiceImpl {
    /// Load an account by id, but only if it is currently enabled. A
    /// disabled/unknown id yields `Err` — this is what makes `start_account`
    /// reject non-enabled accounts without ever touching the port.
    async fn load_enabled_account(&self, account_id: AccountId) -> Result<Account, Error> {
        self.account_service
            .list_enabled()
            .await?
            .into_iter()
            .find(|a| a.id == account_id)
            .ok_or_else(|| Error::Validation(format!("account {account_id} is not enabled or does not exist")))
    }

    fn folder_config(f: &Folder) -> FolderConfig {
        FolderConfig {
            id: f.id,
            path: f.path.clone(),
            idle_enabled: f.idle_enabled,
            uidvalidity: f.uidvalidity,
            last_uid: f.last_uid,
        }
    }

    /// Decrypt the account's credentials, build connection params, and drive
    /// the port. On success the account is marked `Syncing`; on port
    /// failure it is marked `Error` (best-effort) and the original error
    /// propagates.
    async fn start_one(&self, account: &Account) -> Result<(), Error> {
        let folders = self.folder_service.list_enabled_folders(account.id).await?;
        let plaintext = self.cipher_service.decrypt(account.id, &account.credentials)?;
        // Credentials are stored JSON-serialized (see
        // `AccountServiceImpl::encrypt_password`), so deserialize the
        // `Credentials` enum rather than treating the plaintext as a raw
        // password.
        let Credentials::Password(password) = serde_json::from_slice::<Credentials>(&plaintext).map_err(|e| Error::CredentialsDeserialize(e.to_string()))?;
        let creds = ImapCredentials {
            username: account.username.clone(),
            password: SecretString::from(password),
        };
        let params = ImapConnectionParams {
            server: account.server.clone(),
            credentials: creds,
            folders: folders.iter().map(Self::folder_config).collect(),
        };
        match self.port.start_account(account.id, params).await {
            Ok(()) => {
                self.account_service.set_status(account.id, AccountStatus::Syncing, None).await?;
                Ok(())
            }
            Err(e) => {
                // Best-effort: record the failure; do not mask the original error.
                let _ = self.account_service.set_status(account.id, AccountStatus::Error, Some(e.to_string())).await;
                Err(e)
            }
        }
    }
}

#[async_trait]
impl ImapAccountService for ImapAccountServiceImpl {
    async fn test_connection(&self, server: ImapServerConfig, creds: ImapCredentials) -> Result<(), Error> {
        self.port.test_connection(&server, &creds).await
    }

    async fn list_remote_folders(&self, server: ImapServerConfig, creds: ImapCredentials) -> Result<Vec<RemoteFolder>, Error> {
        self.port.list_folders(&server, &creds).await
    }

    async fn start_account(&self, account_id: AccountId) -> Result<(), Error> {
        let account = self.load_enabled_account(account_id).await?;
        self.start_one(&account).await
    }

    async fn stop_account(&self, account_id: AccountId) -> Result<(), Error> {
        self.port.stop_account(account_id).await
    }

    async fn status(&self, account_id: AccountId) -> Result<SyncStatus, Error> {
        self.port.status(account_id).await
    }

    async fn start_all_enabled(&self) -> Result<(), Error> {
        use futures::stream::{self, StreamExt};

        let accounts = self.account_service.list_enabled().await?;
        stream::iter(accounts)
            .for_each_concurrent(4, |account| async move {
                if let Err(e) = self.start_one(&account).await {
                    tracing::warn!(account_id = account.id, error = %e, "failed to start account sync");
                }
            })
            .await;
        Ok(())
    }

    async fn stop_all(&self) -> Result<(), Error> {
        use std::collections::HashSet;

        use futures::stream::{self, StreamExt};

        // Stop every account that is actually running. An account disabled while
        // its sync tasks are live drops out of `list_enabled()` but remains in
        // the adapter's tracked set, so stop the *union* of the two — not
        // enabled-only — to guarantee no IDLE/poll task leaks past shutdown.
        //
        // The tracked set is authoritative and reachable without the DB. stop_all
        // runs exactly once at shutdown with no retry, so a DB failure here must
        // NOT prevent stopping the tracked accounts: degrade to empty-enabled and
        // still tear down the tracked set.
        let enabled_ids = match self.account_service.list_enabled().await {
            Ok(accounts) => accounts.into_iter().map(|a| a.id).collect::<Vec<_>>(),
            Err(e) => {
                tracing::warn!(error = %e, "list_enabled failed during stop_all; stopping tracked accounts only");
                Vec::new()
            }
        };
        let ids: HashSet<AccountId> = enabled_ids.into_iter().chain(self.port.tracked_accounts().await).collect();
        stream::iter(ids)
            .for_each_concurrent(4, |id| async move {
                if let Err(e) = self.port.stop_account(id).await {
                    tracing::warn!(account_id = id, error = %e, "failed to stop account sync");
                }
            })
            .await;
        Ok(())
    }

    async fn reconcile_statuses(&self) -> Result<(), Error> {
        use std::collections::HashSet;

        let enabled = self.account_service.list_enabled().await?;
        let enabled_ids: HashSet<AccountId> = enabled.iter().map(|a| a.id).collect();

        // Enabled accounts: drive the persisted `AccountStatus` toward the live
        // `SyncState`, writing only when it actually differs.
        for account in &enabled {
            let Ok(live) = self.port.status(account.id).await else {
                continue; // untracked / not running
            };
            let desired = match live.state {
                SyncState::Error => AccountStatus::Error,
                SyncState::Idle => AccountStatus::Idle,
                SyncState::Syncing | SyncState::Connecting => AccountStatus::Syncing,
                SyncState::NotRunning => continue,
            };
            if account.status != desired {
                let _ = self.account_service.set_status(account.id, desired, live.last_error.clone()).await;
            }
        }

        // Teardown backstop: an account that is still tracked (running) but no
        // longer enabled was disabled without a successful `stop_account` (the
        // frontend stop failed, or a non-frontend path disabled it). Reclaim it
        // here so its leaked IDLE/poll tasks cannot outlive the disable.
        for id in self.port.tracked_accounts().await {
            if !enabled_ids.contains(&id) {
                if let Err(e) = self.port.stop_account(id).await {
                    tracing::warn!(account_id = id, error = %e, "failed to stop disabled-but-tracked account");
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        account::{AccountBuilder, AccountToken, MockAccountService},
        folder::{FolderBuilder, FolderId, FolderToken, MockFolderService},
        imap::{model::TlsMode, port::MockImapPort},
        types::EmailAddress,
    };

    fn server() -> ImapServerConfig {
        ImapServerConfig {
            host: "imap.example.com".into(),
            port: 993,
            tls: TlsMode::Tls,
        }
    }
    fn creds() -> ImapCredentials {
        ImapCredentials {
            username: "alice".into(),
            password: SecretString::from("pw"),
        }
    }

    /// A real test cipher; lets us encrypt a known password for a fixture
    /// account so `start_one` can decrypt it without mocking the cipher.
    fn cipher() -> Arc<dyn CipherService> {
        crate::crypto::create_cipher_service("imap-service-test-secret")
    }

    fn account(id: AccountId, cipher: &Arc<dyn CipherService>, password: &str) -> Account {
        // Encrypt exactly as `AccountServiceImpl::encrypt_password` does:
        // JSON-serialize the `Credentials` enum, then encrypt. `start_one`
        // deserializes the same way.
        let plaintext = serde_json::to_vec(&Credentials::Password(password.to_string())).unwrap();
        let ciphertext = cipher.encrypt(id, &plaintext);
        AccountBuilder::default()
            .id(id)
            .version(0)
            .token(AccountToken::new(id))
            .user_id(1)
            .display_name("Test".into())
            .email_address(EmailAddress::new("a@b.com").unwrap())
            .server(server())
            .username("alice".into())
            .credentials(ciphertext)
            .status(AccountStatus::PendingFirstSync)
            .build()
            .unwrap()
    }

    fn folder(id: FolderId, account_id: AccountId, path: &str, idle: bool) -> Folder {
        FolderBuilder::default()
            .id(id)
            .version(0)
            .token(FolderToken::new(id))
            .account_id(account_id)
            .path(path.into())
            .enabled(true)
            .idle_enabled(idle)
            .last_uid(0)
            .build()
            .unwrap()
    }

    fn sync_status(state: SyncState, last_error: Option<String>) -> SyncStatus {
        SyncStatus {
            state,
            last_sync_started_at: None,
            last_sync_finished_at: None,
            last_error,
            messages_ingested_session: 0,
        }
    }

    #[tokio::test]
    async fn test_connection_forwards_to_port() {
        let mut port = MockImapPort::new();
        port.expect_test_connection().times(1).returning(|_, _| Ok(()));
        let svc = create_imap_account_service(
            Arc::new(port),
            Arc::new(MockAccountService::new()),
            Arc::new(MockFolderService::new()),
            cipher(),
        );
        svc.test_connection(server(), creds()).await.unwrap();
    }

    #[tokio::test]
    async fn list_remote_folders_forwards_result() {
        let mut port = MockImapPort::new();
        port.expect_list_folders().times(1).returning(|_, _| {
            Ok(vec![RemoteFolder {
                path: "INBOX".into(),
                special_use: Some(crate::folder::SpecialUse::Inbox),
                has_children: false,
                no_select: false,
                delimiter: None,
            }])
        });
        let svc = create_imap_account_service(
            Arc::new(port),
            Arc::new(MockAccountService::new()),
            Arc::new(MockFolderService::new()),
            cipher(),
        );
        let folders = svc.list_remote_folders(server(), creds()).await.unwrap();
        assert_eq!(folders.len(), 1);
        assert_eq!(folders[0].path, "INBOX");
    }

    #[tokio::test]
    async fn test_connection_propagates_error() {
        let mut port = MockImapPort::new();
        port.expect_test_connection().returning(|_, _| Err(Error::Infrastructure("auth failed".into())));
        let svc = create_imap_account_service(
            Arc::new(port),
            Arc::new(MockAccountService::new()),
            Arc::new(MockFolderService::new()),
            cipher(),
        );
        let err = svc.test_connection(server(), creds()).await.unwrap_err();
        assert!(matches!(err, Error::Infrastructure(_)));
    }

    #[tokio::test]
    async fn start_account_decrypts_builds_params_and_sets_syncing() {
        let cipher = cipher();
        let acct = account(7, &cipher, "s3cret");

        let mut accounts = MockAccountService::new();
        accounts.expect_list_enabled().times(1).return_once({
            let acct = acct.clone();
            move || Ok(vec![acct])
        });
        accounts
            .expect_set_status()
            .withf(|id, status, last_error| *id == 7 && *status == AccountStatus::Syncing && last_error.is_none())
            .times(1)
            .returning(|_, _, _| Ok(()));

        let mut folders = MockFolderService::new();
        folders
            .expect_list_enabled_folders()
            .withf(|id| *id == 7)
            .times(1)
            .returning(|aid| Ok(vec![folder(1, aid, "INBOX", true), folder(2, aid, "Sent", false)]));

        let mut port = MockImapPort::new();
        port.expect_start_account()
            .withf(|id, params| {
                *id == 7
                    && params.credentials.username == "alice"
                    && secrecy::ExposeSecret::expose_secret(&params.credentials.password) == "s3cret"
                    && params.folders.len() == 2
                    && params.folders.iter().any(|f| f.path == "INBOX" && f.idle_enabled)
                    && params.folders.iter().any(|f| f.path == "Sent" && !f.idle_enabled)
            })
            .times(1)
            .returning(|_, _| Ok(()));

        let svc = create_imap_account_service(Arc::new(port), Arc::new(accounts), Arc::new(folders), cipher);
        svc.start_account(7).await.unwrap();
    }

    #[tokio::test]
    async fn start_account_unknown_or_disabled_errors_without_calling_port() {
        let mut accounts = MockAccountService::new();
        accounts.expect_list_enabled().times(1).returning(|| Ok(vec![]));
        accounts.expect_set_status().times(0);

        let mut port = MockImapPort::new();
        port.expect_start_account().times(0);

        let svc = create_imap_account_service(Arc::new(port), Arc::new(accounts), Arc::new(MockFolderService::new()), cipher());
        let err = svc.start_account(99).await.unwrap_err();
        assert!(matches!(err, Error::Validation(_)));
    }

    #[tokio::test]
    async fn start_account_port_failure_sets_error_status_and_propagates() {
        let cipher = cipher();
        let acct = account(3, &cipher, "pw");

        let mut accounts = MockAccountService::new();
        accounts.expect_list_enabled().times(1).return_once({
            let acct = acct.clone();
            move || Ok(vec![acct])
        });
        accounts
            .expect_set_status()
            .withf(|id, status, last_error| *id == 3 && *status == AccountStatus::Error && last_error.is_some())
            .times(1)
            .returning(|_, _, _| Ok(()));

        let mut folders = MockFolderService::new();
        folders.expect_list_enabled_folders().returning(|aid| Ok(vec![folder(1, aid, "INBOX", true)]));

        let mut port = MockImapPort::new();
        port.expect_start_account()
            .times(1)
            .returning(|_, _| Err(Error::Infrastructure("connect refused".into())));

        let svc = create_imap_account_service(Arc::new(port), Arc::new(accounts), Arc::new(folders), cipher);
        let err = svc.start_account(3).await.unwrap_err();
        assert!(matches!(err, Error::Infrastructure(_)));
    }

    #[tokio::test]
    async fn start_all_enabled_tolerates_one_failure() {
        let cipher = cipher();
        let a = account(1, &cipher, "pw");
        let b = account(2, &cipher, "pw");

        let mut accounts = MockAccountService::new();
        accounts.expect_list_enabled().times(1).return_once(move || Ok(vec![a, b]));
        // a -> Syncing, b -> Error; order is non-deterministic under concurrency,
        // so match on either valid (id, status) pairing.
        accounts
            .expect_set_status()
            .withf(|id, status, _| (*id == 1 && *status == AccountStatus::Syncing) || (*id == 2 && *status == AccountStatus::Error))
            .times(2)
            .returning(|_, _, _| Ok(()));

        let mut folders = MockFolderService::new();
        folders.expect_list_enabled_folders().returning(|aid| Ok(vec![folder(aid, aid, "INBOX", true)]));

        let mut port = MockImapPort::new();
        port.expect_start_account()
            .returning(|id, _| if id == 1 { Ok(()) } else { Err(Error::Infrastructure("boom".into())) });

        let svc = create_imap_account_service(Arc::new(port), Arc::new(accounts), Arc::new(folders), cipher);
        svc.start_all_enabled().await.unwrap();
    }

    #[tokio::test]
    async fn reconcile_statuses_persists_only_changes() {
        let cipher = cipher();
        // a: live Error, db Syncing -> persist Error. b: live Idle, db Idle -> no
        // write.
        let mut a = account(1, &cipher, "pw");
        a.status = AccountStatus::Syncing;
        let mut b = account(2, &cipher, "pw");
        b.status = AccountStatus::Idle;

        let mut accounts = MockAccountService::new();
        accounts.expect_list_enabled().times(1).return_once(move || Ok(vec![a, b]));
        accounts
            .expect_set_status()
            .withf(|id, status, last_error| *id == 1 && *status == AccountStatus::Error && last_error.as_deref() == Some("down"))
            .times(1)
            .returning(|_, _, _| Ok(()));

        let mut port = MockImapPort::new();
        port.expect_status().returning(|id| {
            if id == 1 {
                Ok(sync_status(SyncState::Error, Some("down".into())))
            } else {
                Ok(sync_status(SyncState::Idle, None))
            }
        });
        port.expect_tracked_accounts().returning(|| vec![1, 2]);

        let svc = create_imap_account_service(Arc::new(port), Arc::new(accounts), Arc::new(MockFolderService::new()), cipher);
        svc.reconcile_statuses().await.unwrap();
    }

    #[tokio::test]
    async fn stop_all_stops_union_of_enabled_and_tracked() {
        let cipher = cipher();
        let a = account(1, &cipher, "pw");

        let mut accounts = MockAccountService::new();
        accounts.expect_list_enabled().times(1).return_once(move || Ok(vec![a]));

        let mut port = MockImapPort::new();
        // Account 1 is enabled; account 2 was disabled while still tracked.
        // stop_all must cover the union {1, 2}.
        port.expect_tracked_accounts().times(1).returning(|| vec![1, 2]);
        port.expect_stop_account().withf(|id| *id == 1 || *id == 2).times(2).returning(|_| Ok(()));

        let svc = create_imap_account_service(Arc::new(port), Arc::new(accounts), Arc::new(MockFolderService::new()), cipher);
        svc.stop_all().await.unwrap();
    }

    #[tokio::test]
    async fn stop_all_stops_tracked_when_list_enabled_fails() {
        let cipher = cipher();

        let mut accounts = MockAccountService::new();
        accounts
            .expect_list_enabled()
            .times(1)
            .returning(|| Err(Error::Infrastructure("db down".into())));

        let mut port = MockImapPort::new();
        // DB is unreachable, but the tracked set is still authoritative.
        port.expect_tracked_accounts().times(1).returning(|| vec![5]);
        port.expect_stop_account().withf(|id| *id == 5).times(1).returning(|_| Ok(()));

        let svc = create_imap_account_service(Arc::new(port), Arc::new(accounts), Arc::new(MockFolderService::new()), cipher);
        svc.stop_all().await.unwrap();
    }

    #[tokio::test]
    async fn reconcile_tears_down_tracked_but_disabled_account() {
        let cipher = cipher();
        // Enabled account 1: live Idle == db Idle, so no status write.
        let mut a = account(1, &cipher, "pw");
        a.status = AccountStatus::Idle;

        let mut accounts = MockAccountService::new();
        accounts.expect_list_enabled().times(1).return_once(move || Ok(vec![a]));
        accounts.expect_set_status().never();

        let mut port = MockImapPort::new();
        port.expect_status().returning(|_| Ok(sync_status(SyncState::Idle, None)));
        // Account 2 is still tracked but no longer enabled — the leak case.
        port.expect_tracked_accounts().times(1).returning(|| vec![1, 2]);
        port.expect_stop_account().withf(|id| *id == 2).times(1).returning(|_| Ok(()));

        let svc = create_imap_account_service(Arc::new(port), Arc::new(accounts), Arc::new(MockFolderService::new()), cipher);
        svc.reconcile_statuses().await.unwrap();
    }
}
