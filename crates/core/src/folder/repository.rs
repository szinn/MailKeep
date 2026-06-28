use std::collections::HashMap;

use chrono::{DateTime, Utc};

use crate::{
    Error,
    account::AccountId,
    folder::model::{Folder, FolderId, FolderToken, SpecialUse},
    repository::Transaction,
};

#[derive(Debug, Clone)]
pub struct NewFolderRow {
    pub token: FolderToken,
    pub account_id: AccountId,
    pub path: String,
    pub display_name: Option<String>,
    pub special_use: Option<SpecialUse>,
    pub idle_enabled: bool,
    pub uidvalidity: Option<u32>,
}

#[async_trait::async_trait]
#[cfg_attr(test, mockall::automock)]
pub trait FolderRepository: Send + Sync {
    async fn create_many(&self, transaction: &dyn Transaction, account_id: AccountId, folders: Vec<NewFolderRow>) -> Result<Vec<Folder>, Error>;

    async fn find_by_id_for_account(&self, transaction: &dyn Transaction, account_id: AccountId, folder_id: FolderId) -> Result<Option<Folder>, Error>;

    async fn find_by_account_and_path(&self, transaction: &dyn Transaction, account_id: AccountId, path: &str) -> Result<Option<Folder>, Error>;

    async fn list_for_account(&self, transaction: &dyn Transaction, account_id: AccountId) -> Result<Vec<Folder>, Error>;

    async fn list_enabled_for_account(&self, transaction: &dyn Transaction, account_id: AccountId) -> Result<Vec<Folder>, Error>;

    /// Returns `account_id -> max(last_synced_at)` for the given accounts, in a
    /// single grouped query. Accounts with no folders or no synced folders are
    /// absent from the map. An empty `account_ids` returns an empty map.
    async fn max_last_synced_by_account(&self, transaction: &dyn Transaction, account_ids: &[AccountId]) -> Result<HashMap<AccountId, DateTime<Utc>>, Error>;

    async fn update_enabled(&self, transaction: &dyn Transaction, folder_id: FolderId, enabled: bool) -> Result<(), Error>;

    async fn update_idle_enabled(&self, transaction: &dyn Transaction, folder_id: FolderId, idle_enabled: bool) -> Result<(), Error>;

    async fn update_sync_state(
        &self,
        transaction: &dyn Transaction,
        folder_id: FolderId,
        uidvalidity: u32,
        last_uid: u32,
        last_synced_at: DateTime<Utc>,
    ) -> Result<(), Error>;

    async fn delete_by_id(&self, transaction: &dyn Transaction, folder_id: FolderId) -> Result<(), Error>;
}
