use std::sync::Arc;

use async_trait::async_trait;

use crate::{
    Error,
    ingest::model::{IngestRequest, IngestResult, ParseMessageJob},
    jobs::{JobService, JobServiceExt},
    storage::RawStorageService,
};

/// Synchronous front door for the ingest pipeline.
#[async_trait]
pub trait IngestService: Send + Sync {
    /// Store the raw bytes (idempotent, encrypted) then enqueue a parse job.
    async fn ingest_raw(&self, request: IngestRequest) -> Result<IngestResult, Error>;
}

pub(crate) struct IngestServiceImpl {
    raw_storage_service: Arc<dyn RawStorageService>,
    job_service: Arc<dyn JobService>,
}

impl IngestServiceImpl {
    pub(crate) fn new(raw_storage_service: Arc<dyn RawStorageService>, job_service: Arc<dyn JobService>) -> Self {
        Self {
            raw_storage_service,
            job_service,
        }
    }
}

#[async_trait]
impl IngestService for IngestServiceImpl {
    async fn ingest_raw(&self, request: IngestRequest) -> Result<IngestResult, Error> {
        let content_hash = self.raw_storage_service.put_if_absent(request.account_id, &request.raw_bytes).await?;

        let job = ParseMessageJob {
            account_id: request.account_id,
            folder_id: request.folder_id,
            uid: request.uid,
            uidvalidity: request.uidvalidity,
            content_hash,
            internal_date: request.internal_date,
            flags: request.flags,
        };
        let job_id = self.job_service.enqueue(&job).await?;

        Ok(IngestResult { content_hash, job_id })
    }
}

#[cfg(test)]
mod tests {
    use bytes::Bytes;
    use chrono::Utc;
    use mockall::Sequence;

    use super::*;
    use crate::{jobs::service::MockJobService, message::MessageFlags, storage::MockRawStorageService, types::ContentHash};

    #[tokio::test]
    async fn ingest_raw_stores_then_enqueues_in_order() {
        let hash = ContentHash::compute(b"raw message bytes");
        let mut seq = Sequence::new();

        let mut raw = MockRawStorageService::new();
        raw.expect_put_if_absent()
            .times(1)
            .in_sequence(&mut seq)
            .withf(|account_id, plaintext| *account_id == 7 && plaintext == b"raw message bytes")
            .returning(move |_, _| Box::pin(async move { Ok(hash) }));

        let mut jobs = MockJobService::new();
        jobs.expect_enqueue_raw()
            .times(1)
            .in_sequence(&mut seq)
            .withf(move |job_type, payload, priority| {
                job_type == "parse_message"
                    && *priority == crate::jobs::PRIORITY_INGEST
                    && payload.get("content_hash").and_then(|v| v.as_str()) == Some(hash.as_hex().as_str())
                    && payload.get("account_id").and_then(serde_json::Value::as_u64) == Some(7)
                    && payload.get("folder_id").and_then(serde_json::Value::as_u64) == Some(3)
            })
            .returning(|_, _, _| Box::pin(async { Ok(99_i64) }));

        let svc = IngestServiceImpl::new(Arc::new(raw), Arc::new(jobs));
        let result = svc
            .ingest_raw(IngestRequest {
                account_id: 7,
                folder_id: 3,
                uid: 100,
                uidvalidity: 1000,
                internal_date: Utc::now(),
                flags: MessageFlags::default(),
                raw_bytes: Bytes::from_static(b"raw message bytes"),
            })
            .await
            .unwrap();

        assert_eq!(result.content_hash, hash);
        assert_eq!(result.job_id, 99);
    }
}
