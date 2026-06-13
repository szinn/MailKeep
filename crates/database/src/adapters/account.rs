use chrono::Utc;
use mk_core::{
    Error, RepositoryError,
    account::{Account, AccountId, AccountRepository, AccountStatus, AccountToken, NewAccount},
    crypto::Ciphertext,
    imap::ImapServerConfig,
    repository::Transaction,
    types::EmailAddress,
    user::UserId,
};
use sea_orm::{ActiveModelTrait, ActiveValue::Set, ColumnTrait, EntityTrait, ModelTrait, QueryFilter};

use crate::{
    entities::{accounts, prelude},
    error::handle_dberr,
    transaction::TransactionImpl,
};

impl From<accounts::Model> for Account {
    fn from(model: accounts::Model) -> Self {
        let token = AccountToken::new(model.id as u64);
        let email_address = EmailAddress::new(model.email_address).expect("database email_address should be valid");
        let server: ImapServerConfig = serde_json::from_str(&model.server).expect("database server JSON should be valid");
        let status = AccountStatus::from_db_str(&model.status).expect("database status string should match AccountStatus variant");

        Self {
            id: model.id as u64,
            version: model.version as u64,
            token,
            user_id: model.user_id as u64,
            display_name: model.display_name,
            email_address,
            server,
            username: model.username,
            credentials: Ciphertext::from_raw(model.credentials),
            enabled: model.enabled,
            status,
            last_error: model.last_error,
            last_synced_at: model.last_synced_at.map(|d| d.with_timezone(&Utc)),
            created_at: model.created_at.with_timezone(&Utc),
            updated_at: model.updated_at.with_timezone(&Utc),
        }
    }
}

pub(crate) struct AccountRepositoryAdapter;

impl AccountRepositoryAdapter {
    pub(crate) fn new() -> Self {
        Self
    }
}

#[async_trait::async_trait]
impl AccountRepository for AccountRepositoryAdapter {
    async fn insert(&self, transaction: &dyn Transaction, new: NewAccount) -> Result<Account, Error> {
        let transaction = TransactionImpl::get_db_transaction(transaction)?;
        let now = Utc::now();
        let server = serde_json::to_string(&new.server).map_err(|e| Error::Infrastructure(e.to_string()))?;

        let model = accounts::ActiveModel {
            id: Set(new.token.id() as i64),
            token: Set(new.token.to_string()),
            user_id: Set(new.user_id as i64),
            display_name: Set(new.display_name),
            email_address: Set(new.email_address.into_inner()),
            server: Set(server),
            username: Set(new.username),
            credentials: Set(new.credentials.as_bytes().to_vec()),
            enabled: Set(true),
            status: Set(AccountStatus::PendingFirstSync.as_str().to_string()),
            last_error: Set(None),
            last_synced_at: Set(None),
            version: Set(0),
            created_at: Set(now.into()),
            updated_at: Set(now.into()),
        };
        let model = model.insert(transaction).await.map_err(handle_dberr)?;
        Ok(model.into())
    }

    async fn find_by_id_for_user(&self, transaction: &dyn Transaction, user_id: UserId, account_id: AccountId) -> Result<Option<Account>, Error> {
        if account_id == 0 {
            return Err(Error::InvalidId(account_id));
        }
        let transaction = TransactionImpl::get_db_transaction(transaction)?;
        Ok(prelude::Accounts::find_by_id(account_id as i64)
            .filter(accounts::Column::UserId.eq(user_id as i64))
            .one(transaction)
            .await
            .map_err(handle_dberr)?
            .map(Into::into))
    }

    async fn list_for_user(&self, transaction: &dyn Transaction, user_id: UserId) -> Result<Vec<Account>, Error> {
        let transaction = TransactionImpl::get_db_transaction(transaction)?;
        let rows = prelude::Accounts::find()
            .filter(accounts::Column::UserId.eq(user_id as i64))
            .all(transaction)
            .await
            .map_err(handle_dberr)?;
        Ok(rows.into_iter().map(Into::into).collect())
    }

    async fn list_enabled(&self, transaction: &dyn Transaction) -> Result<Vec<Account>, Error> {
        let transaction = TransactionImpl::get_db_transaction(transaction)?;
        let rows = prelude::Accounts::find()
            .filter(accounts::Column::Enabled.eq(true))
            .all(transaction)
            .await
            .map_err(handle_dberr)?;
        Ok(rows.into_iter().map(Into::into).collect())
    }

