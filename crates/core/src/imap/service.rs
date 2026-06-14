use std::sync::Arc;

use async_trait::async_trait;

use crate::{
    Error,
    account::AccountId,
    imap::{
        model::{ImapCredentials, ImapServerConfig, RemoteFolder, SyncStatus},
        port::ImapPort,
    },
};

/// Application-facing IMAP service. MK-6 wires `test_connection` and
/// `list_remote_folders`; the rest return `Error::Unimplemented` until MK-7.
///
/// Note: `start_all_enabled`/`stop_all` are service-orchestration methods (they
/// iterate over accounts in MK-7) and have no 1:1 `ImapPort` counterpart — do
/// not add them to the port.
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

/// Forwarding [`ImapAccountService`] over an [`ImapPort`]; lifecycle methods are
/// stubbed with [`Error::Unimplemented`] until MK-7.
pub struct ImapAccountServiceImpl {
    port: Arc<dyn ImapPort>,
}

impl ImapAccountServiceImpl {
    #[must_use]
    pub fn new(port: Arc<dyn ImapPort>) -> Self {
        Self { port }
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

    async fn start_account(&self, _account_id: AccountId) -> Result<(), Error> {
        Err(Error::Unimplemented("ImapAccountService::start_account (MK-7)"))
    }

    async fn stop_account(&self, _account_id: AccountId) -> Result<(), Error> {
        Err(Error::Unimplemented("ImapAccountService::stop_account (MK-7)"))
    }

    async fn status(&self, _account_id: AccountId) -> Result<SyncStatus, Error> {
        Err(Error::Unimplemented("ImapAccountService::status (MK-7)"))
    }

    async fn start_all_enabled(&self) -> Result<(), Error> {
        Err(Error::Unimplemented("ImapAccountService::start_all_enabled (MK-7)"))
    }

    async fn stop_all(&self) -> Result<(), Error> {
        Err(Error::Unimplemented("ImapAccountService::stop_all (MK-7)"))
    }
}

#[cfg(test)]
mod tests {
    use secrecy::SecretString;

    use super::*;
    use crate::imap::{model::TlsMode, port::MockImapPort};

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

    #[tokio::test]
    async fn test_connection_forwards_to_port() {
        let mut port = MockImapPort::new();
        port.expect_test_connection().times(1).returning(|_, _| Ok(()));
        let svc = ImapAccountServiceImpl::new(Arc::new(port));
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
        let svc = ImapAccountServiceImpl::new(Arc::new(port));
        let folders = svc.list_remote_folders(server(), creds()).await.unwrap();
        assert_eq!(folders.len(), 1);
        assert_eq!(folders[0].path, "INBOX");
    }

    #[tokio::test]
    async fn test_connection_propagates_error() {
        let mut port = MockImapPort::new();
        port.expect_test_connection().returning(|_, _| Err(Error::Infrastructure("auth failed".into())));
        let svc = ImapAccountServiceImpl::new(Arc::new(port));
        let err = svc.test_connection(server(), creds()).await.unwrap_err();
        assert!(matches!(err, Error::Infrastructure(_)));
    }

    #[tokio::test]
    async fn lifecycle_methods_are_unimplemented() {
        let svc = ImapAccountServiceImpl::new(Arc::new(MockImapPort::new()));
        assert!(matches!(svc.start_account(1).await, Err(Error::Unimplemented(_))));
        assert!(matches!(svc.stop_account(1).await, Err(Error::Unimplemented(_))));
        assert!(matches!(svc.status(1).await, Err(Error::Unimplemented(_))));
        assert!(matches!(svc.start_all_enabled().await, Err(Error::Unimplemented(_))));
        assert!(matches!(svc.stop_all().await, Err(Error::Unimplemented(_))));
    }
}
