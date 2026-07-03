use crate::{Error, repository::Transaction, stats::ArchiveStats, user::UserId};

#[async_trait::async_trait]
#[cfg_attr(test, mockall::automock)]
pub trait StatsRepository: Send + Sync {
    /// Compute aggregate statistics over the given user's own accounts in a
    /// single pass per table. Implementations MUST scope every aggregate to
    /// `user_id` (only the user's own accounts).
    async fn archive_stats(&self, transaction: &dyn Transaction, user_id: UserId) -> Result<ArchiveStats, Error>;
}
