use std::sync::Arc;

use secrecy::{ExposeSecret, SecretString};

use crate::{
    Error, RepositoryError,
    account::{Account, AccountId, AccountStatus, AccountToken, Credentials, NewAccount, PartialAccountUpdate},
    crypto::{CipherService, Ciphertext},
    imap::ImapServerConfig,
    repository::RepositoryService,
    storage::{AttachmentStorageService, RawStorageService},
    types::EmailAddress,
    user::UserId,
    with_read_only_transaction, with_transaction,
};

#[async_trait::async_trait]
pub trait AccountService: Send + Sync {
    async fn create_account(&self, params: CreateAccountParams) -> Result<Account, Error>;
    async fn get_account(&self, user_id: UserId, account_id: AccountId) -> Result<Account, Error>;
    async fn list_accounts(&self, user_id: UserId) -> Result<Vec<Account>, Error>;
    async fn list_enabled(&self) -> Result<Vec<Account>, Error>;

    async fn update_account(&self, user_id: UserId, account_id: AccountId, expected_version: u64, input: PartialAccountInput) -> Result<Account, Error>;

    async fn enable(&self, user_id: UserId, account_id: AccountId) -> Result<(), Error>;
    async fn disable(&self, user_id: UserId, account_id: AccountId) -> Result<(), Error>;

    async fn set_status(&self, account_id: AccountId, status: AccountStatus, last_error: Option<String>) -> Result<(), Error>;

    async fn delete_account(&self, user_id: UserId, account_id: AccountId) -> Result<(), Error>;

    async fn decrypt_credentials(&self, user_id: UserId, account_id: AccountId) -> Result<Credentials, Error>;
}

pub struct CreateAccountParams {
    pub user_id: UserId,
    pub display_name: String,
    pub email_address: EmailAddress,
    pub server: ImapServerConfig,
    pub username: String,
    pub password: SecretString,
}

/// Form-facing partial update — holds plaintext password. Service re-encrypts
/// before constructing `PartialAccountUpdate` for the repository.
#[derive(Default)]
pub struct PartialAccountInput {
    pub display_name: Option<String>,
    pub password: Option<SecretString>,
    pub server: Option<ImapServerConfig>,
    pub username: Option<String>,
}

pub(crate) struct AccountServiceImpl {
    repository_service: Arc<RepositoryService>,
    cipher_service: Arc<dyn CipherService>,
    raw_storage_service: Arc<dyn RawStorageService>,
    attachment_storage_service: Arc<dyn AttachmentStorageService>,
}

impl AccountServiceImpl {
    pub(crate) fn new(
        repository_service: Arc<RepositoryService>,
        cipher_service: Arc<dyn CipherService>,
        raw_storage_service: Arc<dyn RawStorageService>,
        attachment_storage_service: Arc<dyn AttachmentStorageService>,
    ) -> Self {
        Self {
            repository_service,
            cipher_service,
            raw_storage_service,
            attachment_storage_service,
        }
    }

    fn validate_create(p: &CreateAccountParams) -> Result<(), Error> {
        if p.display_name.trim().is_empty() {
            return Err(Error::Validation("display_name is required".into()));
        }
        if p.server.host.trim().is_empty() {
            return Err(Error::Validation("server.host is required".into()));
        }
        if p.server.port == 0 {
            return Err(Error::Validation("server.port must be > 0".into()));
        }
        if p.username.trim().is_empty() {
            return Err(Error::Validation("username is required".into()));
        }
        if p.password.expose_secret().is_empty() {
            return Err(Error::Validation("password is required".into()));
        }
        Ok(())
    }

    fn validate_update(input: &PartialAccountInput) -> Result<(), Error> {
        if let Some(d) = &input.display_name
            && d.trim().is_empty()
        {
            return Err(Error::Validation("display_name must be non-empty".into()));
        }
        if let Some(s) = &input.server {
            if s.host.trim().is_empty() {
                return Err(Error::Validation("server.host must be non-empty".into()));
            }
            if s.port == 0 {
                return Err(Error::Validation("server.port must be > 0".into()));
            }
        }
        if let Some(u) = &input.username
            && u.trim().is_empty()
        {
            return Err(Error::Validation("username must be non-empty".into()));
        }
        if let Some(p) = &input.password
            && p.expose_secret().is_empty()
        {
            return Err(Error::Validation("password must be non-empty".into()));
        }
        Ok(())
    }

