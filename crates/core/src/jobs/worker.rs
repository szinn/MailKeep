use std::{sync::Arc, time::Duration};

use tokio_graceful_shutdown::{IntoSubsystem, SubsystemBuilder, SubsystemHandle};

use crate::{
    CoreServices, Error,
    jobs::{JobRepository, JobService},
    repository::{Repository, transaction},
};

pub struct JobWorker {
    job_service: Arc<dyn JobService>,
    repository: Arc<dyn Repository>,
    job_repo: Arc<dyn JobRepository>,
    poll_interval: Duration,
}

impl JobWorker {
    pub fn new(job_service: Arc<dyn JobService>, repository: Arc<dyn Repository>, job_repo: Arc<dyn JobRepository>, poll_interval: Duration) -> Self {
        Self {
            job_service,
            repository,
            job_repo,
            poll_interval,
        }
    }
}

impl IntoSubsystem<Error> for JobWorker {
    async fn run(self, subsys: &mut SubsystemHandle) -> Result<(), Error> {
        let job_repo = self.job_repo;
        let repository = self.repository;
        let job_service = self.job_service;
        let poll_interval = self.poll_interval;

        // Crash recovery: reset any jobs left running from a previous crash.
        let reset = transaction(&*repository, |tx| {
            let job_repo = job_repo.clone();
            Box::pin(async move { job_repo.reset_running_to_pending(tx).await })
        })
        .await?;

        if reset > 0 {
            tracing::warn!("reset {} running jobs to pending after startup", reset);
        }

        let mut counter: u32 = 0;
        loop {
            tokio::select! {
                () = subsys.on_shutdown_requested() => {
                    tracing::info!("JobWorker shutting down...");
                    break;
                }
                () = async {} => {
                    let mut job_processed = false;
                    if counter == 0 {
                        let job = {
                            let job_repo = job_repo.clone();
                            match transaction(&*repository, |tx| {
                                Box::pin(async move { job_repo.claim_next(tx).await })
                            })
                            .await
                            {
                                Ok(j) => j,
                                Err(e) if e.is_transient() => {
                                    tracing::warn!("DB unavailable in worker (claim_next), pausing 10s: {e}");
                                    tokio::time::sleep(Duration::from_secs(10)).await;
                                    continue;
                                }
                                Err(e) => return Err(e),
                            }
                        };

                        if let Some(job) = job {
                            job_processed = true;
                            let job_type = job.job_type.clone();
                            let payload = job.payload.clone();

                            match job_service.dispatch(&job_type, payload).await {
                                Ok(()) => {
                                    let job_repo = job_repo.clone();
                                    match transaction(&*repository, |tx| {
                                        let job = job.clone();
                                        Box::pin(async move { job_repo.complete(tx, job).await })
                                    })
                                    .await
                                    {
                                        Ok(_) => {}
                                        Err(e) if e.is_transient() => {
                                            tracing::warn!("DB unavailable in worker (complete), pausing 10s: {e}");
                                            tokio::time::sleep(Duration::from_secs(10)).await;
                                            continue;
                                        }
                                        Err(e) => return Err(e),
                                    }
                                }
                                Err(e) => {
                                    tracing::error!(job_type, error = %e, "job handler failed");
                                    let job_repo = job_repo.clone();
                                    match transaction(&*repository, |tx| {
                                        let job = job.clone();
                                        Box::pin(async move {
                                            job_repo.fail(tx, job, e.to_string()).await
                                        })
                                    })
                                    .await
                                    {
                                        Ok(_) => {}
                                        Err(e) if e.is_transient() => {
                                            tracing::warn!("DB unavailable in worker (fail), pausing 10s: {e}");
                                            tokio::time::sleep(Duration::from_secs(10)).await;
                                            continue;
                                        }
                                        Err(e) => return Err(e),
                                    }
                                }
                            }
                        }
                    }

                    if !job_processed {
                        counter += 1;
                        #[expect(clippy::cast_possible_truncation, reason = "poll interval in seconds fits in u32")]
                        let poll_secs = poll_interval.as_secs() as u32;
                        if counter >= poll_secs {
                            counter = 0;
                        }
                    }
                    tokio::time::sleep(Duration::from_secs(1)).await;
                }
            }
        }

        Ok(())
    }
}

pub struct JobWorkerSubsystem {
    concurrency: usize,
    job_service: Arc<dyn JobService>,
    job_repo: Arc<dyn JobRepository>,
    repository: Arc<dyn Repository>,
}

impl IntoSubsystem<Error> for JobWorkerSubsystem {
    async fn run(self, subsys: &mut SubsystemHandle) -> Result<(), Error> {
        for i in 0..self.concurrency {
            let worker = JobWorker::new(self.job_service.clone(), self.repository.clone(), self.job_repo.clone(), Duration::from_secs(5));
            subsys.start(SubsystemBuilder::new(format!("job-worker-{i}"), worker.into_subsystem()));
        }
        subsys.on_shutdown_requested().await;
        Ok(())
    }
}

pub fn create_job_worker_subsystem(core: &Arc<CoreServices>) -> JobWorkerSubsystem {
    let concurrency = core.job_concurrency.max(1);
    JobWorkerSubsystem {
        concurrency,
        job_service: core.job_service.clone(),
        job_repo: core.repository_service.job_repository().clone(),
        repository: core.repository_service.repository().clone(),
    }
}

#[cfg(test)]
mod tests {
    use crate::{Error, RepositoryError};

    #[test]
    fn transient_connection_error_is_transient() {
        let e = Error::RepositoryError(RepositoryError::Connection("gone".into()));
        assert!(e.is_transient(), "connection error must be transient");
    }

    #[test]
    fn infrastructure_error_is_not_transient() {
        let e = Error::Infrastructure("bad query".into());
        assert!(!e.is_transient(), "infrastructure error must not be transient");
    }
}
