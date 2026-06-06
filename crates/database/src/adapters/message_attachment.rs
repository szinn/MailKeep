use chrono::Utc;
use mk_core::{
    Error,
    message::{MessageAttachment, MessageAttachmentRepository, MessageAttachmentToken, MessageId, NewMessageAttachmentRow},
    repository::Transaction,
    types::ContentHash,
};
use sea_orm::{ActiveModelTrait, ActiveValue::Set, ColumnTrait, EntityTrait, QueryFilter};

use crate::{
    entities::{message_attachments, prelude},
    error::handle_dberr,
    transaction::TransactionImpl,
};

impl From<message_attachments::Model> for MessageAttachment {
    fn from(model: message_attachments::Model) -> Self {
        let token = MessageAttachmentToken::new(model.id as u64);
        let content_hash = ContentHash::from_hex(&model.content_hash).expect("database content_hash should be 64-char hex");
        Self {
            id: model.id as u64,
            version: model.version as u64,
            token,
            message_id: model.message_id as u64,
            account_id: model.account_id as u64,
            content_hash,
            filename: model.filename,
            content_type: model.content_type,
            size_bytes: model.size_bytes,
            is_inline: model.is_inline,
            content_id: model.content_id,
            created_at: model.created_at.with_timezone(&Utc),
            updated_at: model.updated_at.with_timezone(&Utc),
        }
    }
}

pub(crate) struct MessageAttachmentRepositoryAdapter;

impl MessageAttachmentRepositoryAdapter {
    pub(crate) fn new() -> Self {
        Self
    }
}

#[async_trait::async_trait]
impl MessageAttachmentRepository for MessageAttachmentRepositoryAdapter {
    async fn create_many(&self, transaction: &dyn Transaction, rows: Vec<NewMessageAttachmentRow>) -> Result<Vec<MessageAttachment>, Error> {
        let transaction = TransactionImpl::get_db_transaction(transaction)?;
        let now = Utc::now();

        let mut inserted = Vec::with_capacity(rows.len());
        for row in rows {
            if row.message_id == 0 {
                return Err(Error::InvalidId(row.message_id));
            }
            if row.account_id == 0 {
                return Err(Error::InvalidId(row.account_id));
            }
            let model = message_attachments::ActiveModel {
                id: Set(row.token.id() as i64),
                version: Set(0),
                token: Set(row.token.to_string()),
                message_id: Set(row.message_id as i64),
                account_id: Set(row.account_id as i64),
                content_hash: Set(row.content_hash.as_hex()),
                filename: Set(row.filename),
                content_type: Set(row.content_type),
                size_bytes: Set(row.size_bytes),
                is_inline: Set(row.is_inline),
                content_id: Set(row.content_id),
                created_at: Set(now.into()),
                updated_at: Set(now.into()),
            };
            let saved = model.insert(transaction).await.map_err(handle_dberr)?;
            inserted.push(saved.into());
        }
        Ok(inserted)
    }

    async fn list_for_message(&self, transaction: &dyn Transaction, message_id: MessageId) -> Result<Vec<MessageAttachment>, Error> {
        if message_id == 0 {
            return Err(Error::InvalidId(message_id));
        }
        let transaction = TransactionImpl::get_db_transaction(transaction)?;
        let rows = prelude::MessageAttachments::find()
            .filter(message_attachments::Column::MessageId.eq(message_id as i64))
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
        account::{AccountToken, NewAccount},
        crypto::Ciphertext,
        imap::{ImapServerConfig, TlsMode},
        message::{MessageAttachmentToken, MessageToken, NewMessageAttachmentRow, NewMessageRow},
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

    async fn make_message(svc: &Arc<RepositoryService>, account_id: u64, rfc_id: &str) -> u64 {
        let tx = svc.repository().begin().await.unwrap();
        let row = NewMessageRow {
            token: MessageToken::generate(),
            account_id,
            rfc822_message_id: rfc_id.to_string(),
            content_hash: ContentHash::compute(b"x"),
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
            has_attachments: true,
            attachment_count: 1,
        };
        let msg = svc.message_repository().create(&*tx, row).await.unwrap();
        tx.commit().await.unwrap();
        msg.id
    }

    fn att_row(message_id: u64, account_id: u64, filename: &str) -> NewMessageAttachmentRow {
        NewMessageAttachmentRow {
            token: MessageAttachmentToken::generate(),
            message_id,
            account_id,
            content_hash: ContentHash::compute(b"attach"),
            filename: Some(filename.into()),
            content_type: "application/pdf".into(),
            size_bytes: 2048,
            is_inline: false,
            content_id: Some("cid-1".into()),
        }
    }

    #[tokio::test]
    async fn create_many_round_trip() {
        let svc = setup().await;
        let user_id = make_user(&svc, "alice", "alice@example.com").await;
        let account_id = make_account(&svc, user_id, "example.com").await;
        let message_id = make_message(&svc, account_id, "<m@x.com>").await;
        let tx = svc.repository().begin().await.unwrap();

        let rows = vec![att_row(message_id, account_id, "report.pdf"), att_row(message_id, account_id, "summary.pdf")];
        let inserted = svc.message_attachment_repository().create_many(&*tx, rows).await.unwrap();
        assert_eq!(inserted.len(), 2);
        assert!(inserted.iter().all(|a| a.message_id == message_id));
        assert!(inserted.iter().all(|a| a.account_id == account_id));
        assert!(inserted.iter().all(|a| a.content_type == "application/pdf"));

        let listed = svc.message_attachment_repository().list_for_message(&*tx, message_id).await.unwrap();
        assert_eq!(listed.len(), 2);
    }

    #[tokio::test]
    async fn list_for_message_returns_empty_for_unknown_message() {
        let svc = setup().await;
        let tx = svc.repository().begin().await.unwrap();
        let rows = svc.message_attachment_repository().list_for_message(&*tx, 9_999_999).await.unwrap();
        assert!(rows.is_empty());
    }

    #[tokio::test]
    async fn cascade_on_message_delete_removes_attachments() {
        let svc = setup().await;
        let user_id = make_user(&svc, "alice", "alice@example.com").await;
        let account_id = make_account(&svc, user_id, "example.com").await;
        let message_id = make_message(&svc, account_id, "<m@x.com>").await;
        let tx = svc.repository().begin().await.unwrap();

        svc.message_attachment_repository()
            .create_many(&*tx, vec![att_row(message_id, account_id, "file.pdf")])
            .await
            .unwrap();

        use sea_orm::EntityTrait;
        let db_tx = crate::transaction::TransactionImpl::get_db_transaction(&*tx).unwrap();
        crate::entities::prelude::Messages::delete_by_id(message_id as i64).exec(db_tx).await.unwrap();

        let after = svc.message_attachment_repository().list_for_message(&*tx, message_id).await.unwrap();
        assert!(after.is_empty(), "FK cascade should drop the attachments");
    }
}
