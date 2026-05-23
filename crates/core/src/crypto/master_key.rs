use hkdf::Hkdf;
use secrecy::{ExposeSecret, SecretBox};
use sha2::Sha256;

const HKDF_INFO: &[u8] = b"mailkeep:master:v1";

/// 32-byte master key derived from `MAILKEEP__ENCRYPTION_SECRET` via
/// HKDF-SHA256.
///
/// Crate-private — only `create_cipher_service` constructs one, and it
/// never leaks past the cipher service it builds. Wrapped in `SecretBox`
/// so the bytes are zeroized on drop; no `Debug` impl so the key cannot
/// be accidentally printed.
pub(crate) struct MasterKey(SecretBox<[u8; 32]>);

impl MasterKey {
    /// Derive a 32-byte key from the user-supplied secret.
    ///
    /// Uses HKDF-SHA256 with no salt and info = `b"mailkeep:master:v1"`. The
    /// secret is expected to be high-entropy (e.g. `openssl rand -hex 32`).
    /// Infallible — 32 bytes is well under HKDF's 255 × HashLen limit.
    pub(crate) fn derive(secret: &str) -> Self {
        let hk = Hkdf::<Sha256>::new(None, secret.as_bytes());
        let mut key = [0u8; 32];
        hk.expand(HKDF_INFO, &mut key).expect("HKDF expand for 32 bytes never fails");
        Self(SecretBox::new(Box::new(key)))
    }

    pub(crate) fn expose(&self) -> &[u8; 32] {
        self.0.expose_secret()
    }
}

#[cfg(test)]
mod tests {
    use super::MasterKey;

    #[test]
    fn derive_is_deterministic() {
        let a = MasterKey::derive("the same secret");
        let b = MasterKey::derive("the same secret");
        assert_eq!(a.expose(), b.expose());
    }

    #[test]
    fn different_secrets_produce_different_keys() {
        let a = MasterKey::derive("one");
        let b = MasterKey::derive("two");
        assert_ne!(a.expose(), b.expose());
    }

    #[test]
    fn derived_key_is_32_bytes() {
        let key = MasterKey::derive("any");
        assert_eq!(key.expose().len(), 32);
    }
}
