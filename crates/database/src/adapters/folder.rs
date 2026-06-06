use chrono::{DateTime, Utc};
use mk_core::{
    Error, RepositoryError,
    account::AccountId,
    folder::{Folder, FolderId, FolderRepository, FolderToken, NewFolderRow, SpecialUse},
    repository::Transaction,
};
use sea_orm::{ActiveModelTrait, ActiveValue::Set, ColumnTrait, EntityTrait, ModelTrait, QueryFilter};

use crate::{
    entities::{folders, prelude},
    error::handle_dberr,
    transaction::TransactionImpl,
};

impl From<folders::Model> for Folder {
    fn from(model: folders::Model) -> Self {
        let token = FolderToken::new(model.id as u64);
        let special_use = model.special_use.as_deref().and_then(|s| s.parse::<SpecialUse>().ok());
        Self {
            id: model.id as u64,
            version: model.version as u64,
            token,
            account_id: model.account_id as u64,
            path: model.path,
            display_name: model.display_name,
            special_use,
            enabled: model.enabled,
            idle_enabled: model.idle_enabled,
            uidvalidity: model.uidvalidity.map(|v| v as u32),
            last_uid: model.last_uid as u32,
            last_synced_at: model.last_synced_at.map(|d| d.with_timezone(&Utc)),
            last_error: model.last_error,
            created_at: model.created_at.with_timezone(&Utc),
            updated_at: model.updated_at.with_timezone(&Utc),
        }
    }
}

pub(crate) struct FolderRepositoryAdapter;

impl FolderRepositoryAdapter {
    pub(crate) fn new() -> Self {
        Self
    }
}

#[async_trait::async_trait]
impl FolderRepository for FolderRepositoryAdapter {
    async fn create_many(&self, transaction: &dyn Transaction, account_id: AccountId, folders: Vec<NewFolderRow>) -> Result<Vec<Folder>, Error> {
        if account_id == 0 {
            return Err(Error::InvalidId(account_id));
        }
        let transaction = TransactionImpl::get_db_transaction(transaction)?;
        let now = Utc::now();

        let mut inserted = Vec::with_capacity(folders.len());
        for row in folders {
            if row.account_id != account_id {
                return Err(Error::Validation("NewFolderRow.account_id must match the create_many account_id".into()));
            }
            let model = folders::ActiveModel {
                id: Set(row.token.id() as i64),
                version: Set(0),
                token: Set(row.token.to_string()),
                account_id: Set(account_id as i64),
                path: Set(row.path),
                display_name: Set(row.display_name),
                special_use: Set(row.special_use.map(|s| s.as_str().to_string())),
                enabled: Set(true),
                idle_enabled: Set(row.idle_enabled),
                uidvalidity: Set(row.uidvalidity.map(i64::from)),
                last_uid: Set(0),
                last_synced_at: Set(None),
                last_error: Set(None),
                created_at: Set(now.into()),
                updated_at: Set(now.into()),
            };
            let saved = model.insert(transaction).await.map_err(handle_dberr)?;
            inserted.push(saved.into());
        }
        Ok(inserted)
    }

    async fn find_by_id_for_account(&self, transaction: &dyn Transaction, account_id: AccountId, folder_id: FolderId) -> Result<Option<Folder>, Error> {
        if account_id == 0 {
            return Err(Error::InvalidId(account_id));
        }
        if folder_id == 0 {
            return Err(Error::InvalidId(folder_id));
        }
        let transaction = TransactionImpl::get_db_transaction(transaction)?;
        Ok(prelude::Folders::find_by_id(folder_id as i64)
            .filter(folders::Column::AccountId.eq(account_id as i64))
            .one(transaction)
            .await
            .map_err(handle_dberr)?
            .map(Into::into))
    }

    async fn find_by_account_and_path(&self, transaction: &dyn Transaction, account_id: AccountId, path: &str) -> Result<Option<Folder>, Error> {
        if account_id == 0 {
            return Err(Error::InvalidId(account_id));
        }
        let transaction = TransactionImpl::get_db_transaction(transaction)?;
        Ok(prelude::Folders::find()
            .filter(folders::Column::AccountId.eq(account_id as i64))
            .filter(folders::Column::Path.eq(path))
            .one(transaction)
            .await
            .map_err(handle_dberr)?
            .map(Into::into))
    }

