//! Temporary stub `JobRepository` used by `create_repository_service` until
//! Task 5 of MK-2 lands the real `JobRepositoryAdapter`. Every method panics —
//! the binary must not invoke any `JobRepository` method between Task 2 and
//! Task 5. Delete this file as part of Task 5.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use mk_core::{
    Error,
    jobs::{Job, JobRepository},
    repository::Transaction,
};

pub(crate) struct NopJobRepository;

#[async_trait]
impl JobRepository for NopJobRepository {
    async fn enqueue_raw(&self, _tx: &dyn Transaction, _job_type: &str, _payload: serde_json::Value, _priority: i16) -> Result<Job, Error> {
        unimplemented!("NopJobRepository: real adapter lands in Task 5")
    }

    async fn enqueue_delayed(
        &self,
        _tx: &dyn Transaction,
        _job_type: &str,
        _payload: serde_json::Value,
        _priority: i16,
        _delay: chrono::Duration,
    ) -> Result<Job, Error> {
        unimplemented!("NopJobRepository: real adapter lands in Task 5")
    }

    async fn claim_next(&self, _tx: &dyn Transaction) -> Result<Option<Job>, Error> {
        unimplemented!("NopJobRepository: real adapter lands in Task 5")
    }

    async fn complete(&self, _tx: &dyn Transaction, _job: Job) -> Result<Job, Error> {
        unimplemented!("NopJobRepository: real adapter lands in Task 5")
    }

    async fn fail(&self, _tx: &dyn Transaction, _job: Job, _error: String) -> Result<Job, Error> {
        unimplemented!("NopJobRepository: real adapter lands in Task 5")
    }

    async fn reset_running_to_pending(&self, _tx: &dyn Transaction) -> Result<u64, Error> {
        unimplemented!("NopJobRepository: real adapter lands in Task 5")
    }

    async fn count_pending_by_type(&self, _tx: &dyn Transaction, _job_type: &str) -> Result<u64, Error> {
        unimplemented!("NopJobRepository: real adapter lands in Task 5")
    }

    async fn count_all_pending(&self, _tx: &dyn Transaction) -> Result<u64, Error> {
        unimplemented!("NopJobRepository: real adapter lands in Task 5")
    }

    async fn delete_old_jobs(&self, _tx: &dyn Transaction, _cutoff: DateTime<Utc>) -> Result<u64, Error> {
        unimplemented!("NopJobRepository: real adapter lands in Task 5")
    }
}
