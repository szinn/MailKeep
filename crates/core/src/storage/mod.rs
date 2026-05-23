//! Encrypted-at-rest storage services.
//!
//! Two traits with identical shape (`RawStorageService` for raw .eml bytes,
//! `AttachmentStorageService` for extracted attachment bytes). Both accept
//! plaintext on `put_if_absent` / return plaintext from `get` — encryption
//! is structural to the trait contract. Implementations live in adapter
//! crates (`mk-storage` for the filesystem variant in M1).

mod service;

pub use service::{AttachmentStorageService, RawStorageService};
