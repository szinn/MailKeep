pub mod account;
pub mod auth;
pub mod crypto;
pub mod error;
pub mod jobs;
pub mod repository;
pub mod storage;
pub mod types;
pub mod user;

use std::sync::Arc;

use derive_builder::Builder;
pub use error::{Error, ErrorKind, RepositoryError};
use tokio_graceful_shutdown::{IntoSubsystem, SubsystemBuilder, SubsystemHandle};

use crate::{
    auth::{AuthService, AuthServiceImpl},
    crypto::CipherService,
    jobs::{JobService, create_job_service, create_job_worker_subsystem},
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
    pub(crate) job_concurrency: usize,
}

pub struct CoreServices {
    pub auth_service: Arc<dyn AuthService>,
    pub user_service: Arc<dyn UserService>,
    pub user_setting_service: Arc<dyn UserSettingService>,
    pub cipher_service: Arc<dyn CipherService>,
    pub raw_storage_service: Arc<dyn RawStorageService>,
    pub attachment_storage_service: Arc<dyn AttachmentStorageService>,
    pub job_service: Arc<dyn JobService>,
    pub repository_service: Arc<RepositoryService>,
    pub job_concurrency: usize,
}

impl CoreServices {
    pub(crate) fn new(external: ExternalServices) -> Self {
        let ExternalServices {
            repository_service,
            cipher_service,
            raw_storage_service,
            attachment_storage_service,
            job_concurrency,
        } = external;

        let job_service = create_job_service(repository_service.clone());

        Self {
            auth_service: Arc::new(AuthServiceImpl::new(repository_service.clone())),
            user_service: Arc::new(UserServiceImpl::new(repository_service.clone())),
            user_setting_service: Arc::new(UserSettingServiceImpl::new(repository_service.clone())),
            cipher_service,
            raw_storage_service,
            attachment_storage_service,
            job_service,
            repository_service,
            job_concurrency,
        }
    }
}

pub fn create_services(external: ExternalServices) -> Result<Arc<CoreServices>, Error> {
    Ok(Arc::new(CoreServices::new(external)))
}

pub struct CoreSubsystem {
    core: Arc<CoreServices>,
}

impl IntoSubsystem<Error> for CoreSubsystem {
    async fn run(self, subsys: &mut SubsystemHandle) -> Result<(), Error> {
        tracing::info!("CoreSubsystem starting...");

        let jobs = create_job_worker_subsystem(&self.core);

        subsys.start(SubsystemBuilder::new("Jobs", jobs.into_subsystem()));

        tracing::info!("CoreSubsystem started");

        subsys.on_shutdown_requested().await;
        Ok(())
    }
}

#[must_use]
pub fn create_core_subsystem(core: &Arc<CoreServices>) -> CoreSubsystem {
    CoreSubsystem { core: core.clone() }
}
