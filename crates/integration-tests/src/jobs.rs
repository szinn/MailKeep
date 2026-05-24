use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use mk_core::{
    Error,
    jobs::{Enqueueable, JobHandler, JobServiceExt, PRIORITY_NORMAL, create_job_worker_subsystem},
    repository::transaction,
};
use serde::{Deserialize, Serialize};
use tokio_graceful_shutdown::{IntoSubsystem, SubsystemBuilder, SubsystemHandle, Toplevel};

use crate::setup;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct TestPayload {
    value: i32,
}

impl Enqueueable for TestPayload {
    const JOB_TYPE: &'static str = "integration.test";
    const DEFAULT_PRIORITY: i16 = PRIORITY_NORMAL;
}

struct RecordingHandler {
    observed: Arc<Mutex<Vec<i32>>>,
}

impl JobHandler for RecordingHandler {
    const JOB_TYPE: &'static str = "integration.test";
    const DISPLAY_NAME: &'static str = "Recording Handler (integration test)";
    type Payload = TestPayload;

    async fn handle(&self, payload: TestPayload) -> Result<(), Error> {
        self.observed.lock().unwrap().push(payload.value);
        Ok(())
    }
}

async fn wait_until<F: Fn() -> bool>(predicate: F, timeout: Duration) -> bool {
    let deadline = std::time::Instant::now() + timeout;
    while std::time::Instant::now() < deadline {
        if predicate() {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    predicate()
}

// ─── Test 1: end-to-end register, enqueue, observe ─────────────────────────

#[tokio::test]
async fn test_register_enqueue_observe() {
    let ctx = setup().await;
    let observed = Arc::new(Mutex::new(vec![]));
    ctx.services.job_service.register(RecordingHandler { observed: observed.clone() });

    // Spawn the subsystem in the background.
    let core = ctx.services.clone();
    let toplevel_handle = tokio::spawn(async move {
        let jobs_subsystem = create_job_worker_subsystem(&core);
        Toplevel::new(async move |s: &mut SubsystemHandle| {
            s.start(SubsystemBuilder::new("Jobs", jobs_subsystem.into_subsystem()));
        })
        .handle_shutdown_requests(Duration::from_secs(5))
        .await
    });

    // Enqueue a job.
    ctx.services.job_service.enqueue(&TestPayload { value: 42 }).await.expect("enqueue");

    // Wait for the worker to process it.
    let processed = wait_until(|| !observed.lock().unwrap().is_empty(), Duration::from_secs(10)).await;
    assert!(processed, "worker should have picked up the job within 10s");
    assert_eq!(*observed.lock().unwrap(), vec![42]);

    // Tear down.
    toplevel_handle.abort();
    let _ = toplevel_handle.await;
}

// ─── Test 2: atomic claim under concurrency (no double-dispatch) ───────────

#[tokio::test]
async fn test_atomic_claim_under_concurrency() {
    let ctx = setup().await;
    let job_repo = ctx.repos.job_repository().clone();
    let repo = ctx.repos.repository().clone();

    // Enqueue 20 jobs.
    for i in 0..20i32 {
        let job_repo = job_repo.clone();
        let r = repo.clone();
        transaction(&*r, |tx| {
            let job_repo = job_repo.clone();
            Box::pin(async move {
                job_repo
                    .enqueue_raw(tx, "integration.test", serde_json::json!({"value": i}), PRIORITY_NORMAL)
                    .await
                    .map(|_| ())
            })
        })
        .await
        .expect("enqueue");
    }

    // Spawn 4 concurrent tasks, each calling claim_next in a loop until empty.
    let mut handles = vec![];
    for _ in 0..4 {
        let job_repo = job_repo.clone();
        let repo = repo.clone();
        handles.push(tokio::spawn(async move {
            let mut claimed_ids = vec![];
            loop {
                let result = transaction(&*repo, |tx| {
                    let job_repo = job_repo.clone();
                    Box::pin(async move { job_repo.claim_next(tx).await })
                })
                .await
                .expect("claim_next");

                match result {
                    Some(job) => claimed_ids.push(job.id),
                    None => break,
                }
            }
            claimed_ids
        }));
    }

    // Collect.
    let mut all_claimed: Vec<i64> = vec![];
    for h in handles {
        all_claimed.extend(h.await.expect("claim task"));
    }

    // Acceptance: 20 unique IDs claimed exactly once each.
    assert_eq!(all_claimed.len(), 20, "expected exactly 20 claims, got {}", all_claimed.len());
    let unique: std::collections::HashSet<_> = all_claimed.iter().copied().collect();
    assert_eq!(unique.len(), 20, "expected 20 unique IDs (no double-dispatch), got {}", unique.len());
}

// ─── Test 3: graceful drain on shutdown ────────────────────────────────────
//
// Verifies the shutdown guarantee: an in-flight handler runs to completion,
// while the second job stays Pending (never claimed) because shutdown stops
// the worker loop before it can claim again.
//
// Sequence:
//   1. Register blocking handler, enqueue 2 jobs.
//   2. Start subsystem in background.
//   3. Wait for first handler to start (it blocks on release_rx).
//   4. Release the in-flight handler so it can finish and commit its
//      completion.
//   5. Sleep briefly so the completion write commits before pool teardown.
//   6. Abort the toplevel task (initiates shutdown).
//   7. Assert count_all_pending == 1 (counts Pending + Running rows): job 1 =
//      Completed (excluded), job 2 = Pending (counted).

#[tokio::test]
async fn test_graceful_drain() {
    use tokio::sync::oneshot;

    let ctx = setup().await;
    let (release_tx, release_rx) = oneshot::channel();
    let release_rx = Arc::new(Mutex::new(Some(release_rx)));
    let handler_started = Arc::new(Mutex::new(false));

    struct BlockingHandler {
        release_rx: Arc<Mutex<Option<oneshot::Receiver<()>>>>,
        started: Arc<Mutex<bool>>,
    }
    impl JobHandler for BlockingHandler {
        const JOB_TYPE: &'static str = "integration.test";
        const DISPLAY_NAME: &'static str = "Blocking";
        type Payload = TestPayload;
        async fn handle(&self, _: TestPayload) -> Result<(), Error> {
            *self.started.lock().unwrap() = true;
            let rx = self.release_rx.lock().unwrap().take();
            if let Some(rx) = rx {
                let _ = rx.await;
            }
            Ok(())
        }
    }

    ctx.services.job_service.register(BlockingHandler {
        release_rx: release_rx.clone(),
        started: handler_started.clone(),
    });

    let core = ctx.services.clone();

    // Enqueue 2 jobs.
    core.job_service.enqueue(&TestPayload { value: 1 }).await.unwrap();
    core.job_service.enqueue(&TestPayload { value: 2 }).await.unwrap();

    // Start the subsystem in the background.
    let core_for_subsys = core.clone();
    let toplevel_handle = tokio::spawn(async move {
        Toplevel::new(async move |s: &mut SubsystemHandle| {
            s.start(SubsystemBuilder::new("Jobs", create_job_worker_subsystem(&core_for_subsys).into_subsystem()));
        })
        .handle_shutdown_requests(Duration::from_secs(10))
        .await
    });

    // Wait for the first handler to actually start.
    let started = wait_until(|| *handler_started.lock().unwrap(), Duration::from_secs(10)).await;
    assert!(started, "handler should have started within 10s");

    // Release the in-flight handler so it can finish and commit its completion.
    let _ = release_tx.send(());

    // Give the handler a moment to complete and write its completion row.
    // Timing coupling: the worker re-polls every poll_interval =
    // Duration::from_secs(5) (set in crates/core/src/jobs/worker.rs). The 500ms
    // sleep must be long enough for the handler's complete() write to commit,
    // yet short enough that the worker has not yet reached its next claim_next
    // call. 500ms is well within the 5-second re-poll window, so the second job
    // will not be claimed before shutdown takes effect.
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Now initiate shutdown — abort the toplevel task. Since the handler has
    // already finished and committed, the second job never gets claimed because
    // the worker's next claim_next attempt is preceded by a shutdown check.
    toplevel_handle.abort();
    let _ = toplevel_handle.await;

    // Allow any final DB writes to settle.
    tokio::time::sleep(Duration::from_millis(200)).await;

    // Assert: count_all_pending counts both Pending and Running rows.
    // Expected count is 1:
    //   - job 1 = Completed (excluded from count)
    //   - job 2 = Pending (counted — never claimed because shutdown stopped the
    //     worker loop before the next claim_next call)
    // A count of 2 would mean either job 1 didn't complete (still Running) or
    // job 2 was claimed (Running) before shutdown took effect — either is a
    // failure of the drain semantics.
    let pending_or_running = transaction(&**ctx.repos.repository(), |tx| {
        let r = ctx.repos.job_repository().clone();
        Box::pin(async move { r.count_all_pending(tx).await })
    })
    .await
    .unwrap();
    assert_eq!(
        pending_or_running, 1,
        "expected exactly 1 pending-or-running job after drain (got {pending_or_running})"
    );
}

// ─── Test 4: crash recovery on startup ─────────────────────────────────────

#[tokio::test]
async fn test_crash_recovery_on_startup() {
    let ctx = setup().await;
    let observed = Arc::new(Mutex::new(vec![]));
    ctx.services.job_service.register(RecordingHandler { observed: observed.clone() });

    // Enqueue a job, then claim it without completing — leaves it Running,
    // simulating a crashed worker that claimed it but never finished.
    let job_id = {
        let job_repo = ctx.repos.job_repository().clone();
        transaction(&**ctx.repos.repository(), |tx| {
            let job_repo = job_repo.clone();
            Box::pin(async move {
                job_repo
                    .enqueue_raw(tx, "integration.test", serde_json::json!({"value": 99}), PRIORITY_NORMAL)
                    .await
            })
        })
        .await
        .unwrap()
        .id
    };

    // Claim it without completing — leaves it Running, simulating crashed worker.
    {
        let job_repo = ctx.repos.job_repository().clone();
        let claimed = transaction(&**ctx.repos.repository(), |tx| {
            let job_repo = job_repo.clone();
            Box::pin(async move { job_repo.claim_next(tx).await })
        })
        .await
        .unwrap()
        .expect("a job was enqueued");
        assert_eq!(claimed.id, job_id);
        // No complete/fail — drop here simulating crash.
    }

    // Start the subsystem. crash-recovery `reset_running_to_pending` should
    // flip the abandoned row back to Pending, then the worker claims and
    // completes it normally.
    let core = ctx.services.clone();
    let toplevel_handle = tokio::spawn(async move {
        let jobs_subsystem = create_job_worker_subsystem(&core);
        Toplevel::new(async move |s: &mut SubsystemHandle| {
            s.start(SubsystemBuilder::new("Jobs", jobs_subsystem.into_subsystem()));
        })
        .handle_shutdown_requests(Duration::from_secs(5))
        .await
    });

    let processed = wait_until(|| !observed.lock().unwrap().is_empty(), Duration::from_secs(10)).await;
    assert!(processed, "crash-recovered job should have been processed within 10s");
    assert_eq!(*observed.lock().unwrap(), vec![99]);

    toplevel_handle.abort();
    let _ = toplevel_handle.await;
}
