use bytes::Bytes;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::{
    account::AccountId,
    folder::FolderId,
    jobs::{Enqueueable, JobId, PRIORITY_INGEST},
    message::MessageFlags,
    types::ContentHash,
};

/// Request to ingest a single raw RFC822 message.
///
/// Shape is IMAP-friendly even though M5 feeds it from fixtures — M7 supplies
/// these fields from the live sync loop.
#[derive(Debug, Clone)]
pub struct IngestRequest {
    pub account_id: AccountId,
    pub folder_id: FolderId,
    pub uid: u32,
    pub uidvalidity: u32,
    pub internal_date: DateTime<Utc>,
    pub flags: MessageFlags,
    /// Raw .eml plaintext (zero-copy). Up to ~25 MB in memory is acceptable for
    /// v1.
    pub raw_bytes: Bytes,
}

/// Outcome of `ingest_raw`: the content hash of the stored raw bytes and the
/// id of the enqueued parse job.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IngestResult {
    pub content_hash: ContentHash,
    pub job_id: JobId,
}

/// Background job payload: parse a stored raw message into DB rows + attachment
/// blobs. Produced by `IngestService`, consumed by `crates/parser`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParseMessageJob {
    pub account_id: AccountId,
    pub folder_id: FolderId,
    pub uid: u32,
    pub uidvalidity: u32,
    pub content_hash: ContentHash,
    pub internal_date: DateTime<Utc>,
    pub flags: MessageFlags,
}

impl Enqueueable for ParseMessageJob {
    const JOB_TYPE: &'static str = "parse_message";
    const DEFAULT_PRIORITY: i16 = PRIORITY_INGEST;
}
