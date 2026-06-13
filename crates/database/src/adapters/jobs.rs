use chrono::{DateTime, Utc};
use mk_core::{
    Error, RepositoryError,
    jobs::{Job, JobRepository, JobStatus},
    repository::Transaction,
};
use sea_orm::{ActiveModelBehavior, ActiveModelTrait, ColumnTrait, EntityTrait, ExprTrait, PaginatorTrait, QueryFilter, QueryOrder, sea_query::Expr};

use crate::{
    entities::{jobs, prelude},
    error::handle_dberr,
    transaction::TransactionImpl,
};

pub(crate) struct JobRepositoryAdapter;

impl JobRepositoryAdapter {
    pub(crate) fn new() -> Self {
        Self
    }
}

#[async_trait::async_trait]
impl JobRepository for JobRepositoryAdapter {
    async fn enqueue_raw(&self, transaction: &dyn Transaction, job_type: &str, payload: serde_json::Value, priority: i16) -> Result<Job, Error> {
        let db_tx = TransactionImpl::get_db_transaction(transaction)?;

        let model = jobs::ActiveModel {
            job_type: sea_orm::ActiveValue::Set(job_type.to_owned()),
            payload: sea_orm::ActiveValue::Set(payload),
            priority: sea_orm::ActiveValue::Set(priority),
            ..jobs::ActiveModel::new()
        };

        let inserted = model.insert(db_tx).await.map_err(handle_dberr)?;
        Ok(inserted.into())
    }

    async fn enqueue_delayed(
        &self,
        transaction: &dyn Transaction,
        job_type: &str,
        payload: serde_json::Value,
        priority: i16,
        delay: chrono::Duration,
    ) -> Result<Job, Error> {
        let db_tx = TransactionImpl::get_db_transaction(transaction)?;

        let model = jobs::ActiveModel {
            job_type: sea_orm::ActiveValue::Set(job_type.to_owned()),
            payload: sea_orm::ActiveValue::Set(payload),
            priority: sea_orm::ActiveValue::Set(priority),
            scheduled_at: sea_orm::ActiveValue::Set((Utc::now() + delay).into()),
            ..jobs::ActiveModel::new()
        };

        let inserted = model.insert(db_tx).await.map_err(handle_dberr)?;
        Ok(inserted.into())
    }

    async fn claim_next(&self, transaction: &dyn Transaction) -> Result<Option<Job>, Error> {
        let db_tx = TransactionImpl::get_db_transaction(transaction)?;
        let now = Utc::now();

        const MAX_CLAIM_ATTEMPTS: u32 = 5;

        for _ in 0..MAX_CLAIM_ATTEMPTS {
            // SELECT the highest-priority pending job that is ready to run.
            let candidate = prelude::Jobs::find()
                .filter(jobs::Column::Status.eq(jobs::job_status_to_str(&JobStatus::Pending)))
                .filter(jobs::Column::ScheduledAt.lte(now.fixed_offset()))
                .order_by_desc(jobs::Column::Priority)
                .order_by_asc(jobs::Column::ScheduledAt)
                .order_by_asc(jobs::Column::Id)
                .one(db_tx)
                .await
                .map_err(handle_dberr)?;

            let Some(candidate) = candidate else {
                return Ok(None);
            };

            let candidate_id = candidate.id;
            let candidate_version = candidate.version;

            // Attempt to claim it with an optimistic-locking UPDATE.
            let result = prelude::Jobs::update_many()
                .col_expr(jobs::Column::Status, Expr::value(jobs::job_status_to_str(&JobStatus::Running)))
                .col_expr(jobs::Column::Attempt, Expr::col(jobs::Column::Attempt).add(1))
                .col_expr(jobs::Column::StartedAt, Expr::value(now.fixed_offset()))
                .col_expr(jobs::Column::Version, Expr::col(jobs::Column::Version).add(1))
                .col_expr(jobs::Column::UpdatedAt, Expr::value(now.fixed_offset()))
                .filter(jobs::Column::Id.eq(candidate_id))
                .filter(jobs::Column::Version.eq(candidate_version))
                .filter(jobs::Column::Status.eq(jobs::job_status_to_str(&JobStatus::Pending)))
                .exec(db_tx)
                .await
                .map_err(handle_dberr)?;

            if result.rows_affected == 1 {
                // Fetch the updated model to return accurate field values.
                let claimed = prelude::Jobs::find_by_id(candidate_id)
                    .one(db_tx)
                    .await
                    .map_err(handle_dberr)?
                    .ok_or(Error::RepositoryError(RepositoryError::NotFound))?;
                return Ok(Some(claimed.into()));
            }
            // Another worker claimed it — try the next candidate immediately.
        }

        Ok(None)
    }