    fn encrypt_password(&self, account_id: AccountId, password: &SecretString) -> Result<Ciphertext, Error> {
        let creds = Credentials::Password(password.expose_secret().to_owned());
        let plaintext = serde_json::to_vec(&creds).map_err(|e| Error::CryptoError(format!("serialize credentials: {e}")))?;
        Ok(self.cipher_service.encrypt(account_id, &plaintext))
    }
}

#[async_trait::async_trait]
impl AccountService for AccountServiceImpl {
    async fn create_account(&self, params: CreateAccountParams) -> Result<Account, Error> {
        Self::validate_create(&params)?;
        let token = AccountToken::generate();
        let account_id = token.id();
        let credentials = self.encrypt_password(account_id, &params.password)?;

        let new_account = NewAccount {
            user_id: params.user_id,
            display_name: params.display_name,
            email_address: params.email_address,
            server: params.server,
            username: params.username,
            credentials,
            token,
        };
        with_transaction!(self, account_repository, |tx| account_repository.insert(tx, new_account).await)
    }

    async fn get_account(&self, user_id: UserId, account_id: AccountId) -> Result<Account, Error> {
        with_read_only_transaction!(self, account_repository, |tx| {
            account_repository
                .find_by_id_for_user(tx, user_id, account_id)
                .await?
                .ok_or(Error::RepositoryError(RepositoryError::NotFound))
        })
    }

    async fn list_accounts(&self, user_id: UserId) -> Result<Vec<Account>, Error> {
        with_read_only_transaction!(self, account_repository, |tx| account_repository.list_for_user(tx, user_id).await)
    }

    async fn list_enabled(&self) -> Result<Vec<Account>, Error> {
        with_read_only_transaction!(self, account_repository, |tx| account_repository.list_enabled(tx).await)
    }

    async fn update_account(&self, user_id: UserId, account_id: AccountId, expected_version: u64, input: PartialAccountInput) -> Result<Account, Error> {
        Self::validate_update(&input)?;
        let credentials = match &input.password {
            Some(pw) => Some(self.encrypt_password(account_id, pw)?),
            None => None,
        };
        let partial = PartialAccountUpdate {
            display_name: input.display_name,
            credentials,
            server: input.server,
            username: input.username,
        };
        with_transaction!(self, account_repository, |tx| {
            let mut existing = account_repository
                .find_by_id_for_user(tx, user_id, account_id)
                .await?
                .ok_or(Error::RepositoryError(RepositoryError::NotFound))?;
            if existing.version != expected_version {
                return Err(Error::RepositoryError(RepositoryError::Conflict));
            }
            partial.apply_to(&mut existing);
            account_repository.update(tx, existing).await
        })
    }

    async fn enable(&self, user_id: UserId, account_id: AccountId) -> Result<(), Error> {
        with_transaction!(self, account_repository, |tx| {
            account_repository
                .find_by_id_for_user(tx, user_id, account_id)
                .await?
                .ok_or(Error::RepositoryError(RepositoryError::NotFound))?;
            account_repository.set_enabled(tx, account_id, true).await?;
            account_repository.set_status(tx, account_id, AccountStatus::PendingFirstSync, None).await
        })
    }

    async fn disable(&self, user_id: UserId, account_id: AccountId) -> Result<(), Error> {
        with_transaction!(self, account_repository, |tx| {
            account_repository
                .find_by_id_for_user(tx, user_id, account_id)
                .await?
                .ok_or(Error::RepositoryError(RepositoryError::NotFound))?;
            account_repository.set_enabled(tx, account_id, false).await?;
            account_repository.set_status(tx, account_id, AccountStatus::Disabled, None).await
        })
    }

    async fn set_status(&self, account_id: AccountId, status: AccountStatus, last_error: Option<String>) -> Result<(), Error> {
        with_transaction!(self, account_repository, |tx| account_repository
            .set_status(tx, account_id, status, last_error)
            .await)
    }

