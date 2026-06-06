use std::sync::Arc;

use chrono::{DateTime, Utc};

use crate::{
    Error,
    account::AccountId,
    folder::model::{Folder, FolderId, NewFolderRequest},
    repository::RepositoryService,
};

#[async_trait::async_trait]
pub trait FolderService: Send + Sync {
    async fn create_folders_for_account(&self, account_id: AccountId, requests: Vec<NewFolderRequest>) -> Result<Vec<Folder>, Error>;

    async fn list_folders(&self, account_id: AccountId) -> Result<Vec<Folder>, Error>;

    async fn list_enabled_folders(&self, account_id: AccountId) -> Result<Vec<Folder>, Error>;

    async fn set_enabled(&self, folder_id: FolderId, enabled: bool) -> Result<(), Error>;

    async fn set_idle_enabled(&self, folder_id: FolderId, idle_enabled: bool) -> Result<(), Error>;

    async fn record_sync_progress(&self, folder_id: FolderId, uidvalidity: u32, last_uid: u32, last_synced_at: DateTime<Utc>) -> Result<(), Error>;

    async fn delete_folder(&self, folder_id: FolderId) -> Result<(), Error>;
}

pub(crate) struct FolderServiceImpl {
    #[allow(dead_code, reason = "wired in CoreServices; Task 2 uses it from method bodies")]
    repository_service: Arc<RepositoryService>,
}

impl FolderServiceImpl {
    #[must_use]
    pub(crate) fn new(repository_service: Arc<RepositoryService>) -> Self {
        Self { repository_service }
    }
}

#[async_trait::async_trait]
impl FolderService for FolderServiceImpl {
    async fn create_folders_for_account(&self, _account_id: AccountId, _requests: Vec<NewFolderRequest>) -> Result<Vec<Folder>, Error> {
        unimplemented!("Task 2 implements FolderService methods")
    }

    async fn list_folders(&self, _account_id: AccountId) -> Result<Vec<Folder>, Error> {
        unimplemented!("Task 2 implements FolderService methods")
    }

    async fn list_enabled_folders(&self, _account_id: AccountId) -> Result<Vec<Folder>, Error> {
        unimplemented!("Task 2 implements FolderService methods")
    }

    async fn set_enabled(&self, _folder_id: FolderId, _enabled: bool) -> Result<(), Error> {
        unimplemented!("Task 2 implements FolderService methods")
    }

    async fn set_idle_enabled(&self, _folder_id: FolderId, _idle_enabled: bool) -> Result<(), Error> {
        unimplemented!("Task 2 implements FolderService methods")
    }

    async fn record_sync_progress(&self, _folder_id: FolderId, _uidvalidity: u32, _last_uid: u32, _last_synced_at: DateTime<Utc>) -> Result<(), Error> {
        unimplemented!("Task 2 implements FolderService methods")
    }

    async fn delete_folder(&self, _folder_id: FolderId) -> Result<(), Error> {
        unimplemented!("Task 2 implements FolderService methods")
    }
}
