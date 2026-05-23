pub mod account;
pub mod auth;
pub mod crypto;
pub mod error;
pub mod repository;
pub mod storage;
pub mod types;
pub mod user;

use std::sync::Arc;

use derive_builder::Builder;
pub use error::{Error, ErrorKind, RepositoryError};

use crate::{
    auth::{AuthService, AuthServiceImpl},
    crypto::CipherService,
    repository::RepositoryService,
    storage::{AttachmentStorageService, RawStorageService},
    user::{UserService, UserServiceImpl, UserSettingService, UserSettingServiceImpl},
};

#[cfg(feature = "test-support")]
pub mod test_support;

/// All externally-provided adapter implementations required by `CoreServices`.
///
/// Use `ExternalServicesBuilder` to construct — all fields are required and
/// `.build()` returns an error if any are missing.
#[derive(Builder)]
#[builder(pattern = "owned")]
pub struct ExternalServices {
    pub(crate) repository_service: Arc<RepositoryService>,
    pub(crate) cipher_service: Arc<dyn CipherService>,
    pub(crate) raw_storage_service: Arc<dyn RawStorageService>,
    pub(crate) attachment_storage_service: Arc<dyn AttachmentStorageService>,
}

pub struct CoreServices {
    pub auth_service: Arc<dyn AuthService>,
    pub user_service: Arc<dyn UserService>,
    pub user_setting_service: Arc<dyn UserSettingService>,
    pub cipher_service: Arc<dyn CipherService>,
    pub raw_storage_service: Arc<dyn RawStorageService>,
    pub attachment_storage_service: Arc<dyn AttachmentStorageService>,
}

impl CoreServices {
    pub(crate) fn new(external: ExternalServices) -> Self {
        let ExternalServices {
            repository_service,
            cipher_service,
            raw_storage_service,
            attachment_storage_service,
        } = external;

        Self {
            auth_service: Arc::new(AuthServiceImpl::new(repository_service.clone())),
            user_service: Arc::new(UserServiceImpl::new(repository_service.clone())),
            user_setting_service: Arc::new(UserSettingServiceImpl::new(repository_service)),
            cipher_service,
            raw_storage_service,
            attachment_storage_service,
        }
    }
}

pub fn create_services(external: ExternalServices) -> Result<Arc<CoreServices>, Error> {
    Ok(Arc::new(CoreServices::new(external)))
}
