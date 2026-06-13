use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use mk_core::{
    Error, RepositoryError, create_core_subsystem,
    jobs::{Enqueueable, JobHandler, JobServiceExt, PRIORITY_NORMAL},
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
        let core_subsystem = create_core_subsystem(&core);
        Toplevel::new(async move |s: &mut SubsystemHandle| {
            s.start(SubsystemBuilder::new("Core", core_subsystem.into_subsystem()));
        })
        .handle_shutdown_requests(Duration::from_secs(5))
        .await
    });

    // Enqueue a job.
    ctx.services.job_service.enqueue(&TestPayload { value: 42 }).await.expect("enqueue");

    // Wait for the worker to process it.
    let processed = wait_until(|| !observed.lock().unwrap().is_empty(), Duration::from_millis(500)).await;
    assert!(processed, "worker should have picked up the job within 500ms (wake-on-enqueue)");
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
//   2. Start subsystem with a co-resident "shutdown trigger" subsystem.
//   3. Trigger subsystem waits for handler_started, then calls
//      request_shutdown().
//   4. With shutdown signalled, the blocking handler is released.
//   5. Worker commits job 1 completion, loops back, sees
//      is_shutdown_requested(), breaks — never reaching the claim_next call
//      that would pick up job 2.
//   6. Assert count_all_pending == 1: job 1 = Completed, job 2 = Pending.
//
// Using request_shutdown() (graceful) rather than aborting the tokio task
// ensures is_shutdown_requested() returns true inside the worker loop,
// which is the code path being tested (MK-13).

