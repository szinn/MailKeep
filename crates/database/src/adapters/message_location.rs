use chrono::Utc;
use mk_core::{
    Error,
    folder::FolderId,
    message::{MessageId, MessageLocation, MessageLocationRepository, MessageLocationToken, NewMessageLocationRow},
    repository::Transaction,
};
use sea_orm::{ActiveModelTrait, ActiveValue::Set, ColumnTrait, EntityTrait, QueryFilter, QuerySelect};

use crate::{
    entities::{message_locations, prelude},
    error::handle_dberr,
    transaction::TransactionImpl,
};

impl From<message_locations::Model> for MessageLocation {
    fn from(model: message_locations::Model) -> Self {
        let token = MessageLocationToken::new(model.id as u64);
        Self {
            id: model.id as u64,
            version: model.version as u64,
            token,
            message_id: model.message_id as u64,
            folder_id: model.folder_id as u64,
            uid: model.uid as u32,
            uidvalidity: model.uidvalidity as u32,
            flags: serde_json::from_value(model.flags).expect("database flags JSON should be valid"),
            internal_date: model.internal_date.with_timezone(&Utc),
            first_seen_at: model.first_seen_at.with_timezone(&Utc),
            updated_at: model.updated_at.with_timezone(&Utc),
        }
    }
}

pub(crate) struct MessageLocationRepositoryAdapter;

impl MessageLocationRepositoryAdapter {
    pub(crate) fn new() -> Self {
        Self
    }
}

#[async_trait::async_trait]
impl MessageLocationRepository for MessageLocationRepositoryAdapter {
    async fn find_by_message_and_folder(
        &self,
        transaction: &dyn Transaction,
        message_id: MessageId,
        folder_id: FolderId,
    ) -> Result<Option<MessageLocation>, Error> {
        if message_id == 0 {
            return Err(Error::InvalidId(message_id));
        }
        if folder_id == 0 {
            return Err(Error::InvalidId(folder_id));
        }
        let transaction = TransactionImpl::get_db_transaction(transaction)?;
        Ok(prelude::MessageLocations::find()
            .filter(message_locations::Column::MessageId.eq(message_id as i64))
            .filter(message_locations::Column::FolderId.eq(folder_id as i64))
            .one(transaction)
            .await
            .map_err(handle_dberr)?
            .map(Into::into))
    }

    async fn upsert(&self, transaction: &dyn Transaction, new: NewMessageLocationRow) -> Result<MessageLocation, Error> {
        if new.message_id == 0 {
            return Err(Error::InvalidId(new.message_id));
        }
        if new.folder_id == 0 {
            return Err(Error::InvalidId(new.folder_id));
        }
        let transaction = TransactionImpl::get_db_transaction(transaction)?;
        let now = Utc::now();
        let flags_json = serde_json::to_value(&new.flags).map_err(|e| Error::Infrastructure(e.to_string()))?;

        // Find-then-update-or-insert.
        let existing = prelude::MessageLocations::find()
            .filter(message_locations::Column::MessageId.eq(new.message_id as i64))
            .filter(message_locations::Column::FolderId.eq(new.folder_id as i64))
            .one(transaction)
            .await
            .map_err(handle_dberr)?;

        if let Some(found) = existing {
            let mut updater: message_locations::ActiveModel = found.into();
            updater.uid = Set(i64::from(new.uid));
            updater.uidvalidity = Set(i64::from(new.uidvalidity));
            updater.flags = Set(flags_json);
            updater.internal_date = Set(new.internal_date.into());
            let saved = updater.update(transaction).await.map_err(handle_dberr)?;
            Ok(saved.into())
        } else {
            let model = message_locations::ActiveModel {
                id: Set(new.token.id() as i64),
                version: Set(0),
                token: Set(new.token.to_string()),
                message_id: Set(new.message_id as i64),
                folder_id: Set(new.folder_id as i64),
                uid: Set(i64::from(new.uid)),
                uidvalidity: Set(i64::from(new.uidvalidity)),
                flags: Set(flags_json),
                internal_date: Set(new.internal_date.into()),
                first_seen_at: Set(now.into()),
                updated_at: Set(now.into()),
            };
            let saved = model.insert(transaction).await.map_err(handle_dberr)?;
            Ok(saved.into())
        }
    }

