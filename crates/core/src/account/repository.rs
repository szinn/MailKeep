use crate::{
    Error,
    account::{Account, AccountId, AccountStatus, NewAccount},
    repository::Transaction,
    user::UserId,
};

#[async_trait::async_trait]
#[cfg_attr(test, mockall::automock)]
pub trait AccountRepository: Send + Sync {
    /// Insert a new account row. The caller (service) is responsible for
    /// generating the token, deriving `id` from it, and encrypting credentials
    /// with AAD = id before constructing `NewAccount`.
    async fn insert(&self, transaction: &dyn Transaction, new: NewAccount) -> Result<Account, Error>;

    /// Multi-tenant primary lookup. Returns `None` if the account does not
    /// exist OR if it belongs to a different user.
    async fn find_by_id_for_user(&self, transaction: &dyn Transaction, user_id: UserId, account_id: AccountId) -> Result<Option<Account>, Error>;

    async fn list_for_user(&self, transaction: &dyn Transaction, user_id: UserId) -> Result<Vec<Account>, Error>;

    /// All enabled accounts across users. Used by MK-7 at startup to enumerate
    /// sync targets — intentionally has no `user_id` parameter.
    async fn list_enabled(&self, transaction: &dyn Transaction) -> Result<Vec<Account>, Error>;

    /// Optimistic-locked update. Compares `account.version` against the row's
    /// stored version; returns
    /// `Error::RepositoryError(RepositoryError::Conflict)` on mismatch.
    /// Bumps version on success.
    async fn update(&self, transaction: &dyn Transaction, account: Account) -> Result<Account, Error>;

    /// System write — sets status (and optionally last_error). Bypasses the
    /// version check because this is called by the sync loop and must not
    /// contend with user form edits. Bumps version on commit.
    async fn set_status(&self, transaction: &dyn Transaction, account_id: AccountId, status: AccountStatus, last_error: Option<String>) -> Result<(), Error>;

    /// System write — sets enabled flag without version check. Bumps version.
    async fn set_enabled(&self, transaction: &dyn Transaction, account_id: AccountId, enabled: bool) -> Result<(), Error>;

    async fn delete(&self, transaction: &dyn Transaction, account: Account) -> Result<Account, Error>;
}
