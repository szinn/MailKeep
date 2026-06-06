use std::sync::Arc;

use chrono::{DateTime, Utc};

use crate::{
    Error,
    account::AccountId,
    folder::FolderId,
    message::{Message, MessageFlags, MessageId, ParsedMessage, RecordedMessage},
    repository::RepositoryService,
};

#[async_trait::async_trait]
pub trait MessageService: Send + Sync {
    async fn record_parsed_message(
        &self,
        account_id: AccountId,
        folder_id: FolderId,
        uid: u32,
        uidvalidity: u32,
        internal_date: DateTime<Utc>,
        flags: MessageFlags,
        parsed: ParsedMessage,
    ) -> Result<RecordedMessage, Error>;

    async fn delete_locations_for_folder(&self, folder_id: FolderId) -> Result<u64, Error>;

    async fn get_message_for_account(&self, account_id: AccountId, message_id: MessageId) -> Result<Option<Message>, Error>;

    async fn list_messages_for_account(&self, account_id: AccountId, limit: u32, offset: u32) -> Result<Vec<Message>, Error>;
}

pub(crate) struct MessageServiceImpl {
    #[allow(dead_code, reason = "Task 4 wires this into transactional method bodies")]
    repository_service: Arc<RepositoryService>,
}

impl MessageServiceImpl {
    #[must_use]
    pub(crate) fn new(repository_service: Arc<RepositoryService>) -> Self {
        Self { repository_service }
    }
}

#[async_trait::async_trait]
impl MessageService for MessageServiceImpl {
    async fn record_parsed_message(
        &self,
        _account_id: AccountId,
        _folder_id: FolderId,
        _uid: u32,
        _uidvalidity: u32,
        _internal_date: DateTime<Utc>,
        _flags: MessageFlags,
        _parsed: ParsedMessage,
    ) -> Result<RecordedMessage, Error> {
        unimplemented!("Task 4")
    }

    async fn delete_locations_for_folder(&self, _folder_id: FolderId) -> Result<u64, Error> {
        unimplemented!("Task 4")
    }

    async fn get_message_for_account(&self, _account_id: AccountId, _message_id: MessageId) -> Result<Option<Message>, Error> {
        unimplemented!("Task 4")
    }

    async fn list_messages_for_account(&self, _account_id: AccountId, _limit: u32, _offset: u32) -> Result<Vec<Message>, Error> {
        unimplemented!("Task 4")
    }
}