    async fn update(&self, transaction: &dyn Transaction, account: Account) -> Result<Account, Error> {
        if account.id == 0 {
            return Err(Error::InvalidId(account.id));
        }
        let transaction = TransactionImpl::get_db_transaction(transaction)?;

        let existing = prelude::Accounts::find_by_id(account.id as i64)
            .one(transaction)
            .await
            .map_err(handle_dberr)?
            .ok_or(Error::RepositoryError(RepositoryError::NotFound))?;

        if existing.version != account.version as i64 {
            return Err(Error::RepositoryError(RepositoryError::Conflict));
        }

        let server = serde_json::to_string(&account.server).map_err(|e| Error::Infrastructure(e.to_string()))?;

        let mut updater: accounts::ActiveModel = existing.clone().into();
        if existing.display_name != account.display_name {
            updater.display_name = Set(account.display_name);
        }
        if existing.email_address != account.email_address.as_str() {
            updater.email_address = Set(account.email_address.into_inner());
        }
        if existing.server != server {
            updater.server = Set(server);
        }
        if existing.username != account.username {
            updater.username = Set(account.username);
        }
        if existing.credentials.as_slice() != account.credentials.as_bytes() {
            updater.credentials = Set(account.credentials.as_bytes().to_vec());
        }
        if existing.enabled != account.enabled {
            updater.enabled = Set(account.enabled);
        }
        let status_str = account.status.as_str();
        if existing.status != status_str {
            updater.status = Set(status_str.to_string());
        }
        if existing.last_error != account.last_error {
            updater.last_error = Set(account.last_error);
        }

        let result = updater.update(transaction).await.map_err(handle_dberr)?;
        Ok(result.into())
    }

    async fn set_status(&self, transaction: &dyn Transaction, account_id: AccountId, status: AccountStatus, last_error: Option<String>) -> Result<(), Error> {
        if account_id == 0 {
            return Err(Error::InvalidId(account_id));
        }
        let transaction = TransactionImpl::get_db_transaction(transaction)?;
        let existing = prelude::Accounts::find_by_id(account_id as i64)
            .one(transaction)
            .await
            .map_err(handle_dberr)?
            .ok_or(Error::RepositoryError(RepositoryError::NotFound))?;

        let mut updater: accounts::ActiveModel = existing.into();
        updater.status = Set(status.as_str().to_string());
        updater.last_error = Set(last_error);
        updater.update(transaction).await.map_err(handle_dberr)?;
        Ok(())
    }

    async fn set_enabled(&self, transaction: &dyn Transaction, account_id: AccountId, enabled: bool) -> Result<(), Error> {
        if account_id == 0 {
            return Err(Error::InvalidId(account_id));
        }
        let transaction = TransactionImpl::get_db_transaction(transaction)?;
        let existing = prelude::Accounts::find_by_id(account_id as i64)
            .one(transaction)
            .await
            .map_err(handle_dberr)?
            .ok_or(Error::RepositoryError(RepositoryError::NotFound))?;

        let mut updater: accounts::ActiveModel = existing.into();
        updater.enabled = Set(enabled);
        updater.update(transaction).await.map_err(handle_dberr)?;
        Ok(())
    }

    async fn delete(&self, transaction: &dyn Transaction, account: Account) -> Result<Account, Error> {
        if account.id == 0 {
            return Err(Error::InvalidId(account.id));
        }
        let transaction = TransactionImpl::get_db_transaction(transaction)?;
        let existing = prelude::Accounts::find_by_id(account.id as i64)
            .one(transaction)
            .await
            .map_err(handle_dberr)?
            .ok_or(Error::RepositoryError(RepositoryError::NotFound))?;

        if existing.version != account.version as i64 {
            return Err(Error::RepositoryError(RepositoryError::Conflict));
        }

        let returned: Account = existing.clone().into();
        existing.delete(transaction).await.map_err(handle_dberr)?;
        Ok(returned)
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::HashSet, sync::Arc};

    use mk_core::{
        Error, RepositoryError,
        account::{AccountStatus, AccountToken, NewAccount},
        crypto::Ciphertext,
        imap::{ImapServerConfig, TlsMode},
        repository::RepositoryService,
        types::EmailAddress,
        user::NewUser,
    };
    use sea_orm::Database;

    use crate::create_repository_service;

    async fn setup() -> Arc<RepositoryService> {
        let db = Database::connect("sqlite::memory:").await.unwrap();
        create_repository_service(db).await.unwrap()
    }

    async fn make_user(svc: &Arc<RepositoryService>, username: &str, email: &str) -> u64 {
        let tx = svc.repository().begin().await.unwrap();
        let new_user = NewUser::new(username, "hash", email, HashSet::new(), "Test", false).unwrap();
        let user = svc.user_repository().add_user(&*tx, new_user).await.unwrap();
        tx.commit().await.unwrap();
        user.id
    }