    async fn delete_account(&self, user_id: UserId, account_id: AccountId) -> Result<(), Error> {
        let account_to_delete = with_transaction!(self, account_repository, |tx| {
            let existing = account_repository
                .find_by_id_for_user(tx, user_id, account_id)
                .await?
                .ok_or(Error::RepositoryError(RepositoryError::NotFound))?;
            account_repository.delete(tx, existing).await
        })?;

        // Best-effort storage cleanup post-commit. Each storage service has its
        // own per-account directory tree. Failure is logged but does NOT fail
        // the operation — cryptographic erasure (AAD now invalid) is the real
        // guarantee.
        if let Err(e) = self.raw_storage_service.delete_account(account_to_delete.id).await {
            tracing::warn!(
                account_id = account_to_delete.id,
                error = %e,
                "failed to delete raw storage for account; row is gone, ciphertext is cryptographically inaccessible",
            );
        }
        if let Err(e) = self.attachment_storage_service.delete_account(account_to_delete.id).await {
            tracing::warn!(
                account_id = account_to_delete.id,
                error = %e,
                "failed to delete attachment storage for account; row is gone, ciphertext is cryptographically inaccessible",
            );
        }
        Ok(())
    }

    async fn decrypt_credentials(&self, user_id: UserId, account_id: AccountId) -> Result<Credentials, Error> {
        let account = self.get_account(user_id, account_id).await?;
        let plaintext = self.cipher_service.decrypt(account_id, &account.credentials)?;
        serde_json::from_slice::<Credentials>(&plaintext).map_err(|e| Error::CredentialsDeserialize(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use mockall::predicate::*;

    use super::*;
    use crate::{
        account::{AccountBuilder, repository::MockAccountRepository},
        crypto::create_cipher_service,
        imap::TlsMode,
        repository::testing::default_repository_service_builder,
        storage::{MockAttachmentStorageService, MockRawStorageService},
    };

    fn cipher() -> Arc<dyn CipherService> {
        create_cipher_service("test-secret-MK-3")
    }

    fn make_params(user_id: u64, password: &str) -> CreateAccountParams {
        CreateAccountParams {
            user_id,
            display_name: "Test".into(),
            email_address: EmailAddress::new("a@b.com").unwrap(),
            server: ImapServerConfig {
                host: "imap.example.com".into(),
                port: 993,
                tls: TlsMode::Tls,
            },
            username: "a@b.com".into(),
            password: SecretString::from(password.to_string()),
        }
    }

    /// Build a service with a configurable account repo + non-expecting storage
    /// mocks. Tests that exercise delete_account override the storage mocks
    /// via the alt builder below.
    fn make_service(account_repo: MockAccountRepository) -> AccountServiceImpl {
        make_service_with_storage(account_repo, MockRawStorageService::new(), MockAttachmentStorageService::new())
    }

    fn make_service_with_storage(account_repo: MockAccountRepository, raw: MockRawStorageService, attach: MockAttachmentStorageService) -> AccountServiceImpl {
        let repository_service = Arc::new(
            default_repository_service_builder()
                .account_repository(Arc::new(account_repo))
                .build()
                .expect("all fields provided"),
        );
        AccountServiceImpl::new(repository_service, cipher(), Arc::new(raw), Arc::new(attach))
    }

    fn fake_existing_account(id: u64, user_id: u64) -> Account {
        AccountBuilder::default()
            .id(id)
            .version(0)
            .token(AccountToken::new(id))
            .user_id(user_id)
            .display_name("Existing".into())
            .email_address(EmailAddress::new("a@b.com").unwrap())
            .server(ImapServerConfig {
                host: "imap.example.com".into(),
                port: 993,
                tls: TlsMode::Tls,
            })
            .username("a@b.com".into())
            .credentials(Ciphertext::from_raw(vec![0u8; 28]))
            .build()
            .unwrap()
    }

    #[tokio::test]
    async fn create_account_encrypts_credentials() {
        let mut repo = MockAccountRepository::new();
        repo.expect_insert().returning(|_, na| {
            assert_ne!(na.credentials.as_bytes(), b"hunter2");
            assert!(!na.credentials.as_bytes().is_empty());
            let mut a = fake_existing_account(na.token.id(), na.user_id);
            a.credentials = na.credentials;
            Box::pin(async move { Ok(a) })
        });
        let svc = make_service(repo);
        svc.create_account(make_params(42, "hunter2")).await.unwrap();
    }

    #[tokio::test]
    async fn create_account_aad_binding_distinct_ciphertexts() {
        let captured = Arc::new(Mutex::new(Vec::<Vec<u8>>::new()));
        let cap = captured.clone();
        let mut repo = MockAccountRepository::new();
        repo.expect_insert().returning(move |_, na| {
            cap.lock().unwrap().push(na.credentials.as_bytes().to_vec());
            let mut a = fake_existing_account(na.token.id(), na.user_id);
            a.credentials = na.credentials;
            Box::pin(async move { Ok(a) })
        });
        let svc = make_service(repo);

        svc.create_account(make_params(42, "samepass")).await.unwrap();
        svc.create_account(make_params(42, "samepass")).await.unwrap();

        let cts = captured.lock().unwrap();
        assert_eq!(cts.len(), 2);
        assert_ne!(cts[0], cts[1]);
    }

    #[tokio::test]
    async fn create_account_validation_empty_display_name() {
        let svc = make_service(MockAccountRepository::new());
        let mut p = make_params(42, "pw");
        p.display_name = "  ".into();
        let err = svc.create_account(p).await;
        assert!(matches!(err, Err(Error::Validation(_))));
    }

    #[tokio::test]
    async fn create_account_validation_empty_password() {
        let svc = make_service(MockAccountRepository::new());
        let p = make_params(42, "");
        let err = svc.create_account(p).await;
        assert!(matches!(err, Err(Error::Validation(_))));
    }

    #[tokio::test]
    async fn create_account_validation_port_zero() {
        let svc = make_service(MockAccountRepository::new());
        let mut p = make_params(42, "pw");
        p.server.port = 0;
        let err = svc.create_account(p).await;
        assert!(matches!(err, Err(Error::Validation(_))));
    }

    #[tokio::test]
    async fn decrypt_credentials_round_trip() {
        let inserted: Arc<Mutex<Option<Account>>> = Arc::new(Mutex::new(None));
        let ins = inserted.clone();
        let mut repo = MockAccountRepository::new();
        repo.expect_insert().returning(move |_, na| {
            let mut a = fake_existing_account(na.token.id(), na.user_id);
            a.credentials = na.credentials;
            *ins.lock().unwrap() = Some(a.clone());
            Box::pin(async move { Ok(a) })
        });
        let find_ins = inserted.clone();
        repo.expect_find_by_id_for_user().returning(move |_, _u, _a| {
            let a = find_ins.lock().unwrap().clone();
            Box::pin(async move { Ok(a) })
        });

        let svc = make_service(repo);
        let created = svc.create_account(make_params(42, "hunter2")).await.unwrap();
        let creds = svc.decrypt_credentials(42, created.id).await.unwrap();
        match creds {
            Credentials::Password(p) => assert_eq!(p, "hunter2"),
        }
    }

    #[tokio::test]
    async fn decrypt_credentials_tenant_mismatch_returns_not_found() {
        let mut repo = MockAccountRepository::new();
        repo.expect_find_by_id_for_user().returning(|_, _, _| Box::pin(async { Ok(None) }));
        let svc = make_service(repo);
        let err = svc.decrypt_credentials(99, 1).await;
        assert!(matches!(err, Err(Error::RepositoryError(RepositoryError::NotFound))));
    }

    #[tokio::test]
    async fn update_account_password_re_encrypts() {
        let original_ct: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));
        let orig = original_ct.clone();
        let mut repo = MockAccountRepository::new();
        repo.expect_insert().returning(move |_, na| {
            *orig.lock().unwrap() = na.credentials.as_bytes().to_vec();
            let mut a = fake_existing_account(na.token.id(), na.user_id);
            a.credentials = na.credentials;
            Box::pin(async move { Ok(a) })
        });
        let orig_find = original_ct.clone();
        repo.expect_find_by_id_for_user().returning(move |_, user_id, account_id| {
            let mut a = fake_existing_account(account_id, user_id);
            a.credentials = Ciphertext::from_raw(orig_find.lock().unwrap().clone());
            Box::pin(async move { Ok(Some(a)) })
        });
        let captured: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));
        let cap = captured.clone();
        repo.expect_update().returning(move |_, acct| {
            *cap.lock().unwrap() = acct.credentials.as_bytes().to_vec();
            Box::pin(async move { Ok(acct) })
        });

