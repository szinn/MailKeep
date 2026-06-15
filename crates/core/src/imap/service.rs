use std::sync::Arc;

use async_trait::async_trait;
use secrecy::SecretString;

use crate::{
    Error,
    account::{Account, AccountId, AccountService, AccountStatus},
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
/// `reconcile_statuses` maps each tracked account's live `SyncState` to a
/// persisted `AccountStatus`.
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
        let password = String::from_utf8(plaintext).map_err(|_| Error::Validation("decrypted credential is not valid UTF-8".into()))?;
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
                self.account_service
                    .set_status(account.id, AccountStatus::Error, Some(e.to_string()))
                    .await
                    .ok();
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
        use futures::stream::{self, StreamExt};

        let accounts = self.account_service.list_enabled().await?;
        stream::iter(accounts)
            .for_each_concurrent(4, |account| async move {
                if let Err(e) = self.port.stop_account(account.id).await {
                    tracing::warn!(account_id = account.id, error = %e, "failed to stop account sync");
                }
            })
            .await;
        Ok(())
    }

    async fn reconcile_statuses(&self) -> Result<(), Error> {
        let accounts = self.account_service.list_enabled().await?;
        for account in accounts {
            let live = match self.port.status(account.id).await {
                Ok(s) => s,
                Err(_) => continue, // untracked / not running
            };
            let desired = match live.state {
                SyncState::Error => AccountStatus::Error,
                SyncState::Idle => AccountStatus::Idle,
                SyncState::Syncing | SyncState::Connecting => AccountStatus::Syncing,
                SyncState::NotRunning => continue,
            };
            if account.status != desired {
                self.account_service.set_status(account.id, desired, live.last_error.clone()).await.ok();
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use secrecy::SecretString;

    use super::*;
    use crate::{
        account::{AccountBuilder, AccountToken, MockAccountService},
        crypto::CipherService,
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
        let ciphertext = cipher.encrypt(id, password.as_bytes());
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

        let svc = create_imap_account_service(Arc::new(port), Arc::new(accounts), Arc::new(MockFolderService::new()), cipher);
        svc.reconcile_statuses().await.unwrap();
    }
}