#[tokio::test]
async fn test_graceful_drain() {
    use tokio::sync::oneshot;

    let ctx = setup().await;
    let (release_tx, release_rx) = oneshot::channel::<()>();
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

    // Start the subsystem with a co-resident shutdown-trigger subsystem.
    // The trigger polls handler_started every 50ms; once the blocking handler
    // has started, it calls subsys.request_shutdown() to set the graceful
    // shutdown flag. This ensures is_shutdown_requested() returns true on the
    // worker's next top-of-loop check (after job 1 is committed).
    let core_for_subsys = core.clone();
    let handler_started_for_trigger = handler_started.clone();
    let toplevel_handle = tokio::spawn(async move {
        Toplevel::new(async move |s: &mut SubsystemHandle| {
            s.start(SubsystemBuilder::new("Core", create_core_subsystem(&core_for_subsys).into_subsystem()));
            struct ShutdownTrigger {
                started: Arc<Mutex<bool>>,
            }
            impl IntoSubsystem<mk_core::Error> for ShutdownTrigger {
                async fn run(self, subsys: &mut SubsystemHandle) -> Result<(), mk_core::Error> {
                    while !*self.started.lock().unwrap() {
                        tokio::time::sleep(Duration::from_millis(50)).await;
                    }
                    // Handler has started — request graceful shutdown now so
                    // is_shutdown_requested() is true by the time job 1 commits.
                    subsys.request_shutdown();
                    Ok(())
                }
            }
            s.start(SubsystemBuilder::new(
                "ShutdownTrigger",
                ShutdownTrigger {
                    started: handler_started_for_trigger,
                }
                .into_subsystem(),
            ));
        })
        .handle_shutdown_requests(Duration::from_secs(10))
        .await
    });

    // Wait for the trigger subsystem to confirm the handler started.
    let started = wait_until(|| *handler_started.lock().unwrap(), Duration::from_secs(10)).await;
    assert!(started, "handler should have started within 10s");

    // Shutdown has been requested (or will be shortly). Release the in-flight
    // handler so it can complete and commit. The worker will see the shutdown
    // flag on its next top-of-loop iteration and exit without claiming job 2.
    let _ = release_tx.send(());

    // Wait for the toplevel to finish gracefully (it shuts down naturally after
    // request_shutdown() is processed).
    let _ = tokio::time::timeout(Duration::from_secs(15), toplevel_handle).await;

    // Assert: count_all_pending counts both Pending and Running rows.
    // Expected count is 1:
    //   - job 1 = Completed (excluded from count)
    //   - job 2 = Pending (counted — never claimed because shutdown stopped the
    //     worker loop before the next claim_next call)
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

// ─── Test 5: wake fires worker mid-poll (MK-16) ───────────────────────────

#[tokio::test]
async fn test_wake_mid_poll() {
    let ctx = setup().await;
    let observed = Arc::new(Mutex::new(vec![]));
    ctx.services.job_service.register(RecordingHandler { observed: observed.clone() });

    // Start subsystem with NO pending jobs. Workers will call claim_next
    // (return None) and enter the None-branch select! to block.
    let core = ctx.services.clone();
    let toplevel_handle = tokio::spawn(async move {
        let core_subsystem = create_core_subsystem(&core);
        Toplevel::new(async move |s: &mut SubsystemHandle| {
            s.start(SubsystemBuilder::new("Core", core_subsystem.into_subsystem()));
        })
        .handle_shutdown_requests(Duration::from_secs(5))
        .await
    });

    // Wait long enough for workers to be genuinely inside the select's
    // poll-interval sleep arm.
    tokio::time::sleep(Duration::from_millis(200)).await;

    // Enqueue. notify_waiters() fires, worker wakes from the 5s sleep.
    let enqueue_start = std::time::Instant::now();
    ctx.services.job_service.enqueue(&TestPayload { value: 99 }).await.unwrap();

    let processed = wait_until(|| !observed.lock().unwrap().is_empty(), Duration::from_millis(500)).await;
    assert!(processed, "wake should have fired worker mid-poll within 500ms");

    let elapsed = enqueue_start.elapsed();
    assert!(
        elapsed < Duration::from_secs(2),
        "wake-driven claim took {}ms — should be near-instant, well under the 5s poll interval",
        elapsed.as_millis()
    );
    assert_eq!(*observed.lock().unwrap(), vec![99]);

    toplevel_handle.abort();
    let _ = toplevel_handle.await;
}

// ─── Helpers for terminal/transient tests ─────────────────────────────────

struct FailingHandler {
    transient: bool,
}
impl JobHandler for FailingHandler {
    const JOB_TYPE: &'static str = "integration.test";
    const DISPLAY_NAME: &'static str = "Failing Handler (integration test)";
    type Payload = TestPayload;
    async fn handle(&self, _payload: TestPayload) -> Result<(), Error> {
        if self.transient {
            Err(Error::RepositoryError(RepositoryError::Connection("simulated".into())))
        } else {
            Err(Error::Validation("permanently bad message".into()))
        }
    }
}

async fn count_pending(ctx: &crate::context::TestContext) -> u64 {
    let repo = ctx.repos.repository().clone();
    let job_repo = ctx.repos.job_repository().clone();
    transaction(&*repo, |tx| {
        let r = job_repo.clone();
        Box::pin(async move { r.count_all_pending(tx).await })
    })
    .await
    .unwrap()
}

// ─── Test: terminal error fails job without retry ──────────────────────────

#[tokio::test]
async fn test_terminal_error_fails_without_retry() {
    let ctx = setup().await;
    ctx.services.job_service.register(FailingHandler { transient: false });

    let core = ctx.services.clone();
    let toplevel_handle = tokio::spawn(async move {
        Toplevel::new(async move |s: &mut SubsystemHandle| {
            s.start(SubsystemBuilder::new("Core", create_core_subsystem(&core).into_subsystem()));
        })
        .handle_shutdown_requests(Duration::from_secs(5))
        .await
    });

    ctx.services.job_service.enqueue(&TestPayload { value: 1 }).await.unwrap();

    // Terminal failure: the job is marked Failed and never re-queued.
    // Poll until pending count reaches 0 (job moved to Failed).
    let settled = {
        let mut ok = false;
        for _ in 0..20 {
            if count_pending(&ctx).await == 0 {
                ok = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        ok
    };
    assert!(settled, "terminal-failed job must not remain pending/running");

    toplevel_handle.abort();
    let _ = toplevel_handle.await;
}

// ─── Test: transient error reschedules job ─────────────────────────────────

#[tokio::test]
async fn test_transient_error_reschedules() {
    let ctx = setup().await;
    ctx.services.job_service.register(FailingHandler { transient: true });

    let core = ctx.services.clone();
    let toplevel_handle = tokio::spawn(async move {
        Toplevel::new(async move |s: &mut SubsystemHandle| {
            s.start(SubsystemBuilder::new("Core", create_core_subsystem(&core).into_subsystem()));
        })
        .handle_shutdown_requests(Duration::from_secs(5))
        .await
    });

    ctx.services.job_service.enqueue(&TestPayload { value: 2 }).await.unwrap();

    // Transient failure reschedules with backoff (≥60s), so the job stays
    // pending (count == 1) and is not immediately re-claimed.
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(count_pending(&ctx).await, 1, "transient-failed job should be rescheduled (still pending)");

    toplevel_handle.abort();
    let _ = toplevel_handle.await;
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
    // No complete/fail afterwards — the claimed job is simply left Running.
    let job_repo = ctx.repos.job_repository().clone();
    let claimed = transaction(&**ctx.repos.repository(), |tx| {
        let job_repo = job_repo.clone();
        Box::pin(async move { job_repo.claim_next(tx).await })
    })
    .await
    .unwrap()
    .expect("a job was enqueued");
    assert_eq!(claimed.id, job_id);

    // Start the subsystem. crash-recovery `reset_running_to_pending` should
    // flip the abandoned row back to Pending, then the worker claims and
    // completes it normally.
    let core = ctx.services.clone();
    let toplevel_handle = tokio::spawn(async move {
        let core_subsystem = create_core_subsystem(&core);
        Toplevel::new(async move |s: &mut SubsystemHandle| {
            s.start(SubsystemBuilder::new("Core", core_subsystem.into_subsystem()));
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
