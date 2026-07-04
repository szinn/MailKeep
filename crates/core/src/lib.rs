pub mod account;
pub mod auth;
pub mod crypto;
pub mod error;
pub mod event;
pub mod folder;
pub mod imap;
pub mod ingest;
pub mod jobs;
pub mod message;
pub mod repository;
pub mod search;
pub mod stats;
pub mod storage;
pub mod types;
pub mod user;

use std::sync::Arc;

use derive_builder::Builder;
pub use error::{Error, ErrorKind, RepositoryError};
use tokio_graceful_shutdown::{IntoSubsystem, SubsystemBuilder, SubsystemHandle};

use crate::{
    account::{AccountService, AccountServiceImpl},
    auth::{AuthService, AuthServiceImpl},
    crypto::CipherService,
    event::{EventService, create_event_service},
    folder::{FolderService, FolderServiceImpl},
    imap::{ImapAccountService, ImapPortFactory, create_imap_account_service},
    ingest::{IngestService, create_ingest_service},
    jobs::{JobService, create_job_service, create_job_worker_subsystem},
    message::{MessageService, MessageServiceImpl},
    repository::RepositoryService,
    search::SearchService,
    stats::{StatsService, StatsServiceImpl},
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
    pub(crate) search_service: Arc<dyn SearchService>,
    pub(crate) job_concurrency: usize,
    pub(crate) imap_port_factory: ImapPortFactory,
}

pub struct CoreServices {
    pub account_service: Arc<dyn AccountService>,
    pub event_service: Arc<dyn EventService>,
    pub auth_service: Arc<dyn AuthService>,
    pub user_service: Arc<dyn UserService>,
    pub user_setting_service: Arc<dyn UserSettingService>,
    pub folder_service: Arc<dyn FolderService>,
    pub message_service: Arc<dyn MessageService>,
    pub ingest_service: Arc<dyn IngestService>,
    pub imap_account_service: Arc<dyn ImapAccountService>,
    pub cipher_service: Arc<dyn CipherService>,
    pub raw_storage_service: Arc<dyn RawStorageService>,
    pub attachment_storage_service: Arc<dyn AttachmentStorageService>,
    pub search_service: Arc<dyn SearchService>,
    pub job_service: Arc<dyn JobService>,
    pub stats_service: Arc<dyn StatsService>,
    pub repository_service: Arc<RepositoryService>,
    pub job_concurrency: usize,
    pub(crate) wake_notify: Arc<tokio::sync::Notify>,
}

impl CoreServices {
    pub(crate) fn new(external: ExternalServices) -> Self {
        let ExternalServices {
            repository_service,
            cipher_service,
            raw_storage_service,
            attachment_storage_service,
            search_service,
            job_concurrency,
            imap_port_factory,
        } = external;

        let event_service = create_event_service();
        let (job_service, wake_notify) = create_job_service(repository_service.clone());

        let folder_service: Arc<dyn FolderService> = Arc::new(FolderServiceImpl::new(repository_service.clone()));
        let message_service: Arc<dyn MessageService> = Arc::new(MessageServiceImpl::new(repository_service.clone()));
        let stats_service: Arc<dyn StatsService> = Arc::new(StatsServiceImpl::new(repository_service.clone()));
        let ingest_service = create_ingest_service(raw_storage_service.clone(), job_service.clone());

        let account_service: Arc<dyn AccountService> = Arc::new(AccountServiceImpl::new(
            repository_service.clone(),
            cipher_service.clone(),
            raw_storage_service.clone(),
            attachment_storage_service.clone(),
            search_service.clone(),
            event_service.clone(),
        ));

        let imap_port = imap_port_factory(ingest_service.clone(), folder_service.clone(), message_service.clone());
        let imap_account_service = create_imap_account_service(imap_port, account_service.clone(), folder_service.clone(), cipher_service.clone());

        Self {
            account_service,
            event_service,
            auth_service: Arc::new(AuthServiceImpl::new(repository_service.clone())),
            user_service: Arc::new(UserServiceImpl::new(repository_service.clone())),
            user_setting_service: Arc::new(UserSettingServiceImpl::new(repository_service.clone())),
            folder_service,
            message_service,
            ingest_service,
            imap_account_service,
            cipher_service,
            raw_storage_service,
            attachment_storage_service,
            search_service,
            job_service,
            stats_service,
            repository_service,
            job_concurrency,
            wake_notify,
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
