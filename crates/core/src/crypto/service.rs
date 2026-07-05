use std::sync::Arc;

use chacha20poly1305::{
    ChaCha20Poly1305, Key, Nonce,
    aead::{Aead, Generate, KeyInit, Payload},
};

use super::master_key::MasterKey;
use crate::{Error, account::AccountId, crypto::Ciphertext};

/// Symmetric AEAD service.
///
/// Synchronous (no I/O — pure CPU). Implementations are cheap to clone via
/// `Arc`.
pub trait CipherService: Send + Sync {
    /// Encrypt `plaintext` for the given account. Infallible — key, nonce,
    /// AAD, and plaintext are all under our control; ChaCha20-Poly1305 cannot
    /// fail under valid inputs.
    fn encrypt(&self, account_id: AccountId, plaintext: &[u8]) -> Ciphertext;

    /// Decrypt the ciphertext for the given account.
    ///
    /// Returns `Err(Error::DecryptionFailed)` on AAD mismatch, tag mismatch,
    /// or tamper — uniformly, to avoid oracle leakage. Never panics.
    fn decrypt(&self, account_id: AccountId, ct: &Ciphertext) -> Result<Vec<u8>, Error>;
}

struct ChaChaCipherService {
    cipher: ChaCha20Poly1305,
}

impl ChaChaCipherService {
    fn new(master: &MasterKey) -> Self {
        let key = Key::from(*master.expose());
        let cipher = ChaCha20Poly1305::new(&key);
        Self { cipher }
    }
}

impl CipherService for ChaChaCipherService {
    fn encrypt(&self, account_id: AccountId, plaintext: &[u8]) -> Ciphertext {
        let nonce = Nonce::generate();
        let aad = account_id.to_be_bytes();
        let ct = self
            .cipher
            .encrypt(&nonce, Payload { msg: plaintext, aad: &aad })
            .expect("ChaCha20-Poly1305 encrypt under valid inputs never fails");
        let nonce_bytes: [u8; 12] = nonce.into();
        Ciphertext::from_parts(nonce_bytes, ct)
    }

    fn decrypt(&self, account_id: AccountId, ct: &Ciphertext) -> Result<Vec<u8>, Error> {
        let Some(nonce_bytes) = ct.nonce() else {
            return Err(Error::DecryptionFailed);
        };
        let nonce = Nonce::from(*nonce_bytes);
        let aad = account_id.to_be_bytes();
        self.cipher
            .decrypt(
                &nonce,
                Payload {
                    msg: ct.ciphertext_and_tag(),
                    aad: &aad,
                },
            )
            .map_err(|_| Error::DecryptionFailed)
    }
}

/// Build a `CipherService` from the configured secret string.
///
/// Internally derives the 32-byte HKDF master key and feeds it to the
/// ChaCha20-Poly1305 cipher. The key never escapes this module — callers
/// only hold the returned `Arc<dyn CipherService>`.
#[must_use]
pub fn create_cipher_service(secret: &str) -> Arc<dyn CipherService> {
    let master = MasterKey::derive(secret);
    Arc::new(ChaChaCipherService::new(&master))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::{CipherService, create_cipher_service};
    use crate::{Error, crypto::Ciphertext};

    fn service() -> Arc<dyn CipherService> {
        create_cipher_service("test-secret")
    }

    const ACCOUNT_A: u64 = 100;
    const ACCOUNT_B: u64 = 200;

    #[test]
    fn roundtrip_succeeds() {
        let svc = service();
        let plaintext = b"hello, mailkeep";
        let ct = svc.encrypt(ACCOUNT_A, plaintext);
        let pt = svc.decrypt(ACCOUNT_A, &ct).unwrap();
        assert_eq!(pt, plaintext);
    }

    #[test]
    fn fresh_nonce_per_encrypt_produces_different_ciphertext() {
        let svc = service();
        let plaintext = b"identical input";
        let ct1 = svc.encrypt(ACCOUNT_A, plaintext);
        let ct2 = svc.encrypt(ACCOUNT_A, plaintext);
        assert_ne!(ct1.as_bytes(), ct2.as_bytes());

        assert_eq!(svc.decrypt(ACCOUNT_A, &ct1).unwrap(), plaintext);
        assert_eq!(svc.decrypt(ACCOUNT_A, &ct2).unwrap(), plaintext);
    }

    #[test]
    fn aad_mismatch_returns_decryption_failed() {
        let svc = service();
        let ct = svc.encrypt(ACCOUNT_A, b"secret data");
        match svc.decrypt(ACCOUNT_B, &ct) {
            Err(Error::DecryptionFailed) => {}
            other => panic!("expected DecryptionFailed, got {other:?}"),
        }
    }

    #[test]
    fn tampered_ciphertext_returns_decryption_failed() {
        let svc = service();
        let ct = svc.encrypt(ACCOUNT_A, b"untouched");
        let mut raw = ct.as_bytes().to_vec();
        // Flip a byte in the ciphertext body (skip past the 12-byte nonce).
        raw[20] ^= 0xff;
        let nonce: [u8; 12] = <[u8; 12]>::try_from(&raw[..12]).unwrap();
        let tampered = Ciphertext::from_parts(nonce, raw[12..].to_vec());
        match svc.decrypt(ACCOUNT_A, &tampered) {
            Err(Error::DecryptionFailed) => {}
            other => panic!("expected DecryptionFailed, got {other:?}"),
        }
    }

    #[test]
    fn truncated_ciphertext_returns_decryption_failed() {
        let svc = service();
        let short = Ciphertext::from_raw(vec![0u8; 5]);
        assert!(matches!(svc.decrypt(ACCOUNT_A, &short), Err(Error::DecryptionFailed)));
    }
}
