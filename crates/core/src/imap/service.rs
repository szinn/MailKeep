use async_trait::async_trait;

use crate::{
    Error,
    account::AccountId,
    imap::model::{ImapCredentials, ImapServerConfig, RemoteFolder, SyncStatus},
};

/// Application-facing IMAP service. MK-6 wires `test_connection` and
/// `list_remote_folders`; the rest return `Error::Unimplemented` until MK-7.
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
}