    async fn delete_by_folder_id(&self, transaction: &dyn Transaction, folder_id: FolderId) -> Result<u64, Error> {
        if folder_id == 0 {
            return Err(Error::InvalidId(folder_id));
        }
        let transaction = TransactionImpl::get_db_transaction(transaction)?;
        let res = prelude::MessageLocations::delete_many()
            .filter(message_locations::Column::FolderId.eq(folder_id as i64))
            .exec(transaction)
            .await
            .map_err(handle_dberr)?;
        Ok(res.rows_affected)
    }

    async fn filter_message_ids_in_folders(
        &self,
        transaction: &dyn Transaction,
        message_ids: &[MessageId],
        folder_ids: &[FolderId],
    ) -> Result<Vec<MessageId>, Error> {
        if message_ids.is_empty() || folder_ids.is_empty() {
            return Ok(Vec::new());
        }
        let transaction = TransactionImpl::get_db_transaction(transaction)?;
        let msg_id_list: Vec<i64> = message_ids.iter().map(|id| *id as i64).collect();
        let folder_id_list: Vec<i64> = folder_ids.iter().map(|id| *id as i64).collect();
        let rows: Vec<i64> = prelude::MessageLocations::find()
            .select_only()
            .column(message_locations::Column::MessageId)
            .distinct()
            .filter(message_locations::Column::MessageId.is_in(msg_id_list))
            .filter(message_locations::Column::FolderId.is_in(folder_id_list))
            .into_tuple()
            .all(transaction)
            .await
            .map_err(handle_dberr)?;
        Ok(rows.into_iter().map(|id| id as u64).collect())
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::HashSet, sync::Arc};

