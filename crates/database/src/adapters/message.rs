use chrono::Utc;
use mk_core::{
    Error,
    account::AccountId,
    message::{Message, MessageId, MessageRepository, MessageToken, NewMessageRow},
    repository::Transaction,
    types::{ContentHash, EmailAddress},
};
use sea_orm::{ActiveModelTrait, ActiveValue::Set, ColumnTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect};

use crate::{
    entities::{messages, prelude},
    error::handle_dberr,
    transaction::TransactionImpl,
};

impl From<messages::Model> for Message {
    fn from(model: messages::Model) -> Self {
        let token = MessageToken::new(model.id as u64);
        let content_hash = ContentHash::from_hex(&model.content_hash).expect("database content_hash should be 64-char hex");
        let from_address = EmailAddress::new(model.from_address).expect("database from_address should be valid");
        Self {
            id: model.id as u64,
            version: model.version as u64,
            token,
            account_id: model.account_id as u64,
            rfc822_message_id: model.rfc822_message_id,
            content_hash,
            subject: model.subject,
            from_address,
            from_name: model.from_name,
            to_addresses: serde_json::from_value(model.to_addresses).expect("database to_addresses JSON should be valid"),
            cc_addresses: serde_json::from_value(model.cc_addresses).expect("database cc_addresses JSON should be valid"),
            bcc_addresses: serde_json::from_value(model.bcc_addresses).expect("database bcc_addresses JSON should be valid"),
            reply_to_addresses: serde_json::from_value(model.reply_to_addresses).expect("database reply_to_addresses JSON should be valid"),
            sent_date: model.sent_date.map(|d| d.with_timezone(&Utc)),
            in_reply_to: model.in_reply_to,
            references: serde_json::from_value(model.references).expect("database references JSON should be valid"),
            snippet: model.snippet,
            size_bytes: model.size_bytes,
            has_attachments: model.has_attachments,
            attachment_count: model.attachment_count,
            created_at: model.created_at.with_timezone(&Utc),
            updated_at: model.updated_at.with_timezone(&Utc),
        }
    }
}

pub(crate) struct MessageRepositoryAdapter;

impl MessageRepositoryAdapter {
    pub(crate) fn new() -> Self {
        Self
    }
}

#[async_trait::async_trait]
impl MessageRepository for MessageRepositoryAdapter {
    async fn find_by_account_and_content_hash(
        &self,
        transaction: &dyn Transaction,
        account_id: AccountId,
        content_hash: ContentHash,
    ) -> Result<Option<Message>, Error> {
        if account_id == 0 {
            return Err(Error::InvalidId(account_id));
        }
        let transaction = TransactionImpl::get_db_transaction(transaction)?;
        Ok(prelude::Messages::find()
            .filter(messages::Column::AccountId.eq(account_id as i64))
            .filter(messages::Column::ContentHash.eq(content_hash.as_hex()))
            .one(transaction)
            .await
            .map_err(handle_dberr)?
            .map(Into::into))
    }

    async fn create(&self, transaction: &dyn Transaction, new: NewMessageRow) -> Result<Message, Error> {
        if new.account_id == 0 {
            return Err(Error::InvalidId(new.account_id));
        }
        let transaction = TransactionImpl::get_db_transaction(transaction)?;
        let now = Utc::now();

        let to_addresses = serde_json::to_value(&new.to_addresses).map_err(|e| Error::Infrastructure(e.to_string()))?;
        let cc_addresses = serde_json::to_value(&new.cc_addresses).map_err(|e| Error::Infrastructure(e.to_string()))?;
        let bcc_addresses = serde_json::to_value(&new.bcc_addresses).map_err(|e| Error::Infrastructure(e.to_string()))?;
        let reply_to_addresses = serde_json::to_value(&new.reply_to_addresses).map_err(|e| Error::Infrastructure(e.to_string()))?;
        let references = serde_json::to_value(&new.references).map_err(|e| Error::Infrastructure(e.to_string()))?;

        let model = messages::ActiveModel {
            id: Set(new.token.id() as i64),
            version: Set(0),
            token: Set(new.token.to_string()),
            account_id: Set(new.account_id as i64),
            rfc822_message_id: Set(new.rfc822_message_id),
            content_hash: Set(new.content_hash.as_hex()),
            subject: Set(new.subject),
            from_address: Set(new.from_address.into_inner()),
            from_name: Set(new.from_name),
            to_addresses: Set(to_addresses),
            cc_addresses: Set(cc_addresses),
            bcc_addresses: Set(bcc_addresses),
            reply_to_addresses: Set(reply_to_addresses),
            sent_date: Set(new.sent_date.map(Into::into)),
            in_reply_to: Set(new.in_reply_to),
            references: Set(references),
            snippet: Set(new.snippet),
            size_bytes: Set(new.size_bytes),
            has_attachments: Set(new.has_attachments),
            attachment_count: Set(new.attachment_count),
            created_at: Set(now.into()),
            updated_at: Set(now.into()),
        };
        let saved = model.insert(transaction).await.map_err(handle_dberr)?;
        Ok(saved.into())
    }

    async fn find_by_id_for_account(&self, transaction: &dyn Transaction, account_id: AccountId, message_id: MessageId) -> Result<Option<Message>, Error> {
        if account_id == 0 {
            return Err(Error::InvalidId(account_id));
        }
        if message_id == 0 {
            return Err(Error::InvalidId(message_id));
        }
        let transaction = TransactionImpl::get_db_transaction(transaction)?;
        Ok(prelude::Messages::find_by_id(message_id as i64)
            .filter(messages::Column::AccountId.eq(account_id as i64))
            .one(transaction)
            .await
            .map_err(handle_dberr)?
            .map(Into::into))
    }

