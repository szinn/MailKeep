use std::sync::Arc;

use async_trait::async_trait;

use crate::{
    Error, ExternalServicesBuilder,
    account::AccountId,
    crypto::{CipherService, create_cipher_service},
    storage::{AttachmentStorageService, RawStorageService},
    types::ContentHash,
};

/// Nop `RawStorageService` — panics on any call. Suitable only for tests that
/// do not exercise storage.
struct NopRawStorage;

#[async_trait]
impl RawStorageService for NopRawStorage {
    async fn put_if_absent(&self, _account_id: AccountId, _plaintext: &[u8]) -> Result<ContentHash, Error> {
        unimplemented!("NopRawStorage: not available in test context")
    }

    async fn get(&self, _account_id: AccountId, _key: &ContentHash) -> Result<Vec<u8>, Error> {
        unimplemented!("NopRawStorage: not available in test context")
    }

    async fn exists(&self, _account_id: AccountId, _key: &ContentHash) -> Result<bool, Error> {
        unimplemented!("NopRawStorage: not available in test context")
    }

    async fn delete_account(&self, _account_id: AccountId) -> Result<(), Error> {
        unimplemented!("NopRawStorage: not available in test context")
    }
}

/// Nop `AttachmentStorageService` — panics on any call. Suitable only for
/// tests that do not exercise attachment storage.
struct NopAttachmentStorage;

#[async_trait]
impl AttachmentStorageService for NopAttachmentStorage {
    async fn put_if_absent(&self, _account_id: AccountId, _plaintext: &[u8]) -> Result<ContentHash, Error> {
        unimplemented!("NopAttachmentStorage: not available in test context")
    }

    async fn get(&self, _account_id: AccountId, _key: &ContentHash) -> Result<Vec<u8>, Error> {
        unimplemented!("NopAttachmentStorage: not available in test context")
    }

    async fn exists(&self, _account_id: AccountId, _key: &ContentHash) -> Result<bool, Error> {
        unimplemented!("NopAttachmentStorage: not available in test context")
    }

    async fn delete_account(&self, _account_id: AccountId) -> Result<(), Error> {
        unimplemented!("NopAttachmentStorage: not available in test context")
    }
}

/// Returns a `CipherService` backed by a fixed test key. Suitable for any
/// test that exercises crypto without needing a real secret.
pub fn test_cipher_service() -> Arc<dyn CipherService> {
    create_cipher_service("test-support-secret")
}

/// Returns an `ExternalServicesBuilder` pre-populated with nop implementations
/// for all fields except `repository_service`, which callers must always
/// provide.
///
/// Override individual fields for the service(s) under test before calling
/// `.build()`.
#[must_use]
pub fn default_external_services_builder() -> ExternalServicesBuilder {
    ExternalServicesBuilder::default()
        .cipher_service(test_cipher_service())
        .raw_storage_service(Arc::new(NopRawStorage) as Arc<dyn RawStorageService>)
        .attachment_storage_service(Arc::new(NopAttachmentStorage) as Arc<dyn AttachmentStorageService>)
        .job_concurrency(1)
}
