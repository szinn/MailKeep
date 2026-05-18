pub mod auth;
pub mod user;

// use std::{
//     path::PathBuf,
//     sync::{Arc, Mutex},
//     time::Duration,
// };

use derive_builder::Builder;
pub use error::{Error, ErrorKind, RepositoryError};
// pub use resilience::{CheckResult, CheckedSubsystem, ResilienceWrapper};
// use tokio_graceful_shutdown::{
//     ErrorAction, IntoSubsystem, SubsystemBuilder, SubsystemHandle,
//     errors::{SubsystemError, SubsystemJoinError},
// };

// use crate::{
//     app_setting::{AppSettingService, AppSettingServiceImpl},
//     auth::{AuthService, AuthServiceImpl},
//     book::{BookService, BookServiceImpl},
//     collection::{CollectionService, CollectionServiceImpl},
//     device::{DeviceService, service::DeviceServiceImpl},
//     event::{EventService, create_event_service},
//     format::FormatService,
//     health::{HealthCheckSubsystem, HealthService, create_health_subsystem},
//     import::{BookdropScanSubsystem, ImportJobService, create_bookdrop_scan_subsystem},
//     jobs::{JobService, JobWorker, create_job_service},
//     koreader::{KoReaderService, KoReaderServiceImpl},
//     library::{LibraryService, LibraryServiceImpl},
//     message::{SystemMessageService, SystemMessageServiceImpl},
//     metadata::{MetadataService, create_metadata_service},
//     opds::{OpdsService, OpdsServiceImpl},
//     pipeline::{PipelineService, PipelineServiceImpl},
//     reading::{ReadingService, ReadingServiceImpl},
//     repository::RepositoryService,
//     shelf::{ShelfService, service::ShelfServiceImpl},
//     storage::FileStoreService,
//     user::{UserService, UserServiceImpl, UserSettingService, UserSettingServiceImpl},
// };

#[cfg(feature = "test-support")]
pub mod test_support;

/// All externally-provided adapter implementations required by `CoreServices`.
///
/// Use `ExternalServicesBuilder` to construct — all fields are required and
/// `.build()` returns an error if any are missing.
#[derive(Builder)]
#[builder(pattern = "owned")]
pub struct ExternalServices {
    // pub(crate) repository_service: Arc<RepositoryService>,
}

pub struct CoreServices {
    // pub(crate) repository_service: Arc<RepositoryService>,
    pub auth_service: Arc<dyn AuthService>,
    pub user_service: Arc<dyn UserService>,
}

impl CoreServices {
    pub(crate) fn new(external: ExternalServices, encryption_secret: &str) -> Self {
        let ExternalServices {
            // repository_service,
        } = external;

        Self {
            // repository_service: repository_service.clone(),
            auth_service: Arc::new(AuthServiceImpl::new(repository_service.clone())),
            user_service: Arc::new(UserServiceImpl::new(repository_service.clone())),
        }
    }
}

pub fn create_services(external: ExternalServices, encryption_secret: &str) -> Result<Arc<CoreServices>, Error> {
    Ok(Arc::new(CoreServices::new(external, encryption_secret)))
}
