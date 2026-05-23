//! Symmetric encryption layer.
//!
//! HKDF-SHA256 derives a 32-byte master key from `MAILKEEP__ENCRYPTION_SECRET`.
//! ChaCha20-Poly1305 AEAD encrypts content with `account_id.to_be_bytes()` as
//! AAD — binds each ciphertext to its owning account.

mod cipher;
mod master_key;
mod service;

pub use cipher::Ciphertext;
pub use service::{CipherService, create_cipher_service};