    async fn list_for_account(&self, transaction: &dyn Transaction, account_id: AccountId, limit: u32, offset: u32) -> Result<Vec<Message>, Error> {
        if account_id == 0 {
            return Err(Error::InvalidId(account_id));
        }
        let transaction = TransactionImpl::get_db_transaction(transaction)?;
        let rows = prelude::Messages::find()
            .filter(messages::Column::AccountId.eq(account_id as i64))
            .order_by_asc(messages::Column::Id)
            .limit(u64::from(limit))
            .offset(u64::from(offset))
            .all(transaction)
            .await
            .map_err(handle_dberr)?;
        Ok(rows.into_iter().map(Into::into).collect())
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::HashSet, sync::Arc};

    use mk_core::{
        Error, RepositoryError,
        account::{AccountToken, NewAccount},
        crypto::Ciphertext,
        imap::{ImapServerConfig, TlsMode},
        message::{MessageToken, NamedAddress, NewMessageRow},
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

    fn new_row(account_id: u64, rfc822_id: &str) -> NewMessageRow {
        NewMessageRow {
            token: MessageToken::generate(),
            account_id,
            rfc822_message_id: rfc822_id.to_string(),
            // Derive the hash from the (unique) id so distinct rows get distinct
            // content hashes — identity is now (account_id, content_hash).
            content_hash: ContentHash::compute(rfc822_id.as_bytes()),
            subject: Some("Hello".into()),
            from_address: EmailAddress::new("alice@example.com").unwrap(),
            from_name: Some("Alice".into()),
            to_addresses: vec![NamedAddress {
                address: EmailAddress::new("bob@example.com").unwrap(),
                name: Some("Bob".into()),
            }],
            cc_addresses: vec![],
            bcc_addresses: vec![],
            reply_to_addresses: vec![],
            sent_date: None,
            in_reply_to: None,
            references: vec!["<a@x.com>".into(), "<b@x.com>".into()],
            snippet: "preview".into(),
            size_bytes: 1024,
            has_attachments: false,
            attachment_count: 0,
        }
    }

    #[tokio::test]
    async fn create_and_find_by_account_and_content_hash() {
        let svc = setup().await;
        let user_id = make_user(&svc, "alice", "alice@example.com").await;
        let account_id = make_account(&svc, user_id, "example.com").await;
        let tx = svc.repository().begin().await.unwrap();

        let row = new_row(account_id, "<msg-1@example.com>");
        let inserted = svc.message_repository().create(&*tx, row).await.unwrap();
        assert_eq!(inserted.account_id, account_id);
        assert_eq!(inserted.rfc822_message_id, "<msg-1@example.com>");
        assert_eq!(inserted.to_addresses.len(), 1);
        assert_eq!(inserted.to_addresses[0].name.as_deref(), Some("Bob"));
        assert_eq!(inserted.references, vec!["<a@x.com>".to_string(), "<b@x.com>".to_string()]);
        assert_eq!(inserted.version, 1);

        let found = svc
            .message_repository()
            .find_by_account_and_content_hash(&*tx, account_id, ContentHash::compute(b"<msg-1@example.com>"))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(found.id, inserted.id);
        assert_eq!(found.snippet, "preview");
    }

    #[tokio::test]
    async fn unique_account_content_hash_constraint() {
        let svc = setup().await;
        let user_id = make_user(&svc, "alice", "alice@example.com").await;
        let account_id = make_account(&svc, user_id, "example.com").await;
        let tx = svc.repository().begin().await.unwrap();

        // Two rows with identical content (new_row derives the hash from the id).
        let row = new_row(account_id, "<dup@example.com>");
        svc.message_repository().create(&*tx, row).await.unwrap();

        let row2 = new_row(account_id, "<dup@example.com>");
        let result = svc.message_repository().create(&*tx, row2).await;
        assert!(matches!(result, Err(Error::RepositoryError(RepositoryError::Constraint(_)))));
    }

    #[tokio::test]
    async fn list_for_account_returns_paginated() {
        let svc = setup().await;
        let user_id = make_user(&svc, "alice", "alice@example.com").await;
        let account_id = make_account(&svc, user_id, "example.com").await;
        let tx = svc.repository().begin().await.unwrap();

        let mut inserted_ids = Vec::new();
        for i in 0..3 {
            let row = new_row(account_id, &format!("<msg-{i}@example.com>"));
            let m = svc.message_repository().create(&*tx, row).await.unwrap();
            inserted_ids.push(m.id);
        }
        // Ensure stable id ordering for the test
        inserted_ids.sort_unstable();

        let page = svc.message_repository().list_for_account(&*tx, account_id, 2, 1).await.unwrap();
        assert_eq!(page.len(), 2);
        assert_eq!(page[0].id, inserted_ids[1]);
        assert_eq!(page[1].id, inserted_ids[2]);
    }

    #[tokio::test]
    async fn cascade_on_account_delete_removes_messages() {
        let svc = setup().await;
        let user_id = make_user(&svc, "alice", "alice@example.com").await;
        let account_id = make_account(&svc, user_id, "example.com").await;
        let tx = svc.repository().begin().await.unwrap();

        let row = new_row(account_id, "<msg@example.com>");
        let inserted = svc.message_repository().create(&*tx, row).await.unwrap();

        let account = svc.account_repository().find_by_id_for_user(&*tx, user_id, account_id).await.unwrap().unwrap();
        svc.account_repository().delete(&*tx, account).await.unwrap();

        let after = svc.message_repository().find_by_id_for_account(&*tx, account_id, inserted.id).await.unwrap();
        assert!(after.is_none(), "FK cascade should have deleted the message row");
    }
}
