use std::{sync::Arc, time::Duration};

use tokio_graceful_shutdown::{IntoSubsystem, SubsystemBuilder, SubsystemHandle};

use crate::{
    CoreServices, Error,
    jobs::{JobRepository, JobService},
    repository::{Repository, transaction},
};

pub(crate) struct JobWorker {
    job_service: Arc<dyn JobService>,
    repository: Arc<dyn Repository>,
    job_repo: Arc<dyn JobRepository>,
    poll_interval: Duration,
    notify: Arc<tokio::sync::Notify>,
}

impl JobWorker {
    pub(crate) fn new(
        job_service: Arc<dyn JobService>,
        repository: Arc<dyn Repository>,
        job_repo: Arc<dyn JobRepository>,
        poll_interval: Duration,
        notify: Arc<tokio::sync::Notify>,
    ) -> Self {
        Self {
            job_service,
            repository,
            job_repo,
            poll_interval,
            notify,
        }
    }
}

impl IntoSubsystem<Error> for JobWorker {
    async fn run(self, subsys: &mut SubsystemHandle) -> Result<(), Error> {
        let job_repo = self.job_repo;
        let repository = self.repository;
        let job_service = self.job_service;
        let poll_interval = self.poll_interval;
        let notify = self.notify;

        loop {
            // Top-of-loop shutdown check — covers shutdown firing while
            // no work is happening between claim attempts.
            if subsys.is_shutdown_requested() {
                tracing::info!("JobWorker shutting down...");
                break;
            }

            // Subscribe to wake notifications BEFORE checking the queue.
            // Any notify_waiters() fired between enable() and the await
            // in the None-branch below is delivered to this pinned future,
            // closing the edge-trigger race window of notify_waiters().
            let notified = notify.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();

            // Try to claim a job.
            let claimed = {
                let job_repo = job_repo.clone();
                match transaction(&*repository, |tx| Box::pin(async move { job_repo.claim_next(tx).await })).await {
                    Ok(j) => j,
                    Err(e) if e.is_transient() => {
                        tracing::warn!("DB unavailable in worker (claim_next), pausing 10s: {e}");
                        tokio::select! {
                            () = subsys.on_shutdown_requested() => {
                                tracing::info!("JobWorker shutting down...");
                                break;
                            }
                            () = tokio::time::sleep(Duration::from_secs(10)) => {}
                        }
                        continue;
                    }
                    Err(e) => return Err(e),
                }
            };

            let Some(job) = claimed else {
                // No work right now — wait for shutdown, OR an enqueue wake
                // (possibly already delivered to `notified` above), OR the
                // fallback poll heartbeat.
                tokio::select! {
                    () = subsys.on_shutdown_requested() => {
                        tracing::info!("JobWorker shutting down...");
                        break;
                    }
                    () = notified => {}
                    () = tokio::time::sleep(poll_interval) => {}
                }
                continue;
            };

            // Dispatch the job's handler, then commit success/failure.
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
                            tokio::select! {
                                () = subsys.on_shutdown_requested() => {
                                    tracing::info!("JobWorker shutting down...");
                                    break;
                                }
                                () = tokio::time::sleep(Duration::from_secs(10)) => {}
                            }
                        }
                        Err(e) => return Err(e),
                    }
                }
                Err(e) => {
                    tracing::error!(job_type, error = %e, "job handler failed");
                    let job_repo = job_repo.clone();
                    match transaction(&*repository, |tx| {
                        let job = job.clone();
                        Box::pin(async move { job_repo.fail(tx, job, e.to_string()).await })
                    })
                    .await
                    {
                        Ok(_) => {}
                        Err(e) if e.is_transient() => {
                            tracing::warn!("DB unavailable in worker (fail), pausing 10s: {e}");
                            tokio::select! {
                                () = subsys.on_shutdown_requested() => {
                                    tracing::info!("JobWorker shutting down...");
                                    break;
                                }
                                () = tokio::time::sleep(Duration::from_secs(10)) => {}
                            }
                        }
                        Err(e) => return Err(e),
                    }
                }
            }
            // Loop back immediately — no inter-job sleep.
        }

        Ok(())
    }
}

pub(crate) struct JobWorkerSubsystem {
    concurrency: usize,
    job_service: Arc<dyn JobService>,
    job_repo: Arc<dyn JobRepository>,
    repository: Arc<dyn Repository>,
    notify: Arc<tokio::sync::Notify>,
}

impl IntoSubsystem<Error> for JobWorkerSubsystem {
    async fn run(self, subsys: &mut SubsystemHandle) -> Result<(), Error> {
        tracing::info!("JobWorkerSubsystem starting {} workers...", self.concurrency);

        // Crash recovery: reset any jobs left running from a previous crash.
        let job_repo = self.job_repo.clone();
        let repository = self.repository.clone();

        let reset = transaction(&*repository, |tx| {
            let job_repo = job_repo.clone();
            Box::pin(async move { job_repo.reset_running_to_pending(tx).await })
        })
        .await?;

        if reset > 0 {
            tracing::warn!("reset {} running jobs to pending after startup", reset);
        }

        for i in 0..self.concurrency {
            let worker = JobWorker::new(
                self.job_service.clone(),
                self.repository.clone(),
                self.job_repo.clone(),
                Duration::from_secs(5),
                self.notify.clone(),
            );
            subsys.start(SubsystemBuilder::new(format!("job-worker-{i}"), worker.into_subsystem()));
        }

        tracing::info!("JobWorkerSubsystem started");

        subsys.on_shutdown_requested().await;
        Ok(())
    }
}

pub(crate) fn create_job_worker_subsystem(core: &Arc<CoreServices>) -> JobWorkerSubsystem {
    let concurrency = core.job_concurrency.max(1);
    JobWorkerSubsystem {
        concurrency,
        job_service: core.job_service.clone(),
        job_repo: core.repository_service.job_repository().clone(),
        repository: core.repository_service.repository().clone(),
        notify: core.wake_notify.clone(),
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