    async fn complete(&self, transaction: &dyn Transaction, job: Job) -> Result<Job, Error> {
        let db_tx = TransactionImpl::get_db_transaction(transaction)?;
        let now = Utc::now();

        let result = prelude::Jobs::update_many()
            .col_expr(jobs::Column::Status, Expr::value(jobs::job_status_to_str(&JobStatus::Completed)))
            .col_expr(jobs::Column::CompletedAt, Expr::value(now.fixed_offset()))
            .col_expr(jobs::Column::Version, Expr::col(jobs::Column::Version).add(1))
            .col_expr(jobs::Column::UpdatedAt, Expr::value(now.fixed_offset()))
            .filter(jobs::Column::Id.eq(job.id))
            .filter(jobs::Column::Version.eq(job.version))
            .exec(db_tx)
            .await
            .map_err(handle_dberr)?;

        if result.rows_affected != 1 {
            return Err(Error::Infrastructure(format!(
                "complete({}) affected {} rows — version conflict or row missing",
                job.id, result.rows_affected
            )));
        }

        let updated = prelude::Jobs::find_by_id(job.id)
            .one(db_tx)
            .await
            .map_err(handle_dberr)?
            .ok_or(Error::RepositoryError(RepositoryError::NotFound))?;

        Ok(updated.into())
    }

    async fn fail(&self, transaction: &dyn Transaction, job: Job, error: String) -> Result<Job, Error> {
        let db_tx = TransactionImpl::get_db_transaction(transaction)?;
        let now = Utc::now();

        if job.attempt < job.max_attempts {
            // Reschedule with exponential backoff: 30s * 2^attempt (shift capped at 20).
            let backoff_secs = 30_i64 * (1_i64 << i64::from(std::cmp::Ord::min(job.attempt, 20)));
            let scheduled_at = now + chrono::Duration::seconds(backoff_secs);

            let result = prelude::Jobs::update_many()
                .col_expr(jobs::Column::Status, Expr::value(jobs::job_status_to_str(&JobStatus::Pending)))
                .col_expr(jobs::Column::ScheduledAt, Expr::value(scheduled_at.fixed_offset()))
                .col_expr(jobs::Column::ErrorMessage, Expr::value(error))
                .col_expr(jobs::Column::Version, Expr::col(jobs::Column::Version).add(1))
                .col_expr(jobs::Column::UpdatedAt, Expr::value(now.fixed_offset()))
                .filter(jobs::Column::Id.eq(job.id))
                .filter(jobs::Column::Version.eq(job.version))
                .exec(db_tx)
                .await
                .map_err(handle_dberr)?;

            if result.rows_affected != 1 {
                return Err(Error::Infrastructure(format!(
                    "fail({}) affected {} rows — version conflict or row missing",
                    job.id, result.rows_affected
                )));
            }
        } else {
            let result = prelude::Jobs::update_many()
                .col_expr(jobs::Column::Status, Expr::value(jobs::job_status_to_str(&JobStatus::Failed)))
                .col_expr(jobs::Column::CompletedAt, Expr::value(now.fixed_offset()))
                .col_expr(jobs::Column::ErrorMessage, Expr::value(error))
                .col_expr(jobs::Column::Version, Expr::col(jobs::Column::Version).add(1))
                .col_expr(jobs::Column::UpdatedAt, Expr::value(now.fixed_offset()))
                .filter(jobs::Column::Id.eq(job.id))
                .filter(jobs::Column::Version.eq(job.version))
                .exec(db_tx)
                .await
                .map_err(handle_dberr)?;

            if result.rows_affected != 1 {
                return Err(Error::Infrastructure(format!(
                    "fail({}) affected {} rows — version conflict or row missing",
                    job.id, result.rows_affected
                )));
            }
        }

        let updated = prelude::Jobs::find_by_id(job.id)
            .one(db_tx)
            .await
            .map_err(handle_dberr)?
            .ok_or(Error::RepositoryError(RepositoryError::NotFound))?;

        Ok(updated.into())
    }