    fn new_account(user_id: u64, host: &str) -> NewAccount {
        let token = AccountToken::generate();
        NewAccount {
            user_id,
            display_name: format!("{host} Account"),
            email_address: EmailAddress::new(format!("user@{host}")).unwrap(),
            server: ImapServerConfig {
                host: host.to_string(),
                port: 993,
                tls: TlsMode::Tls,
            },
            username: format!("user@{host}"),
            credentials: Ciphertext::from_raw(vec![0u8; 28]),
            token,
        }
    }

    #[tokio::test]
    async fn insert_persists_row_with_version_zero() {
        let svc = setup().await;
        let user_id = make_user(&svc, "alice", "alice@example.com").await;
        let tx = svc.repository().begin().await.unwrap();

        let inserted = svc.account_repository().insert(&*tx, new_account(user_id, "example.com")).await.unwrap();
        // before_save in ActiveModelBehavior bumps the written 0 to 1 on insert.
        assert_eq!(inserted.version, 1);
        assert!(inserted.id > 0);
        assert_eq!(inserted.user_id, user_id);
        assert_eq!(inserted.status, AccountStatus::PendingFirstSync);
        assert!(inserted.enabled);
    }

    #[tokio::test]
    async fn find_by_id_for_user_cross_tenant_returns_none() {
        let svc = setup().await;
        let alice = make_user(&svc, "alice", "alice@example.com").await;
        let bob = make_user(&svc, "bob", "bob@example.com").await;
        let tx = svc.repository().begin().await.unwrap();

        let alice_acct = svc.account_repository().insert(&*tx, new_account(alice, "alice.com")).await.unwrap();

        let from_bob = svc.account_repository().find_by_id_for_user(&*tx, bob, alice_acct.id).await.unwrap();
        assert!(from_bob.is_none());

        let from_alice = svc.account_repository().find_by_id_for_user(&*tx, alice, alice_acct.id).await.unwrap();
        assert!(from_alice.is_some());
    }

    #[tokio::test]
    async fn list_for_user_scoping_works() {
        let svc = setup().await;
        let alice = make_user(&svc, "alice", "alice@example.com").await;
        let bob = make_user(&svc, "bob", "bob@example.com").await;
        let tx = svc.repository().begin().await.unwrap();

        svc.account_repository().insert(&*tx, new_account(alice, "alice1.com")).await.unwrap();
        svc.account_repository().insert(&*tx, new_account(alice, "alice2.com")).await.unwrap();
        svc.account_repository().insert(&*tx, new_account(bob, "bob.com")).await.unwrap();

        let alice_list = svc.account_repository().list_for_user(&*tx, alice).await.unwrap();
        assert_eq!(alice_list.len(), 2);
        assert!(alice_list.iter().all(|a| a.user_id == alice));

        let bob_list = svc.account_repository().list_for_user(&*tx, bob).await.unwrap();
        assert_eq!(bob_list.len(), 1);
    }

    #[tokio::test]
    async fn list_enabled_returns_only_enabled_across_users() {
        let svc = setup().await;
        let alice = make_user(&svc, "alice", "alice@example.com").await;
        let bob = make_user(&svc, "bob", "bob@example.com").await;
        let tx = svc.repository().begin().await.unwrap();

        let alice_a = svc.account_repository().insert(&*tx, new_account(alice, "a.com")).await.unwrap();
        let _bob_a = svc.account_repository().insert(&*tx, new_account(bob, "b.com")).await.unwrap();
        svc.account_repository().set_enabled(&*tx, alice_a.id, false).await.unwrap();

        let enabled = svc.account_repository().list_enabled(&*tx).await.unwrap();
        assert_eq!(enabled.len(), 1);
        assert_ne!(enabled[0].id, alice_a.id);
    }

    #[tokio::test]
    async fn update_with_correct_version_succeeds_and_bumps() {
        let svc = setup().await;
        let user_id = make_user(&svc, "alice", "alice@example.com").await;
        let tx = svc.repository().begin().await.unwrap();
        let mut acct = svc.account_repository().insert(&*tx, new_account(user_id, "ex.com")).await.unwrap();

        let version_before = acct.version;
        acct.display_name = "Renamed".to_string();
        let updated = svc.account_repository().update(&*tx, acct).await.unwrap();
        assert_eq!(updated.display_name, "Renamed");
        assert_eq!(updated.version, version_before + 1);
    }

    #[tokio::test]
    async fn update_with_stale_version_returns_conflict() {
        let svc = setup().await;
        let user_id = make_user(&svc, "alice", "alice@example.com").await;
        let tx = svc.repository().begin().await.unwrap();
        let mut acct = svc.account_repository().insert(&*tx, new_account(user_id, "ex.com")).await.unwrap();

        acct.version = 99;
        acct.display_name = "Stale".to_string();
        let err = svc.account_repository().update(&*tx, acct).await;
        assert!(matches!(err, Err(Error::RepositoryError(RepositoryError::Conflict))));
    }

