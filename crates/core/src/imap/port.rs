use async_trait::async_trait;

use crate::{
    Error,
    account::AccountId,
    imap::model::{ImapConnectionParams, ImapCredentials, ImapServerConfig, RemoteFolder, SyncStatus},
};

/// Driven port for the IMAP adapter. MK-6 implements `test_connection` and
/// `list_folders`; the lifecycle methods are stubbed (typed error) until MK-7.
#[cfg_attr(any(test, feature = "test-support"), mockall::automock)]
#[async_trait]
pub trait ImapPort: Send + Sync {
    async fn test_connection(&self, server: &ImapServerConfig, creds: &ImapCredentials) -> Result<(), Error>;
    async fn list_folders(&self, server: &ImapServerConfig, creds: &ImapCredentials) -> Result<Vec<RemoteFolder>, Error>;
    async fn start_account(&self, account_id: AccountId, params: ImapConnectionParams) -> Result<(), Error>;
    async fn stop_account(&self, account_id: AccountId) -> Result<(), Error>;
    async fn status(&self, account_id: AccountId) -> Result<SyncStatus, Error>;
}
