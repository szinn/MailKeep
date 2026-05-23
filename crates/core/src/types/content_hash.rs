use std::fmt::Write as _;

use sha2::{Digest, Sha256};

/// SHA-256 of the plaintext content. Used as the storage key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ContentHash([u8; 32]);

impl ContentHash {
    /// Compute the SHA-256 of the given plaintext.
    #[must_use]
    pub fn compute(bytes: &[u8]) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(bytes);
        let digest = hasher.finalize();
        let mut out = [0u8; 32];
        out.copy_from_slice(&digest);
        Self(out)
    }

    /// Lowercase hex representation (64 chars).
    #[must_use]
    pub fn as_hex(&self) -> String {
        let mut s = String::with_capacity(64);
        for b in &self.0 {
            write!(s, "{b:02x}").expect("write to String is infallible");
        }
        s
    }

    /// Two-level shard directory components from the first 4 hex chars.
    /// E.g., hash `2cf24...` → ("2c", "f2").
    #[must_use]
    pub fn shard_dirs(&self) -> (String, String) {
        (format!("{:02x}", self.0[0]), format!("{:02x}", self.0[1]))
    }

    /// The underlying 32-byte SHA-256 digest.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl std::fmt::Display for ContentHash {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.as_hex())
    }
}

#[cfg(test)]
mod tests {
    use super::ContentHash;

    #[test]
    fn compute_matches_known_sha256_of_hello() {
        let hash = ContentHash::compute(b"hello");
        assert_eq!(hash.as_hex(), "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824");
    }

    #[test]
    fn as_hex_is_lowercase_64_chars() {
        let hash = ContentHash::compute(b"any input");
        let hex = hash.as_hex();
        assert_eq!(hex.len(), 64);
        assert!(hex.chars().all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c)));
    }

    #[test]
    fn shard_dirs_returns_first_two_pairs() {
        let hash = ContentHash::compute(b"hello");
        let (ab, cd) = hash.shard_dirs();
        assert_eq!(ab, "2c");
        assert_eq!(cd, "f2");
    }

    #[test]
    fn display_matches_as_hex() {
        let hash = ContentHash::compute(b"hello");
        assert_eq!(format!("{hash}"), hash.as_hex());
    }

    #[test]
    fn equal_inputs_produce_equal_hashes() {
        let a = ContentHash::compute(b"identical");
        let b = ContentHash::compute(b"identical");
        assert_eq!(a, b);
    }

    #[test]
    fn different_inputs_produce_different_hashes() {
        let a = ContentHash::compute(b"one");
        let b = ContentHash::compute(b"two");
        assert_ne!(a, b);
    }
}
