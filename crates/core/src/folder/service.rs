use std::sync::Arc;

use chrono::{DateTime, Utc};

use crate::{
    Error,
    account::AccountId,
    folder::{
        model::{Folder, FolderId, FolderToken, NewFolderRequest, SpecialUse},
        repository::NewFolderRow,
    },
    repository::RepositoryService,
    with_read_only_transaction, with_transaction,
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
    async fn create_folders_for_account(&self, account_id: AccountId, requests: Vec<NewFolderRequest>) -> Result<Vec<Folder>, Error> {
        let rows: Vec<NewFolderRow> = requests
            .into_iter()
            .map(|req| NewFolderRow {
                token: FolderToken::generate(),
                account_id,
                path: req.path,
                display_name: req.display_name,
                special_use: req.special_use,
                idle_enabled: req.special_use == Some(SpecialUse::Inbox),
                uidvalidity: req.uidvalidity,
            })
            .collect();

        with_transaction!(self, folder_repository, |tx| folder_repository.create_many(tx, account_id, rows).await)
    }

    async fn list_folders(&self, account_id: AccountId) -> Result<Vec<Folder>, Error> {
        with_read_only_transaction!(self, folder_repository, |tx| folder_repository.list_for_account(tx, account_id).await)
    }

    async fn list_enabled_folders(&self, account_id: AccountId) -> Result<Vec<Folder>, Error> {
        with_read_only_transaction!(self, folder_repository, |tx| folder_repository.list_enabled_for_account(tx, account_id).await)
    }

    async fn set_enabled(&self, folder_id: FolderId, enabled: bool) -> Result<(), Error> {
        with_transaction!(self, folder_repository, |tx| folder_repository.update_enabled(tx, folder_id, enabled).await)
    }

    async fn set_idle_enabled(&self, folder_id: FolderId, idle_enabled: bool) -> Result<(), Error> {
        with_transaction!(self, folder_repository, |tx| folder_repository
            .update_idle_enabled(tx, folder_id, idle_enabled)
            .await)
    }

    async fn record_sync_progress(&self, folder_id: FolderId, uidvalidity: u32, last_uid: u32, last_synced_at: DateTime<Utc>) -> Result<(), Error> {
        with_transaction!(self, folder_repository, |tx| folder_repository
            .update_sync_state(tx, folder_id, uidvalidity, last_uid, last_synced_at)
            .await)
    }

    async fn delete_folder(&self, folder_id: FolderId) -> Result<(), Error> {
        with_transaction!(self, folder_repository, |tx| folder_repository.delete_by_id(tx, folder_id).await)
    }
}

#[cfg(test)]
mod tests {
    use mockall::predicate::*;

    use super::*;
    use crate::{
        folder::{model::FolderBuilder, repository::MockFolderRepository},
        repository::testing::default_repository_service_builder,
    };

    fn setup_with_folder_repo(repo: MockFolderRepository) -> FolderServiceImpl {
        let rs = default_repository_service_builder()
            .folder_repository(Arc::new(repo))
            .build()
            .expect("all fields provided");
        FolderServiceImpl::new(Arc::new(rs))
    }

    fn make_folder(row: NewFolderRow) -> Folder {
        FolderBuilder::default()
            .id(1)
            .version(1)
            .token(row.token)
            .account_id(row.account_id)
            .path(row.path)
            .display_name(row.display_name)
            .special_use(row.special_use)
            .enabled(true)
            .idle_enabled(row.idle_enabled)
            .uidvalidity(row.uidvalidity)
            .build()
            .unwrap()
    }

    #[tokio::test]
    async fn create_folders_sets_idle_enabled_for_inbox() {
        let mut repo = MockFolderRepository::new();
        repo.expect_create_many()
            .withf(|_tx, _account, rows: &Vec<NewFolderRow>| {
                rows.iter().any(|r| r.path == "INBOX" && r.idle_enabled) && rows.iter().any(|r| r.path == "Sent" && !r.idle_enabled)
            })
            .times(1)
            .returning(|_, _, rows| Box::pin(async move { Ok(rows.into_iter().map(make_folder).collect()) }));

        let svc = setup_with_folder_repo(repo);
        let folders = svc
            .create_folders_for_account(
                1,
                vec![
                    NewFolderRequest {
                        path: "INBOX".into(),
                        display_name: None,
                        special_use: Some(SpecialUse::Inbox),
                        uidvalidity: None,
                    },
                    NewFolderRequest {
                        path: "Sent".into(),
                        display_name: None,
                        special_use: Some(SpecialUse::Sent),
                        uidvalidity: None,
                    },
                ],
            )
            .await
            .unwrap();
        assert_eq!(folders.len(), 2);
    }

    #[tokio::test]
    async fn create_folders_with_no_special_use_sets_idle_disabled() {
        let mut repo = MockFolderRepository::new();
        repo.expect_create_many()
            .withf(|_tx, _account, rows: &Vec<NewFolderRow>| rows.iter().all(|r| !r.idle_enabled))
            .times(1)
            .returning(|_, _, rows| Box::pin(async move { Ok(rows.into_iter().map(make_folder).collect()) }));

        let svc = setup_with_folder_repo(repo);
        let folders = svc
            .create_folders_for_account(
                1,
                vec![NewFolderRequest {
                    path: "Custom".into(),
                    display_name: None,
                    special_use: None,
                    uidvalidity: None,
                }],
            )
            .await
            .unwrap();
        assert_eq!(folders.len(), 1);
    }

    #[tokio::test]
    async fn set_idle_enabled_delegates_to_repo() {
        let mut repo = MockFolderRepository::new();
        repo.expect_update_idle_enabled()
            .withf(|_tx, folder_id, idle_enabled| *folder_id == 9 && *idle_enabled)
            .times(1)
            .returning(|_, _, _| Box::pin(async { Ok(()) }));

        let svc = setup_with_folder_repo(repo);
        svc.set_idle_enabled(9, true).await.unwrap();
    }

    #[tokio::test]
    async fn list_folders_delegates_to_repo() {
        let mut repo = MockFolderRepository::new();
        repo.expect_list_for_account()
            .with(always(), eq(42u64))
            .times(1)
            .returning(|_, _| Box::pin(async { Ok(vec![]) }));

        let svc = setup_with_folder_repo(repo);
        let folders = svc.list_folders(42).await.unwrap();
        assert!(folders.is_empty());
    }

    #[tokio::test]
    async fn set_enabled_delegates_to_repo() {
        let mut repo = MockFolderRepository::new();
        repo.expect_update_enabled()
            .withf(|_tx, folder_id, enabled| *folder_id == 7 && !*enabled)
            .times(1)
            .returning(|_, _, _| Box::pin(async { Ok(()) }));

        let svc = setup_with_folder_repo(repo);
        svc.set_enabled(7, false).await.unwrap();
    }

    #[tokio::test]
    async fn record_sync_progress_delegates_to_repo() {
        let when = Utc::now();
        let mut repo = MockFolderRepository::new();
        repo.expect_update_sync_state()
            .withf(move |_tx, folder_id, uidvalidity, last_uid, last_synced_at| {
                *folder_id == 5 && *uidvalidity == 100 && *last_uid == 999 && *last_synced_at == when
            })
            .times(1)
            .returning(|_, _, _, _, _| Box::pin(async { Ok(()) }));

        let svc = setup_with_folder_repo(repo);
        svc.record_sync_progress(5, 100, 999, when).await.unwrap();
    }
}
