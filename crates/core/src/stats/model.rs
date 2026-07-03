use chrono::{DateTime, Utc};

/// Aggregate archive statistics for one user, rolled up across all their
/// accounts. All counts are totals; `storage_bytes` is the physical on-disk
/// footprint (raw message blobs + separately-stored extracted attachment
/// blobs). `last_synced_at` is the most recent folder sync across the user's
/// accounts, or `None` if nothing has synced yet.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ArchiveStats {
    pub message_count: u64,
    pub attachment_count: u64,
    pub storage_bytes: u64,
    pub account_count: u64,
    pub last_synced_at: Option<DateTime<Utc>>,
}