    async fn fail_terminal(&self, transaction: &dyn Transaction, job: Job, error: String) -> Result<Job, Error> {
        let db_tx = TransactionImpl::get_db_transaction(transaction)?;
        let now = Utc::now();

        let result = prelude::Jobs::update_many()
            .col_expr(jobs::Column::Status, Expr::value(jobs::job_status_to_str(&JobStatus::Failed)))
            .col_expr(jobs::Column::CompletedAt, Expr::value(now.fixed_offset()))
            .col_expr(jobs::Column::ErrorMessage, Expr::value(error))
            .col_expr(jobs::Column::Version, Expr::col(jobs::Column::Version).add(1))
            .col_expr(jobs::Column::UpdatedAt, Expr::value(now.fixed_offset()))
            .filter(jobs::Column::Id.eq(job.id))
            .filter(jobs::Column::Version.eq(job.version))
            .exec(db_tx)
            .await
            .map_err(handle_dberr)?;

        if result.rows_affected != 1 {
            return Err(Error::Infrastructure(format!(
                "fail_terminal({}) affected {} rows — version conflict or row missing",
                job.id, result.rows_affected
            )));
        }

        let updated = prelude::Jobs::find_by_id(job.id)
            .one(db_tx)
            .await
            .map_err(handle_dberr)?
            .ok_or(Error::RepositoryError(RepositoryError::NotFound))?;

        Ok(updated.into())
    }

    async fn reset_running_to_pending(&self, transaction: &dyn Transaction) -> Result<u64, Error> {
        let db_tx = TransactionImpl::get_db_transaction(transaction)?;
        let now = Utc::now();

        let result = prelude::Jobs::update_many()
            .col_expr(jobs::Column::Status, Expr::value(jobs::job_status_to_str(&JobStatus::Pending)))
            .col_expr(jobs::Column::Version, Expr::col(jobs::Column::Version).add(1))
            .col_expr(jobs::Column::UpdatedAt, Expr::value(now.fixed_offset()))
            .filter(jobs::Column::Status.eq(jobs::job_status_to_str(&JobStatus::Running)))
            .exec(db_tx)
            .await
            .map_err(handle_dberr)?;

        Ok(result.rows_affected)
    }

    async fn count_pending_by_type(&self, transaction: &dyn Transaction, job_type: &str) -> Result<u64, Error> {
        let db_tx = TransactionImpl::get_db_transaction(transaction)?;

        let count = prelude::Jobs::find()
            .filter(jobs::Column::JobType.eq(job_type))
            .filter(jobs::Column::Status.is_in([jobs::job_status_to_str(&JobStatus::Pending), jobs::job_status_to_str(&JobStatus::Running)]))
            .count(db_tx)
            .await
            .map_err(handle_dberr)?;

        Ok(count)
    }

    async fn count_all_pending(&self, transaction: &dyn Transaction) -> Result<u64, Error> {
        let db_tx = TransactionImpl::get_db_transaction(transaction)?;

        let count = prelude::Jobs::find()
            .filter(jobs::Column::Status.is_in([jobs::job_status_to_str(&JobStatus::Pending), jobs::job_status_to_str(&JobStatus::Running)]))
            .count(db_tx)
            .await
            .map_err(handle_dberr)?;

        Ok(count)
    }