    async fn list_for_account(&self, transaction: &dyn Transaction, account_id: AccountId) -> Result<Vec<Folder>, Error> {
        if account_id == 0 {
            return Err(Error::InvalidId(account_id));
        }
        let transaction = TransactionImpl::get_db_transaction(transaction)?;
        let rows = prelude::Folders::find()
            .filter(folders::Column::AccountId.eq(account_id as i64))
            .all(transaction)
            .await
            .map_err(handle_dberr)?;
        Ok(rows.into_iter().map(Into::into).collect())
    }

    async fn list_enabled_for_account(&self, transaction: &dyn Transaction, account_id: AccountId) -> Result<Vec<Folder>, Error> {
        if account_id == 0 {
            return Err(Error::InvalidId(account_id));
        }
        let transaction = TransactionImpl::get_db_transaction(transaction)?;
        let rows = prelude::Folders::find()
            .filter(folders::Column::AccountId.eq(account_id as i64))
            .filter(folders::Column::Enabled.eq(true))
            .all(transaction)
            .await
            .map_err(handle_dberr)?;
        Ok(rows.into_iter().map(Into::into).collect())
    }

    async fn update_enabled(&self, transaction: &dyn Transaction, folder_id: FolderId, enabled: bool) -> Result<(), Error> {
        if folder_id == 0 {
            return Err(Error::InvalidId(folder_id));
        }
        let transaction = TransactionImpl::get_db_transaction(transaction)?;
        let existing = prelude::Folders::find_by_id(folder_id as i64)
            .one(transaction)
            .await
            .map_err(handle_dberr)?
            .ok_or(Error::RepositoryError(RepositoryError::NotFound))?;

        let mut updater: folders::ActiveModel = existing.into();
        updater.enabled = Set(enabled);
        updater.update(transaction).await.map_err(handle_dberr)?;
        Ok(())
    }

    async fn update_idle_enabled(&self, transaction: &dyn Transaction, folder_id: FolderId, idle_enabled: bool) -> Result<(), Error> {
        if folder_id == 0 {
            return Err(Error::InvalidId(folder_id));
        }
        let transaction = TransactionImpl::get_db_transaction(transaction)?;
        let existing = prelude::Folders::find_by_id(folder_id as i64)
            .one(transaction)
            .await
            .map_err(handle_dberr)?
            .ok_or(Error::RepositoryError(RepositoryError::NotFound))?;

        let mut updater: folders::ActiveModel = existing.into();
        updater.idle_enabled = Set(idle_enabled);
        updater.update(transaction).await.map_err(handle_dberr)?;
        Ok(())
    }

    async fn update_sync_state(
        &self,
        transaction: &dyn Transaction,
        folder_id: FolderId,
        uidvalidity: u32,
        last_uid: u32,
        last_synced_at: DateTime<Utc>,
    ) -> Result<(), Error> {
        if folder_id == 0 {
            return Err(Error::InvalidId(folder_id));
        }
        let transaction = TransactionImpl::get_db_transaction(transaction)?;
        let existing = prelude::Folders::find_by_id(folder_id as i64)
            .one(transaction)
            .await
            .map_err(handle_dberr)?
            .ok_or(Error::RepositoryError(RepositoryError::NotFound))?;

        let mut updater: folders::ActiveModel = existing.into();
        updater.uidvalidity = Set(Some(i64::from(uidvalidity)));
        updater.last_uid = Set(i64::from(last_uid));
        updater.last_synced_at = Set(Some(last_synced_at.into()));
        updater.update(transaction).await.map_err(handle_dberr)?;
        Ok(())
    }

    async fn delete_by_id(&self, transaction: &dyn Transaction, folder_id: FolderId) -> Result<(), Error> {
        if folder_id == 0 {
            return Err(Error::InvalidId(folder_id));
        }
        let transaction = TransactionImpl::get_db_transaction(transaction)?;
        let existing = prelude::Folders::find_by_id(folder_id as i64)
            .one(transaction)
            .await
            .map_err(handle_dberr)?
            .ok_or(Error::RepositoryError(RepositoryError::NotFound))?;

        existing.delete(transaction).await.map_err(handle_dberr)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::HashSet, sync::Arc};