        let svc = make_service(repo);
        let created = svc.create_account(make_params(42, "old-pw")).await.unwrap();
        svc.update_account(
            42,
            created.id,
            0,
            PartialAccountInput {
                password: Some(SecretString::from("new-pw".to_string())),
                ..Default::default()
            },
        )
        .await
        .unwrap();

        let new_ct = captured.lock().unwrap().clone();
        let old_ct = original_ct.lock().unwrap().clone();
        assert_ne!(new_ct, old_ct);
    }

    #[tokio::test]
    async fn update_account_tenant_mismatch_returns_not_found() {
        let mut repo = MockAccountRepository::new();
        repo.expect_find_by_id_for_user().returning(|_, _, _| Box::pin(async { Ok(None) }));
        let svc = make_service(repo);
        let err = svc.update_account(99, 1, 0, PartialAccountInput::default()).await;
        assert!(matches!(err, Err(Error::RepositoryError(RepositoryError::NotFound))));
    }

    #[tokio::test]
    async fn update_account_stale_version_returns_conflict() {
        let mut repo = MockAccountRepository::new();
        repo.expect_find_by_id_for_user()
            .returning(|_, user_id, account_id| Box::pin(async move { Ok(Some(fake_existing_account(account_id, user_id))) }));
        let svc = make_service(repo);
        let err = svc
            .update_account(
                42,
                1,
                99,
                PartialAccountInput {
                    display_name: Some("X".into()),
                    ..Default::default()
                },
            )
            .await;
        assert!(matches!(err, Err(Error::RepositoryError(RepositoryError::Conflict))));
    }

    #[tokio::test]
    async fn disable_sets_status_disabled_and_enabled_false() {
        let mut repo = MockAccountRepository::new();
        repo.expect_find_by_id_for_user()
            .returning(|_, user_id, account_id| Box::pin(async move { Ok(Some(fake_existing_account(account_id, user_id))) }));
        repo.expect_set_enabled()
            .withf(|_, _id, enabled| !*enabled)
            .returning(|_, _, _| Box::pin(async { Ok(()) }));
        repo.expect_set_status()
            .withf(|_, _id, status, err| *status == AccountStatus::Disabled && err.is_none())
            .returning(|_, _, _, _| Box::pin(async { Ok(()) }));
        let svc = make_service(repo);
        svc.disable(42, 1).await.unwrap();
    }

    #[tokio::test]
    async fn enable_sets_status_pending_and_enabled_true() {
        let mut repo = MockAccountRepository::new();
        repo.expect_find_by_id_for_user()
            .returning(|_, user_id, account_id| Box::pin(async move { Ok(Some(fake_existing_account(account_id, user_id))) }));
        repo.expect_set_enabled()
            .withf(|_, _id, enabled| *enabled)
            .returning(|_, _, _| Box::pin(async { Ok(()) }));
        repo.expect_set_status()
            .withf(|_, _id, status, err| *status == AccountStatus::PendingFirstSync && err.is_none())
            .returning(|_, _, _, _| Box::pin(async { Ok(()) }));
        let svc = make_service(repo);
        svc.enable(42, 1).await.unwrap();
    }

    #[tokio::test]
    async fn delete_account_calls_both_storage_deletes() {
        let mut repo = MockAccountRepository::new();
        repo.expect_find_by_id_for_user()
            .returning(|_, user_id, account_id| Box::pin(async move { Ok(Some(fake_existing_account(account_id, user_id))) }));
        repo.expect_delete().returning(|_, acct| Box::pin(async move { Ok(acct) }));

        let mut raw = MockRawStorageService::new();
        raw.expect_delete_account().with(eq(1u64)).times(1).returning(|_| Box::pin(async { Ok(()) }));

        let mut attach = MockAttachmentStorageService::new();
        attach.expect_delete_account().with(eq(1u64)).times(1).returning(|_| Box::pin(async { Ok(()) }));

        let svc = make_service_with_storage(repo, raw, attach);
        svc.delete_account(42, 1).await.unwrap();
    }
}
