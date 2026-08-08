use alderkit_token::define_alphabet;
pub use alderkit_token::{define_token_prefix, token::Token};

// The base-32 alphabet used to encode every token in MailKeep.
//
// Must stay byte-for-byte identical to the alphabet `mk-utils`'s `Token`
// used before this crate switched to `alderkit-token` — changing it would
// invalidate every token already issued (stored in the database, embedded
// in outstanding URLs/emails).
define_alphabet!(MailKeepAlphabet, b"Y4XK0N8AR3G6JM2VT9BS5WC1DPH7EUZF");

#[cfg(test)]
mod tests {
    use super::*;

    define_token_prefix!(GoldenPrefix, "G_");
    type GoldenToken = Token<GoldenPrefix, u64, MailKeepAlphabet>;

    #[test]
    fn alphabet_matches_pre_migration_encoding() {
        // Golden values carried over from mk-utils's own test suite
        // (encoding depends only on the alphabet and id, not the prefix).
        assert_eq!(GoldenToken::new(0).to_string(), "G_YYYYYYYYYYYYY");
        assert_eq!(GoldenToken::new(1).to_string(), "G_YYYYYYYYYYYY4");
    }
}
