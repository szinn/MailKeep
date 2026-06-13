mod model;
mod service;

use std::sync::Arc;

pub use model::{IngestRequest, IngestResult, ParseMessageJob};
pub use service::IngestService;
pub(crate) use service::IngestServiceImpl;

use crate::{jobs::JobService, storage::RawStorageService};

/// Construct the ingest service from its storage + jobs dependencies.
#[must_use]
pub fn create_ingest_service(raw_storage_service: Arc<dyn RawStorageService>, job_service: Arc<dyn JobService>) -> Arc<dyn IngestService> {
    Arc::new(IngestServiceImpl::new(raw_storage_service, job_service))
}