    use chrono::Utc;
    use mk_core::{
        account::{AccountToken, NewAccount},
        crypto::Ciphertext,
        folder::{FolderToken, NewFolderRow, SpecialUse},
        imap::{ImapServerConfig, TlsMode},
        message::{MessageFlags, MessageLocationToken, MessageToken, NewMessageLocationRow, NewMessageRow},
        repository::RepositoryService,
        types::{ContentHash, EmailAddress},
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

    async fn make_folder(svc: &Arc<RepositoryService>, account_id: u64, path: &str) -> u64 {
        let tx = svc.repository().begin().await.unwrap();
        let row = NewFolderRow {
            token: FolderToken::generate(),
            account_id,
            path: path.to_string(),
            display_name: None,
            special_use: Some(SpecialUse::Inbox),
            idle_enabled: true,
            uidvalidity: None,
        };
        let folders = svc.folder_repository().create_many(&*tx, account_id, vec![row]).await.unwrap();
        tx.commit().await.unwrap();
        folders[0].id
    }

    async fn make_message(svc: &Arc<RepositoryService>, account_id: u64, rfc_id: &str) -> u64 {
        let tx = svc.repository().begin().await.unwrap();
        let row = NewMessageRow {
            token: MessageToken::generate(),
            account_id,
            rfc822_message_id: rfc_id.to_string(),
            // Derive the hash from the (unique) id so distinct messages get
            // distinct content hashes — identity is (account_id, content_hash).
            content_hash: ContentHash::compute(rfc_id.as_bytes()),
            subject: None,
            from_address: EmailAddress::new("a@b.com").unwrap(),
            from_name: None,
            to_addresses: vec![],
            cc_addresses: vec![],
            bcc_addresses: vec![],
            reply_to_addresses: vec![],
            sent_date: None,
            in_reply_to: None,
            references: vec![],
            snippet: "x".into(),
            size_bytes: 10,
            has_attachments: false,
            attachment_count: 0,
        };
        let msg = svc.message_repository().create(&*tx, row).await.unwrap();
        tx.commit().await.unwrap();
        msg.id
    }

    fn loc_row(message_id: u64, folder_id: u64, uid: u32, flags: MessageFlags) -> NewMessageLocationRow {
        NewMessageLocationRow {
            token: MessageLocationToken::generate(),
            message_id,
            folder_id,
            uid,
            uidvalidity: 100,
            flags,
            internal_date: Utc::now(),
        }
    }

    #[tokio::test]
    async fn upsert_inserts_when_absent() {
        let svc = setup().await;
        let user_id = make_user(&svc, "alice", "alice@example.com").await;
        let account_id = make_account(&svc, user_id, "example.com").await;
        let folder_id = make_folder(&svc, account_id, "INBOX").await;
        let message_id = make_message(&svc, account_id, "<m1@x.com>").await;
        let tx = svc.repository().begin().await.unwrap();

        let saved = svc
            .message_location_repository()
            .upsert(&*tx, loc_row(message_id, folder_id, 1, MessageFlags::default()))
            .await
            .unwrap();
        assert_eq!(saved.message_id, message_id);
        assert_eq!(saved.folder_id, folder_id);
        assert_eq!(saved.uid, 1);
        assert_eq!(saved.version, 1);
    }

    #[tokio::test]
    async fn upsert_updates_when_present() {
        let svc = setup().await;
        let user_id = make_user(&svc, "alice", "alice@example.com").await;
        let account_id = make_account(&svc, user_id, "example.com").await;
        let folder_id = make_folder(&svc, account_id, "INBOX").await;
        let message_id = make_message(&svc, account_id, "<m1@x.com>").await;
        let tx = svc.repository().begin().await.unwrap();

        let first = svc
            .message_location_repository()
            .upsert(&*tx, loc_row(message_id, folder_id, 1, MessageFlags::default()))
            .await
            .unwrap();
        let first_id = first.id;
        let first_version = first.version;

        let new_flags = MessageFlags {
            seen: true,
            ..MessageFlags::default()
        };
        let second_row = NewMessageLocationRow {
            uid: 99,
            ..loc_row(message_id, folder_id, 99, new_flags.clone())
        };
        let second = svc.message_location_repository().upsert(&*tx, second_row).await.unwrap();

        assert_eq!(second.id, first_id, "upsert must not change row id");
        assert!(second.version > first_version, "upsert must bump version");
        assert_eq!(second.uid, 99);
        assert_eq!(second.flags, new_flags);
    }

    #[tokio::test]
    async fn delete_by_folder_id_removes_matching() {
        let svc = setup().await;
        let user_id = make_user(&svc, "alice", "alice@example.com").await;
        let account_id = make_account(&svc, user_id, "example.com").await;
        let folder_a = make_folder(&svc, account_id, "INBOX").await;
        let folder_b = make_folder(&svc, account_id, "Other").await;
        let msg1 = make_message(&svc, account_id, "<m1@x.com>").await;
        let msg2 = make_message(&svc, account_id, "<m2@x.com>").await;
        let tx = svc.repository().begin().await.unwrap();

        svc.message_location_repository()
            .upsert(&*tx, loc_row(msg1, folder_a, 1, MessageFlags::default()))
            .await
            .unwrap();
        svc.message_location_repository()
            .upsert(&*tx, loc_row(msg2, folder_a, 2, MessageFlags::default()))
            .await
            .unwrap();
        svc.message_location_repository()
            .upsert(&*tx, loc_row(msg1, folder_b, 3, MessageFlags::default()))
            .await
            .unwrap();

        let deleted = svc.message_location_repository().delete_by_folder_id(&*tx, folder_a).await.unwrap();
        assert_eq!(deleted, 2);

        // folder_b's location still there
        let still_b = svc
            .message_location_repository()
            .find_by_message_and_folder(&*tx, msg1, folder_b)
            .await
            .unwrap();
        assert!(still_b.is_some());
        let gone_a = svc
            .message_location_repository()
            .find_by_message_and_folder(&*tx, msg1, folder_a)
            .await
            .unwrap();
        assert!(gone_a.is_none());
    }

    #[tokio::test]
    async fn filter_message_ids_in_folders_returns_distinct_subset() {
        let svc = setup().await;
        let user_id = make_user(&svc, "alice", "alice@example.com").await;
        let account_id = make_account(&svc, user_id, "example.com").await;
        let folder_a = make_folder(&svc, account_id, "INBOX").await;
        let folder_b = make_folder(&svc, account_id, "Archive").await;
        let m1 = make_message(&svc, account_id, "<m1@x.com>").await;
        let m2 = make_message(&svc, account_id, "<m2@x.com>").await;
        let m3 = make_message(&svc, account_id, "<m3@x.com>").await;
        let tx = svc.repository().begin().await.unwrap();

        // m1 -> A, m2 -> B, m3 -> A and B.
        svc.message_location_repository()
            .upsert(&*tx, loc_row(m1, folder_a, 1, MessageFlags::default()))
            .await
            .unwrap();
        svc.message_location_repository()
            .upsert(&*tx, loc_row(m2, folder_b, 2, MessageFlags::default()))
            .await
            .unwrap();
        svc.message_location_repository()
            .upsert(&*tx, loc_row(m3, folder_a, 3, MessageFlags::default()))
            .await
            .unwrap();
        svc.message_location_repository()
            .upsert(&*tx, loc_row(m3, folder_b, 4, MessageFlags::default()))
            .await
            .unwrap();

        let repo = svc.message_location_repository();

        // Restricting to folder A yields the messages present in A.
        let mut in_a = repo.filter_message_ids_in_folders(&*tx, &[m1, m2, m3], &[folder_a]).await.unwrap();
        in_a.sort_unstable();
        assert_eq!(in_a, {
            let mut v = vec![m1, m3];
            v.sort_unstable();
            v
        });

        // Across both folders every message qualifies; m3 is de-duplicated.
        let mut in_both = repo.filter_message_ids_in_folders(&*tx, &[m1, m2, m3], &[folder_a, folder_b]).await.unwrap();
        in_both.sort_unstable();
        assert_eq!(in_both, {
            let mut v = vec![m1, m2, m3];
            v.sort_unstable();
            v
        });

        // A message not located in the queried folder is excluded.
        let none = repo.filter_message_ids_in_folders(&*tx, &[m2], &[folder_a]).await.unwrap();
        assert!(none.is_empty());

        // Empty inputs short-circuit to an empty vec.
        assert!(repo.filter_message_ids_in_folders(&*tx, &[], &[folder_a]).await.unwrap().is_empty());
        assert!(repo.filter_message_ids_in_folders(&*tx, &[m1], &[]).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn cascade_on_message_delete_removes_locations() {
        let svc = setup().await;
        let user_id = make_user(&svc, "alice", "alice@example.com").await;
        let account_id = make_account(&svc, user_id, "example.com").await;
        let folder_id = make_folder(&svc, account_id, "INBOX").await;
        let message_id = make_message(&svc, account_id, "<m1@x.com>").await;
        let tx = svc.repository().begin().await.unwrap();

        svc.message_location_repository()
            .upsert(&*tx, loc_row(message_id, folder_id, 1, MessageFlags::default()))
            .await
            .unwrap();

        // Delete message via raw SeaORM (no service helper yet).
        use sea_orm::EntityTrait;
        let db_tx = crate::transaction::TransactionImpl::get_db_transaction(&*tx).unwrap();
        crate::entities::prelude::Messages::delete_by_id(message_id as i64).exec(db_tx).await.unwrap();

        let after = svc
            .message_location_repository()
            .find_by_message_and_folder(&*tx, message_id, folder_id)
            .await
            .unwrap();
        assert!(after.is_none(), "FK cascade should drop the location");
    }

    #[tokio::test]
    async fn cascade_on_folder_delete_removes_locations() {
        let svc = setup().await;
        let user_id = make_user(&svc, "alice", "alice@example.com").await;
        let account_id = make_account(&svc, user_id, "example.com").await;
        let folder_id = make_folder(&svc, account_id, "INBOX").await;
        let message_id = make_message(&svc, account_id, "<m1@x.com>").await;
        let tx = svc.repository().begin().await.unwrap();

        svc.message_location_repository()
            .upsert(&*tx, loc_row(message_id, folder_id, 1, MessageFlags::default()))
            .await
            .unwrap();

        svc.folder_repository().delete_by_id(&*tx, folder_id).await.unwrap();

        let after = svc
            .message_location_repository()
            .find_by_message_and_folder(&*tx, message_id, folder_id)
            .await
            .unwrap();
        assert!(after.is_none(), "FK cascade should drop the location when folder deleted");
    }
}