    use chrono::Utc;
    use mk_core::{
        Error, RepositoryError,
        account::{AccountToken, NewAccount},
        crypto::Ciphertext,
        folder::{FolderToken, NewFolderRow, SpecialUse},
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

    async fn make_account(svc: &Arc<RepositoryService>, user_id: u64, host: &str) -> u64 {
        let tx = svc.repository().begin().await.unwrap();
        let token = AccountToken::generate();
        let na = NewAccount {
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
        };
        let acct = svc.account_repository().insert(&*tx, na).await.unwrap();
        tx.commit().await.unwrap();
        acct.id
    }

    fn new_row(account_id: u64, path: &str, special_use: Option<SpecialUse>, idle_enabled: bool) -> NewFolderRow {
        NewFolderRow {
            token: FolderToken::generate(),
            account_id,
            path: path.to_string(),
            display_name: None,
            special_use,
            idle_enabled,
            uidvalidity: None,
        }
    }

    #[tokio::test]
    async fn create_many_round_trip() {
        let svc = setup().await;
        let user_id = make_user(&svc, "alice", "alice@example.com").await;
        let account_id = make_account(&svc, user_id, "example.com").await;
        let tx = svc.repository().begin().await.unwrap();

        let rows = vec![
            new_row(account_id, "INBOX", Some(SpecialUse::Inbox), true),
            new_row(account_id, "Sent", Some(SpecialUse::Sent), false),
        ];
        let inserted = svc.folder_repository().create_many(&*tx, account_id, rows).await.unwrap();
        assert_eq!(inserted.len(), 2);

        // before_save bumps the written 0 to 1 on insert.
        assert!(inserted.iter().all(|f| f.version == 1));
        assert!(inserted.iter().all(|f| f.account_id == account_id));
        assert!(inserted.iter().all(|f| f.enabled));
        assert!(inserted.iter().all(|f| f.last_uid == 0));

        let inbox = inserted.iter().find(|f| f.path == "INBOX").unwrap();
        assert_eq!(inbox.special_use, Some(SpecialUse::Inbox));
        assert!(inbox.idle_enabled);

        let sent = inserted.iter().find(|f| f.path == "Sent").unwrap();
        assert_eq!(sent.special_use, Some(SpecialUse::Sent));
        assert!(!sent.idle_enabled);

        let listed = svc.folder_repository().list_for_account(&*tx, account_id).await.unwrap();
        assert_eq!(listed.len(), 2);
    }

    #[tokio::test]
    async fn unique_account_path_constraint() {
        let svc = setup().await;
        let user_id = make_user(&svc, "alice", "alice@example.com").await;
        let account_id = make_account(&svc, user_id, "example.com").await;
        let tx = svc.repository().begin().await.unwrap();

        let rows = vec![new_row(account_id, "INBOX", Some(SpecialUse::Inbox), true)];
        svc.folder_repository().create_many(&*tx, account_id, rows).await.unwrap();

        let dup = vec![new_row(account_id, "INBOX", None, false)];
        let result = svc.folder_repository().create_many(&*tx, account_id, dup).await;
        assert!(matches!(result, Err(Error::RepositoryError(RepositoryError::Constraint(_)))));
    }

    #[tokio::test]
    async fn update_sync_state_bumps_version() {
        let svc = setup().await;
        let user_id = make_user(&svc, "alice", "alice@example.com").await;
        let account_id = make_account(&svc, user_id, "example.com").await;
        let tx = svc.repository().begin().await.unwrap();

        let inserted = svc
            .folder_repository()
            .create_many(&*tx, account_id, vec![new_row(account_id, "INBOX", Some(SpecialUse::Inbox), true)])
            .await
            .unwrap();
        let folder = &inserted[0];
        let version_before = folder.version;

        svc.folder_repository().update_sync_state(&*tx, folder.id, 42, 1000, Utc::now()).await.unwrap();

        let fetched = svc
            .folder_repository()
            .find_by_id_for_account(&*tx, account_id, folder.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(fetched.version, version_before + 1);
        assert_eq!(fetched.uidvalidity, Some(42));
        assert_eq!(fetched.last_uid, 1000);
        assert!(fetched.last_synced_at.is_some());
    }

    #[tokio::test]
    async fn update_enabled_persists() {
        let svc = setup().await;
        let user_id = make_user(&svc, "alice", "alice@example.com").await;
        let account_id = make_account(&svc, user_id, "example.com").await;
        let tx = svc.repository().begin().await.unwrap();

        let inserted = svc
            .folder_repository()
            .create_many(&*tx, account_id, vec![new_row(account_id, "INBOX", Some(SpecialUse::Inbox), true)])
            .await
            .unwrap();
        let folder_id = inserted[0].id;

        svc.folder_repository().update_enabled(&*tx, folder_id, false).await.unwrap();
        let fetched = svc
            .folder_repository()
            .find_by_id_for_account(&*tx, account_id, folder_id)
            .await
            .unwrap()
            .unwrap();
        assert!(!fetched.enabled);
    }

    #[tokio::test]
    async fn update_idle_enabled_persists() {
        let svc = setup().await;
        let user_id = make_user(&svc, "alice", "alice@example.com").await;
        let account_id = make_account(&svc, user_id, "example.com").await;
        let tx = svc.repository().begin().await.unwrap();

        let inserted = svc
            .folder_repository()
            .create_many(&*tx, account_id, vec![new_row(account_id, "Sent", Some(SpecialUse::Sent), false)])
            .await
            .unwrap();
        let folder_id = inserted[0].id;

        svc.folder_repository().update_idle_enabled(&*tx, folder_id, true).await.unwrap();
        let fetched = svc
            .folder_repository()
            .find_by_id_for_account(&*tx, account_id, folder_id)
            .await
            .unwrap()
            .unwrap();
        assert!(fetched.idle_enabled);
    }

    #[tokio::test]
    async fn delete_by_id_removes_row() {
        let svc = setup().await;
        let user_id = make_user(&svc, "alice", "alice@example.com").await;
        let account_id = make_account(&svc, user_id, "example.com").await;
        let tx = svc.repository().begin().await.unwrap();

        let inserted = svc
            .folder_repository()
            .create_many(&*tx, account_id, vec![new_row(account_id, "INBOX", Some(SpecialUse::Inbox), true)])
            .await
            .unwrap();
        let folder_id = inserted[0].id;

        svc.folder_repository().delete_by_id(&*tx, folder_id).await.unwrap();
        let fetched = svc.folder_repository().find_by_id_for_account(&*tx, account_id, folder_id).await.unwrap();
        assert!(fetched.is_none());
    }

    #[tokio::test]
    async fn cascade_on_account_delete() {
        let svc = setup().await;
        let user_id = make_user(&svc, "alice", "alice@example.com").await;
        let account_id = make_account(&svc, user_id, "example.com").await;
        let tx = svc.repository().begin().await.unwrap();

        let inserted = svc
            .folder_repository()
            .create_many(&*tx, account_id, vec![new_row(account_id, "INBOX", Some(SpecialUse::Inbox), true)])
            .await
            .unwrap();
        let folder_id = inserted[0].id;

        let account = svc.account_repository().find_by_id_for_user(&*tx, user_id, account_id).await.unwrap().unwrap();
        svc.account_repository().delete(&*tx, account).await.unwrap();

        let after = svc.folder_repository().find_by_id_for_account(&*tx, account_id, folder_id).await.unwrap();
        assert!(after.is_none(), "FK cascade should have deleted the folder row");
    }

    #[tokio::test]
    async fn find_by_account_and_path_returns_none_for_other_account() {
        let svc = setup().await;
        let alice = make_user(&svc, "alice", "alice@example.com").await;
        let bob = make_user(&svc, "bob", "bob@example.com").await;
        let alice_acct = make_account(&svc, alice, "alice.com").await;
        let bob_acct = make_account(&svc, bob, "bob.com").await;
        let tx = svc.repository().begin().await.unwrap();

        svc.folder_repository()
            .create_many(&*tx, alice_acct, vec![new_row(alice_acct, "INBOX", Some(SpecialUse::Inbox), true)])
            .await
            .unwrap();

        let from_bob = svc.folder_repository().find_by_account_and_path(&*tx, bob_acct, "INBOX").await.unwrap();
        assert!(from_bob.is_none());

        let from_alice = svc.folder_repository().find_by_account_and_path(&*tx, alice_acct, "INBOX").await.unwrap();
        assert!(from_alice.is_some());
    }
}
