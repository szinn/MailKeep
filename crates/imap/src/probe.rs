//! Connectivity-probe support for the `mailkeep imap` diagnostic command.
//!
//! `test_connection`/`list_folders` need no sync services, but [`ImapAdapter`]
//! now requires the injected `IngestService`/`FolderService`/`MessageService`
//! for the sync lifecycle. The probe path supplies nop services that panic if a
//! sync method is ever reached — they exist only so the probe-only adapter can
//! be constructed. They are never wired into production sync.

use std::{collections::HashMap, sync::Arc};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use mk_core::{
    Error,
    account::AccountId,
    folder::{Folder, FolderId, FolderService, NewFolderRequest},
    ingest::{IngestRequest, IngestResult, IngestService},
    message::{Message, MessageAttachment, MessageFlags, MessageId, MessageService, MessageToken, ParsedMessage, RecordedMessage},
};

const PANIC_MSG: &str = "probe-only adapter: sync services are not available";

pub(crate) struct NopIngestService;

#[async_trait]
impl IngestService for NopIngestService {
    async fn ingest_raw(&self, _request: IngestRequest) -> Result<IngestResult, Error> {
        unimplemented!("{PANIC_MSG}")
    }
}

pub(crate) struct NopFolderService;

#[async_trait]
impl FolderService for NopFolderService {
    async fn create_folders_for_account(&self, _account_id: AccountId, _requests: Vec<NewFolderRequest>) -> Result<Vec<Folder>, Error> {
        unimplemented!("{PANIC_MSG}")
    }

    async fn list_folders(&self, _account_id: AccountId) -> Result<Vec<Folder>, Error> {
        unimplemented!("{PANIC_MSG}")
    }

    async fn list_enabled_folders(&self, _account_id: AccountId) -> Result<Vec<Folder>, Error> {
        unimplemented!("{PANIC_MSG}")
    }

    async fn set_enabled(&self, _folder_id: FolderId, _enabled: bool) -> Result<(), Error> {
        unimplemented!("{PANIC_MSG}")
    }

    async fn set_idle_enabled(&self, _folder_id: FolderId, _idle_enabled: bool) -> Result<(), Error> {
        unimplemented!("{PANIC_MSG}")
    }

    async fn record_sync_progress(&self, _folder_id: FolderId, _uidvalidity: u32, _last_uid: u32, _last_synced_at: DateTime<Utc>) -> Result<(), Error> {
        unimplemented!("{PANIC_MSG}")
    }

    async fn delete_folder(&self, _folder_id: FolderId) -> Result<(), Error> {
        unimplemented!("{PANIC_MSG}")
    }

    async fn last_synced_by_account(&self, _account_ids: &[AccountId]) -> Result<HashMap<AccountId, DateTime<Utc>>, Error> {
        unimplemented!("{PANIC_MSG}")
    }
}

pub(crate) struct NopMessageService;

#[async_trait]
impl MessageService for NopMessageService {
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
        unimplemented!("{PANIC_MSG}")
    }

    async fn delete_locations_for_folder(&self, _folder_id: FolderId) -> Result<u64, Error> {
        unimplemented!("{PANIC_MSG}")
    }

    async fn get_message_for_account(&self, _account_id: AccountId, _message_id: MessageId) -> Result<Option<Message>, Error> {
        unimplemented!("{PANIC_MSG}")
    }

    async fn list_messages_for_account(&self, _account_id: AccountId, _limit: u32, _offset: u32) -> Result<Vec<Message>, Error> {
        unimplemented!("{PANIC_MSG}")
    }

    async fn get_messages_by_ids(&self, _user_id: mk_core::user::UserId, _ids: &[MessageId]) -> Result<Vec<Message>, Error> {
        unimplemented!("{PANIC_MSG}")
    }

    async fn get_message_with_attachments(
        &self,
        _user_id: mk_core::user::UserId,
        _token: MessageToken,
    ) -> Result<Option<(Message, Vec<MessageAttachment>)>, Error> {
        unimplemented!("{PANIC_MSG}")
    }
}

/// The nop service trio used to construct a probe-only [`ImapAdapter`].
#[must_use]
pub(crate) fn nop_services() -> (Arc<dyn IngestService>, Arc<dyn FolderService>, Arc<dyn MessageService>) {
    (Arc::new(NopIngestService), Arc::new(NopFolderService), Arc::new(NopMessageService))
}
