//! Account domain — M1 stub.
//!
//! Hosts `AccountId` and `AccountToken` type aliases so that `CipherService`
//! can take a typed account identifier (used as AAD when encrypting per-account
//! content). MK-3 will add `model.rs`, `repository.rs`, and `service.rs`
//! alongside.

use mk_utils::{define_token_prefix, token::Token};

define_token_prefix!(AccountTokenPrefix, "A_");

pub type AccountId = u64;
pub type AccountToken = Token<AccountTokenPrefix, AccountId, { i64::MAX as u128 }>;

#[cfg(test)]
mod tests {
    use super::{AccountId, AccountToken};

    #[test]
    fn generated_token_id_is_u64() {
        let token = AccountToken::generate();
        let _id: AccountId = token.id();
    }

    #[test]
    fn token_display_has_account_prefix() {
        let token = AccountToken::generate();
        assert!(token.to_string().starts_with("A_"));
    }
}
