use chrono::{DateTime, Utc};

use crate::{
    Error,
    account::AccountId,
    folder::FolderId,
    message::{Message, MessageAttachment, MessageAttachmentToken, MessageFlags, MessageId, MessageLocation, MessageLocationToken, MessageToken, NamedAddress},
    repository::Transaction,
    types::{ContentHash, EmailAddress},
};

#[derive(Debug, Clone)]
pub struct NewMessageRow {
    pub token: MessageToken,
    pub account_id: AccountId,
    pub rfc822_message_id: String,
    pub content_hash: ContentHash,
    pub subject: Option<String>,
    pub from_address: EmailAddress,
    pub from_name: Option<String>,
    pub to_addresses: Vec<NamedAddress>,
    pub cc_addresses: Vec<NamedAddress>,
    pub bcc_addresses: Vec<NamedAddress>,
    pub reply_to_addresses: Vec<NamedAddress>,
    pub sent_date: Option<DateTime<Utc>>,
    pub in_reply_to: Option<String>,
    pub references: Vec<String>,
    pub snippet: String,
    pub size_bytes: i64,
    pub has_attachments: bool,
    pub attachment_count: i32,
}

#[derive(Debug, Clone)]
pub struct NewMessageLocationRow {
    pub token: MessageLocationToken,
    pub message_id: MessageId,
    pub folder_id: FolderId,
    pub uid: u32,
    pub uidvalidity: u32,
    pub flags: MessageFlags,
    pub internal_date: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct NewMessageAttachmentRow {
    pub token: MessageAttachmentToken,
    pub message_id: MessageId,
    pub account_id: AccountId,
    pub content_hash: ContentHash,
    pub filename: Option<String>,
    pub content_type: String,
    pub size_bytes: i64,
    pub is_inline: bool,
    pub content_id: Option<String>,
}

#[async_trait::async_trait]
#[cfg_attr(test, mockall::automock)]
pub trait MessageRepository: Send + Sync {
    /// Look up a message by its raw-content hash, the archiver's identity key.
    /// Identical bytes are the same archived message regardless of Message-ID.
    async fn find_by_account_and_content_hash(
        &self,
        transaction: &dyn Transaction,
        account_id: AccountId,
        content_hash: ContentHash,
    ) -> Result<Option<Message>, Error>;

    async fn create(&self, transaction: &dyn Transaction, new: NewMessageRow) -> Result<Message, Error>;

    async fn find_by_id_for_account(&self, transaction: &dyn Transaction, account_id: AccountId, message_id: MessageId) -> Result<Option<Message>, Error>;

    async fn list_for_account(&self, transaction: &dyn Transaction, account_id: AccountId, limit: u32, offset: u32) -> Result<Vec<Message>, Error>;
}

#[async_trait::async_trait]
#[cfg_attr(test, mockall::automock)]
pub trait MessageLocationRepository: Send + Sync {
    async fn find_by_message_and_folder(
        &self,
        transaction: &dyn Transaction,
        message_id: MessageId,
        folder_id: FolderId,
    ) -> Result<Option<MessageLocation>, Error>;

    async fn upsert(&self, transaction: &dyn Transaction, new: NewMessageLocationRow) -> Result<MessageLocation, Error>;

    /// Returns the number of rows deleted.
    async fn delete_by_folder_id(&self, transaction: &dyn Transaction, folder_id: FolderId) -> Result<u64, Error>;
}

#[async_trait::async_trait]
#[cfg_attr(test, mockall::automock)]
pub trait MessageAttachmentRepository: Send + Sync {
    async fn create_many(&self, transaction: &dyn Transaction, rows: Vec<NewMessageAttachmentRow>) -> Result<Vec<MessageAttachment>, Error>;

    async fn list_for_message(&self, transaction: &dyn Transaction, message_id: MessageId) -> Result<Vec<MessageAttachment>, Error>;
}
