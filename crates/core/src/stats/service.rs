use std::sync::Arc;

use crate::{Error, repository::RepositoryService, stats::ArchiveStats, user::UserId, with_read_only_transaction};

#[cfg_attr(any(test, feature = "test-support"), mockall::automock)]
#[async_trait::async_trait]
pub trait StatsService: Send + Sync {
    async fn archive_stats(&self, user_id: UserId) -> Result<ArchiveStats, Error>;
}

pub(crate) struct StatsServiceImpl {
    repository_service: Arc<RepositoryService>,
}

impl StatsServiceImpl {
    pub(crate) fn new(repository_service: Arc<RepositoryService>) -> Self {
        Self { repository_service }
    }
}

#[async_trait::async_trait]
impl StatsService for StatsServiceImpl {
    async fn archive_stats(&self, user_id: UserId) -> Result<ArchiveStats, Error> {
        with_read_only_transaction!(self, stats_repository, |tx| stats_repository.archive_stats(tx, user_id).await)
    }
}

#[cfg(test)]
mod tests {
    use chrono::{TimeZone, Utc};

    use super::*;
    use crate::{repository::testing::default_repository_service_builder, stats::repository::MockStatsRepository};

    #[tokio::test]
    async fn archive_stats_delegates_to_repo_with_user_id() {
        let expected = ArchiveStats {
            message_count: 42,
            attachment_count: 7,
            storage_bytes: 123_456,
            account_count: 3,
            last_synced_at: Some(Utc.timestamp_opt(1_700_000_000, 0).unwrap()),
        };
        let ret = expected.clone();
        let mut repo = MockStatsRepository::new();
        repo.expect_archive_stats().withf(|_tx, user_id| *user_id == 99).returning(move |_, _| {
            let r = ret.clone();
            Box::pin(async move { Ok(r) })
        });

        let repository_service = Arc::new(
            default_repository_service_builder()
                .stats_repository(Arc::new(repo))
                .build()
                .expect("all fields provided"),
        );
        let svc = StatsServiceImpl::new(repository_service);

        let got = svc.archive_stats(99).await.unwrap();
        assert_eq!(got, expected);
    }
}