    async fn delete_old_jobs(&self, transaction: &dyn Transaction, cutoff: DateTime<Utc>) -> Result<u64, Error> {
        let db_tx = TransactionImpl::get_db_transaction(transaction)?;

        let result = prelude::Jobs::delete_many()
            .filter(jobs::Column::Status.is_in([jobs::job_status_to_str(&JobStatus::Completed), jobs::job_status_to_str(&JobStatus::Failed)]))
            .filter(jobs::Column::CompletedAt.lt(cutoff.fixed_offset()))
            .exec(db_tx)
            .await
            .map_err(handle_dberr)?;

        Ok(result.rows_affected)
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use mk_core::{jobs::JobStatus, repository::RepositoryService};
    use sea_orm::Database;

    use crate::create_repository_service;

    async fn setup() -> Arc<RepositoryService> {
        let db = Database::connect("sqlite::memory:").await.unwrap();
        create_repository_service(db).await.unwrap()
    }

    // ─── enqueue_raw ──────────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_enqueue_creates_pending_job() {
        let svc = setup().await;
        let tx = svc.repository().begin().await.unwrap();

        let payload = serde_json::json!({ "import_job_id": 42 });
        let job = svc.job_repository().enqueue_raw(&*tx, "process_import", payload.clone(), 1).await.unwrap();

        assert!(job.id > 0);
        assert_eq!(job.job_type, "process_import");
        assert_eq!(job.payload, payload);
        assert_eq!(job.status, JobStatus::Pending);
        assert_eq!(job.priority, 1);
        assert_eq!(job.attempt, 0);
        assert_eq!(job.max_attempts, 3);
    }

    // ─── enqueue_delayed ─────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_enqueue_delayed_not_visible_until_scheduled() {
        let svc = setup().await;
        let tx = svc.repository().begin().await.unwrap();

        // Enqueue with a 1-hour delay
        svc.job_repository()
            .enqueue_delayed(&*tx, "delayed.job", serde_json::json!({}), 0, chrono::Duration::hours(1))
            .await
            .unwrap();