    #[tokio::test]
    async fn set_status_persists_with_and_without_last_error() {
        let svc = setup().await;
        let user_id = make_user(&svc, "alice", "alice@example.com").await;
        let tx = svc.repository().begin().await.unwrap();
        let acct = svc.account_repository().insert(&*tx, new_account(user_id, "ex.com")).await.unwrap();

        svc.account_repository()
            .set_status(&*tx, acct.id, AccountStatus::Error, Some("boom".into()))
            .await
            .unwrap();
        let after_err = svc.account_repository().find_by_id_for_user(&*tx, user_id, acct.id).await.unwrap().unwrap();
        assert_eq!(after_err.status, AccountStatus::Error);
        assert_eq!(after_err.last_error.as_deref(), Some("boom"));

        svc.account_repository().set_status(&*tx, acct.id, AccountStatus::Idle, None).await.unwrap();
        let after_idle = svc.account_repository().find_by_id_for_user(&*tx, user_id, acct.id).await.unwrap().unwrap();
        assert_eq!(after_idle.status, AccountStatus::Idle);
        assert!(after_idle.last_error.is_none());
    }

    #[tokio::test]
    async fn set_enabled_persists() {
        let svc = setup().await;
        let user_id = make_user(&svc, "alice", "alice@example.com").await;
        let tx = svc.repository().begin().await.unwrap();
        let acct = svc.account_repository().insert(&*tx, new_account(user_id, "ex.com")).await.unwrap();

        svc.account_repository().set_enabled(&*tx, acct.id, false).await.unwrap();
        let after = svc.account_repository().find_by_id_for_user(&*tx, user_id, acct.id).await.unwrap().unwrap();
        assert!(!after.enabled);
    }

    #[tokio::test]
    async fn delete_removes_row() {
        let svc = setup().await;
        let user_id = make_user(&svc, "alice", "alice@example.com").await;
        let tx = svc.repository().begin().await.unwrap();
        let acct = svc.account_repository().insert(&*tx, new_account(user_id, "ex.com")).await.unwrap();
        let id = acct.id;

        svc.account_repository().delete(&*tx, acct).await.unwrap();
        let after = svc.account_repository().find_by_id_for_user(&*tx, user_id, id).await.unwrap();
        assert!(after.is_none());
    }

    #[tokio::test]
    async fn credentials_column_is_binary_round_trip() {
        let svc = setup().await;
        let user_id = make_user(&svc, "alice", "alice@example.com").await;
        let tx = svc.repository().begin().await.unwrap();

        let mut na = new_account(user_id, "ex.com");
        let bytes: Vec<u8> = (0u8..40).collect();
        na.credentials = Ciphertext::from_raw(bytes.clone());
        let inserted = svc.account_repository().insert(&*tx, na).await.unwrap();

        let fetched = svc.account_repository().find_by_id_for_user(&*tx, user_id, inserted.id).await.unwrap().unwrap();
        assert_eq!(fetched.credentials.as_bytes(), &*bytes);
    }

    #[tokio::test]
    async fn server_column_round_trips_json() {
        let svc = setup().await;
        let user_id = make_user(&svc, "alice", "alice@example.com").await;
        let tx = svc.repository().begin().await.unwrap();

        let mut na = new_account(user_id, "ex.com");
        na.server = ImapServerConfig {
            host: "imap.fastmail.com".into(),
            port: 143,
            tls: TlsMode::StartTls,
        };
        let inserted = svc.account_repository().insert(&*tx, na).await.unwrap();

        let fetched = svc.account_repository().find_by_id_for_user(&*tx, user_id, inserted.id).await.unwrap().unwrap();
        assert_eq!(fetched.server.host, "imap.fastmail.com");
        assert_eq!(fetched.server.port, 143);
        assert_eq!(fetched.server.tls, TlsMode::StartTls);
    }

    #[tokio::test]
    async fn cascade_delete_when_user_deleted() {
        let svc = setup().await;
        let user_id = make_user(&svc, "alice", "alice@example.com").await;
        let tx = svc.repository().begin().await.unwrap();

        let acct = svc.account_repository().insert(&*tx, new_account(user_id, "ex.com")).await.unwrap();
        let account_id = acct.id;
        let user = svc.user_repository().find_by_id(&*tx, user_id).await.unwrap().unwrap();
        svc.user_repository().delete_user(&*tx, user).await.unwrap();

        let after = svc.account_repository().find_by_id_for_user(&*tx, user_id, account_id).await.unwrap();
        assert!(after.is_none(), "FK cascade should have deleted the account row");
    }
}
