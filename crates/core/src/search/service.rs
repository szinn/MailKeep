use crate::{Error, account::AccountId, search::SearchResults, user::UserId};

/// Full-text search over a user's archived mail.
#[cfg_attr(any(test, feature = "test-support"), mockall::automock)]
#[async_trait::async_trait]
pub trait SearchService: Send + Sync {
    /// Full-text + filtered search scoped to the requesting user's accounts.
    async fn search(&self, user_id: UserId, query: &str, limit: u32, offset: u32) -> Result<SearchResults, Error>;

    /// Remove all indexed documents for an account (called on account
    /// deletion).
    async fn delete_account(&self, account_id: AccountId) -> Result<(), Error>;
}