        // Claim should return None — not yet scheduled
        let claimed = svc.job_repository().claim_next(&*tx).await.unwrap();
        assert!(claimed.is_none(), "delayed job should not be claimable until its scheduled_at");
    }

    // ─── claim_next ───────────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_claim_next_returns_none_when_empty() {
        let svc = setup().await;
        let tx = svc.repository().begin().await.unwrap();

        let result = svc.job_repository().claim_next(&*tx).await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_claim_next_claims_pending_job() {
        let svc = setup().await;
        let tx = svc.repository().begin().await.unwrap();

        svc.job_repository().enqueue_raw(&*tx, "test_job", serde_json::json!({}), 0).await.unwrap();

        let claimed = svc.job_repository().claim_next(&*tx).await.unwrap().unwrap();
        assert_eq!(claimed.status, JobStatus::Running);
        assert_eq!(claimed.attempt, 1);
        assert!(claimed.started_at.is_some());
    }

    #[tokio::test]
    async fn priority_ordered_claim() {
        use mk_core::jobs::{PRIORITY_NORMAL, PRIORITY_USER};

        let svc = setup().await;
        let tx = svc.repository().begin().await.unwrap();

        // Enqueue normal-priority FIRST so insertion order can't accidentally
        // satisfy the assertion via FIFO behavior.
        let normal = svc
            .job_repository()
            .enqueue_raw(&*tx, "normal.job", serde_json::json!({}), PRIORITY_NORMAL)
            .await
            .unwrap();
        let user = svc
            .job_repository()
            .enqueue_raw(&*tx, "user.job", serde_json::json!({}), PRIORITY_USER)
            .await
            .unwrap();

        let first = svc.job_repository().claim_next(&*tx).await.unwrap().unwrap();
        assert_eq!(
            first.id, user.id,
            "PRIORITY_USER ({PRIORITY_USER}) job should be claimed before PRIORITY_NORMAL ({PRIORITY_NORMAL})"
        );

        let second = svc.job_repository().claim_next(&*tx).await.unwrap().unwrap();
        assert_eq!(second.id, normal.id, "PRIORITY_NORMAL job should be claimed second");
    }

    #[tokio::test]
    async fn test_claim_next_skips_already_running() {
        let svc = setup().await;
        let tx = svc.repository().begin().await.unwrap();

        // Enqueue and claim one job — it becomes Running.
        let job = svc.job_repository().enqueue_raw(&*tx, "test_job", serde_json::json!({}), 0).await.unwrap();
        let _claimed = svc.job_repository().claim_next(&*tx).await.unwrap().unwrap();

        // Queue is now empty (job is running, not pending).
        let second = svc.job_repository().claim_next(&*tx).await.unwrap();
        assert!(second.is_none());

        let _ = job;
    }

    // ─── complete ─────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn version_bumps_at_each_step() {
        let svc = setup().await;
        let tx = svc.repository().begin().await.unwrap();

        let enqueued = svc.job_repository().enqueue_raw(&*tx, "test_job", serde_json::json!({}), 0).await.unwrap();
        assert_eq!(enqueued.version, 0, "freshly enqueued job should have version 0");

        let claimed = svc.job_repository().claim_next(&*tx).await.unwrap().unwrap();
        assert_eq!(claimed.id, enqueued.id);
        assert_eq!(claimed.version, 1, "claim_next should bump version to 1");

        let completed = svc.job_repository().complete(&*tx, claimed).await.unwrap();
        assert_eq!(completed.version, 2, "complete should bump version to 2");
    }

    #[tokio::test]
    async fn test_complete_sets_status_and_timestamp() {
        let svc = setup().await;
        let tx = svc.repository().begin().await.unwrap();

        svc.job_repository().enqueue_raw(&*tx, "test_job", serde_json::json!({}), 0).await.unwrap();
        let claimed = svc.job_repository().claim_next(&*tx).await.unwrap().unwrap();

        let completed = svc.job_repository().complete(&*tx, claimed).await.unwrap();
        assert_eq!(completed.status, JobStatus::Completed);
        assert!(completed.completed_at.is_some());
    }

    // ─── fail ─────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_fail_reschedules_when_retries_remain() {
        let svc = setup().await;
        let tx = svc.repository().begin().await.unwrap();

        svc.job_repository().enqueue_raw(&*tx, "test_job", serde_json::json!({}), 0).await.unwrap();
        let claimed = svc.job_repository().claim_next(&*tx).await.unwrap().unwrap();

        // attempt=1, max_attempts=3 → should reschedule
        let failed = svc.job_repository().fail(&*tx, claimed, "transient error".to_owned()).await.unwrap();
        assert_eq!(failed.status, JobStatus::Pending);
        assert_eq!(failed.error_message.as_deref(), Some("transient error"));
        // The backoff is 30s * 2^attempt — for attempt=1 (after one claim),
        // that's exactly 60s (both timestamps share a single Utc::now() in
        // the adapter, so there is no execution-time drift). Window of
        // [55s, 65s] defends against future refactors that split the now
        // variable while still pinning the spec value tightly.
        let backoff = failed.scheduled_at - failed.updated_at;
        assert!(
            backoff >= chrono::Duration::seconds(55) && backoff <= chrono::Duration::seconds(65),
            "expected backoff in [55s, 65s] for attempt=1 (spec: 60s), got {}s",
            backoff.num_seconds()
        );
    }

    #[tokio::test]
    async fn fail_terminal_marks_failed_even_with_attempts_remaining() {
        let svc = setup().await;
        let tx = svc.repository().begin().await.unwrap();

        svc.job_repository().enqueue_raw(&*tx, "test_job", serde_json::json!({}), 0).await.unwrap();
        let claimed = svc.job_repository().claim_next(&*tx).await.unwrap().unwrap();

        // attempt=1, max_attempts=3 — retries remain, but fail_terminal marks Failed
        // anyway
        assert!(claimed.attempt < claimed.max_attempts, "precondition: retries should remain");

        let failed = svc
            .job_repository()
            .fail_terminal(&*tx, claimed, "deterministic parse failure".to_owned())
            .await
            .unwrap();

        assert_eq!(failed.status, JobStatus::Failed);
        assert_eq!(failed.error_message.as_deref(), Some("deterministic parse failure"));
        assert!(failed.completed_at.is_some(), "fail_terminal must set completed_at");
    }

    #[tokio::test]
    async fn test_fail_marks_terminal_when_exhausted() {
        let svc = setup().await;
        let tx = svc.repository().begin().await.unwrap();

        svc.job_repository().enqueue_raw(&*tx, "test_job", serde_json::json!({}), 0).await.unwrap();
        let mut claimed = svc.job_repository().claim_next(&*tx).await.unwrap().unwrap();

        // Simulate attempt == max_attempts.
        claimed.attempt = claimed.max_attempts;

        let failed = svc.job_repository().fail(&*tx, claimed, "fatal error".to_owned()).await.unwrap();
        assert_eq!(failed.status, JobStatus::Failed);
        assert_eq!(failed.error_message.as_deref(), Some("fatal error"));
    }

    // ─── reset_running_to_pending ─────────────────────────────────────────────

    #[tokio::test]
    async fn test_reset_running_to_pending_returns_count() {
        let svc = setup().await;
        let tx = svc.repository().begin().await.unwrap();

        svc.job_repository().enqueue_raw(&*tx, "job_a", serde_json::json!({}), 0).await.unwrap();
        svc.job_repository().enqueue_raw(&*tx, "job_b", serde_json::json!({}), 0).await.unwrap();

        // Claim both to put them in running state.
        svc.job_repository().claim_next(&*tx).await.unwrap().unwrap();
        svc.job_repository().claim_next(&*tx).await.unwrap().unwrap();

        let reset = svc.job_repository().reset_running_to_pending(&*tx).await.unwrap();
        assert_eq!(reset, 2);

        // Both should be claimable again.
        let reclaimed = svc.job_repository().claim_next(&*tx).await.unwrap();
        assert!(reclaimed.is_some());
    }

    // ─── count_pending_by_type ────────────────────────────────────────────────

    #[tokio::test]
    async fn test_count_pending_by_type_counts_pending_and_running() {
        let svc = setup().await;
        let tx = svc.repository().begin().await.unwrap();

        // Two jobs of type "type_a", one of type "type_b".
        svc.job_repository().enqueue_raw(&*tx, "type_a", serde_json::json!({}), 0).await.unwrap();
        svc.job_repository().enqueue_raw(&*tx, "type_a", serde_json::json!({}), 0).await.unwrap();
        svc.job_repository().enqueue_raw(&*tx, "type_b", serde_json::json!({}), 0).await.unwrap();

        // Claim one type_a job (moves it to Running — should still be counted).
        svc.job_repository().claim_next(&*tx).await.unwrap().unwrap();

        let count_a = svc.job_repository().count_pending_by_type(&*tx, "type_a").await.unwrap();
        let count_b = svc.job_repository().count_pending_by_type(&*tx, "type_b").await.unwrap();

        assert_eq!(count_a, 2); // 1 running + 1 pending
        assert_eq!(count_b, 1); // 1 pending
    }

    // ─── count_all_pending ────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_count_all_pending_counts_pending_and_running() {
        let svc = setup().await;
        let tx = svc.repository().begin().await.unwrap();

        // Pending job.
        svc.job_repository().enqueue_raw(&*tx, "job_a", serde_json::json!({}), 0).await.unwrap();
        // Running job (claim it).
        svc.job_repository().enqueue_raw(&*tx, "job_b", serde_json::json!({}), 0).await.unwrap();
        svc.job_repository().claim_next(&*tx).await.unwrap().unwrap();
        // Completed job.
        svc.job_repository().enqueue_raw(&*tx, "job_c", serde_json::json!({}), 0).await.unwrap();
        let claimed = svc.job_repository().claim_next(&*tx).await.unwrap().unwrap();
        svc.job_repository().complete(&*tx, claimed).await.unwrap();

        let count = svc.job_repository().count_all_pending(&*tx).await.unwrap();
        assert_eq!(count, 2); // pending + running, not completed
    }

    // ─── delete_old_jobs ──────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_delete_old_jobs_deletes_completed_before_cutoff() {
        let svc = setup().await;
        let tx = svc.repository().begin().await.unwrap();

        // Enqueue and complete a job.
        svc.job_repository().enqueue_raw(&*tx, "old_job", serde_json::json!({}), 0).await.unwrap();
        let claimed = svc.job_repository().claim_next(&*tx).await.unwrap().unwrap();
        svc.job_repository().complete(&*tx, claimed).await.unwrap();

        // Use a cutoff in the future — everything is "old".
        let cutoff = chrono::Utc::now() + chrono::Duration::hours(1);
        let deleted = svc.job_repository().delete_old_jobs(&*tx, cutoff).await.unwrap();

        assert_eq!(deleted, 1);
    }

    #[tokio::test]
    async fn test_delete_old_jobs_does_not_delete_pending_or_running() {
        let svc = setup().await;
        let tx = svc.repository().begin().await.unwrap();

        // Pending job.
        svc.job_repository().enqueue_raw(&*tx, "pending_job", serde_json::json!({}), 0).await.unwrap();
        // Running job.
        svc.job_repository().enqueue_raw(&*tx, "running_job", serde_json::json!({}), 0).await.unwrap();
        svc.job_repository().claim_next(&*tx).await.unwrap().unwrap();

        let cutoff = chrono::Utc::now() + chrono::Duration::hours(1);
        let deleted = svc.job_repository().delete_old_jobs(&*tx, cutoff).await.unwrap();

        assert_eq!(deleted, 0);
    }

    #[tokio::test]
    async fn test_delete_old_jobs_does_not_delete_recent() {
        let svc = setup().await;
        let tx = svc.repository().begin().await.unwrap();

        // Enqueue and complete a job.
        svc.job_repository().enqueue_raw(&*tx, "recent_job", serde_json::json!({}), 0).await.unwrap();
        let claimed = svc.job_repository().claim_next(&*tx).await.unwrap().unwrap();
        svc.job_repository().complete(&*tx, claimed).await.unwrap();

        // Cutoff in the past — nothing is old enough.
        let cutoff = chrono::Utc::now() - chrono::Duration::hours(1);
        let deleted = svc.job_repository().delete_old_jobs(&*tx, cutoff).await.unwrap();

        assert_eq!(deleted, 0);
    }

    #[tokio::test]
    async fn test_delete_old_jobs_deletes_failed_before_cutoff() {
        let svc = setup().await;
        let tx = svc.repository().begin().await.unwrap();

        // Enqueue, claim, then exhaust retries to make it Failed.
        svc.job_repository().enqueue_raw(&*tx, "failed_job", serde_json::json!({}), 0).await.unwrap();
        let mut claimed = svc.job_repository().claim_next(&*tx).await.unwrap().unwrap();
        claimed.attempt = claimed.max_attempts; // simulate exhausted retries
        svc.job_repository().fail(&*tx, claimed, "fatal".to_owned()).await.unwrap();

        let cutoff = chrono::Utc::now() + chrono::Duration::hours(1);
        let deleted = svc.job_repository().delete_old_jobs(&*tx, cutoff).await.unwrap();

        assert_eq!(deleted, 1);
    }
}
