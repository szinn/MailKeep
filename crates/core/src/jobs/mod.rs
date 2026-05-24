pub mod handler;
pub mod model;
pub mod repository;
pub mod service;
pub mod worker;

pub use handler::{ErasedJobHandler, JobHandler};
pub use model::{Job, JobId, JobStatus};
pub use priority::{PRIORITY_HEALTH, PRIORITY_NORMAL, PRIORITY_USER};
pub use repository::{Enqueueable, JobRepository, JobRepositoryExt};
pub use service::{JobService, JobServiceExt, create_job_service};
pub use worker::{JobWorker, JobWorkerSubsystem, create_job_worker_subsystem};

pub mod priority {
    /// Periodic health checks.
    pub const PRIORITY_HEALTH: i16 = 5;
    /// Standard pipeline work.
    pub const PRIORITY_NORMAL: i16 = 10;
    /// User-initiated actions.
    pub const PRIORITY_USER: i16 = 20;
}
