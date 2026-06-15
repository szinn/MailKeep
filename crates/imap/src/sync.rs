//! Per-account sync task machinery and the shared fetch routine.
//!
//! Task 2 scope (MK-7): a single one-shot pass per account — connect, SELECT
//! each enabled folder, check UIDVALIDITY, fetch everything `> last_uid` in
//! batches, ingest each message, checkpoint per batch — then idle-wait for
//! cancellation. IDLE (Task 4) and the poll timer (Task 3) build on this spine.

use std::sync::Arc;

use bytes::Bytes;
use chrono::Utc;
use futures::StreamExt;
use mk_core::{
    Error,
    account::AccountId,
    folder::{FolderId, FolderService},
    imap::{FolderConfig, ImapConnectionParams, SyncState, SyncStatus},
    ingest::{IngestRequest, IngestService},
    message::{MessageFlags, MessageService},
};
use rustls::ClientConfig;
use tokio::{sync::Mutex, task::JoinSet};
use tokio_util::sync::CancellationToken;

use crate::connect::{ImapSession, connect_and_login};

/// Number of UIDs fetched per `UID FETCH` window.
const FETCH_BATCH: u32 = 200;

/// Live handle for one tracked account's background work. Dropping it does not
/// stop the tasks; callers must `cancel` and drain `tasks` (see
/// `ImapAdapter::stop_account`).
pub(crate) struct AccountHandle {
    pub cancel: CancellationToken,
    pub tasks: JoinSet<()>,
    pub status: Arc<Mutex<SyncStatus>>,
}

/// The status a freshly-started account begins life in (mid-connect).
pub(crate) fn initial_status() -> SyncStatus {
    SyncStatus {
        state: SyncState::Connecting,
        last_sync_started_at: None,
        last_sync_finished_at: None,
        last_error: None,
        messages_ingested_session: 0,
    }
}

/// The status reported for an account that is not tracked / not running.
pub(crate) fn not_running_status() -> SyncStatus {
    SyncStatus {
        state: SyncState::NotRunning,
        last_sync_started_at: None,
        last_sync_finished_at: None,
        last_error: None,
        messages_ingested_session: 0,
    }
}

/// One-shot pass over all of an account's folders, then park until cancelled.
///
/// Task 3 splits this into a poll task; Task 4 adds the IDLE task. For now a
/// single connection syncs every folder once. The decrypted credentials live in
/// `params` and are dropped when this future ends.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_account_once(
    account_id: AccountId,
    params: ImapConnectionParams,
    ingest: Arc<dyn IngestService>,
    folders: Arc<dyn FolderService>,
    messages: Arc<dyn MessageService>,
    tls: Arc<ClientConfig>,
    status: Arc<Mutex<SyncStatus>>,
    cancel: CancellationToken,
) {
    let mut s = status.lock().await;
    s.state = SyncState::Connecting;
    s.last_sync_started_at = Some(Utc::now());
    drop(s);

    match connect_and_login(&params.server, &params.credentials, tls).await {
        Ok(mut session) => {
            for folder in &params.folders {
                if cancel.is_cancelled() {
                    break;
                }
                if let Err(e) = sync_folder(&mut session, account_id, folder, &ingest, &folders, &messages, &status).await {
                    tracing::warn!(account_id, folder = %folder.path, error = %e, "sync_folder failed");
                    let mut s = status.lock().await;
                    s.state = SyncState::Error;
                    s.last_error = Some(e.to_string());
                }
            }
            let mut s = status.lock().await;
            if s.state != SyncState::Error {
                s.state = SyncState::Idle;
            }
            s.last_sync_finished_at = Some(Utc::now());
            drop(s);
            // Best-effort logout; failure here is benign.
            if let Err(e) = session.logout().await {
                tracing::debug!(account_id, ?e, "IMAP logout failed after initial sync");
            }
        }
        Err(e) => {
            tracing::warn!(account_id, error = %e, "initial connect failed");
            let mut s = status.lock().await;
            s.state = SyncState::Error;
            s.last_error = Some(e.to_string());
        }
    }

    // Task 2 has no poll/IDLE loop yet: hold the slot until cancellation so
    // `stop_account` has something to drain and `status` stays queryable.
    cancel.cancelled().await;
}

/// SELECT a folder, check UIDVALIDITY, fetch everything `> last_uid` in
/// batches, ingest each message, checkpoint per batch. Returns the new
/// high-water UID.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn sync_folder(
    session: &mut ImapSession,
    account_id: AccountId,
    folder: &FolderConfig,
    ingest: &Arc<dyn IngestService>,
    folders: &Arc<dyn FolderService>,
    messages: &Arc<dyn MessageService>,
    status: &Arc<Mutex<SyncStatus>>,
) -> Result<u32, Error> {
    let mailbox = session
        .select(&folder.path)
        .await
        .map_err(|e| Error::Infrastructure(format!("SELECT {} failed: {e}", folder.path)))?;
    let server_uidvalidity = mailbox
        .uid_validity
        .ok_or_else(|| Error::Infrastructure(format!("server returned no UIDVALIDITY for {}", folder.path)))?;

    // UIDVALIDITY rollover: if the server's value changed, the old UIDs are
    // meaningless. Reset the cursor (full cleanup is Task 5).
    let mut last_uid = folder.last_uid;
    if let Some(known) = folder.uidvalidity
        && known != server_uidvalidity
    {
        handle_uidvalidity_change(folder.id, server_uidvalidity, folders, messages).await?;
        last_uid = 0;
    }

    status.lock().await.state = SyncState::Syncing;

    // Upper bound: nothing above `uid_next - 1` exists yet. When the server does
    // not advertise UIDNEXT, fall back to an open-ended scan that stops on the
    // first empty window past the high-water mark.
    let upper = mailbox.uid_next.map(|n| n.saturating_sub(1));
    let mut high = last_uid;
    let mut from = last_uid.saturating_add(1);

    if let Some(upper) = upper
        && upper < from
    {
        // Nothing new to fetch.
        return Ok(high);
    }

    loop {
        let to = match upper {
            Some(upper) => from.saturating_add(FETCH_BATCH - 1).min(upper),
            None => from.saturating_add(FETCH_BATCH - 1),
        };
        let range = format!("{from}:{to}");

        let mut fetched_any = false;
        {
            let mut stream = session
                .uid_fetch(&range, "(UID FLAGS INTERNALDATE BODY[])")
                .await
                .map_err(|e| Error::Infrastructure(format!("UID FETCH {range} failed: {e}")))?;
            while let Some(item) = stream.next().await {
                let fetch = item.map_err(|e| Error::Infrastructure(format!("FETCH item error: {e}")))?;
                let Some(uid) = fetch.uid else { continue };
                let Some(body) = fetch.body() else { continue };
                let raw = Bytes::copy_from_slice(body);
                let internal_date = fetch.internal_date().map_or_else(Utc::now, |d| d.with_timezone(&Utc));
                let flags = map_flags(fetch.flags());
                ingest
                    .ingest_raw(IngestRequest {
                        account_id,
                        folder_id: folder.id,
                        uid,
                        uidvalidity: server_uidvalidity,
                        internal_date,
                        flags,
                        raw_bytes: raw,
                    })
                    .await?;
                high = high.max(uid);
                fetched_any = true;
                status.lock().await.messages_ingested_session += 1;
            }
        }

        if fetched_any {
            folders.record_sync_progress(folder.id, server_uidvalidity, high, Utc::now()).await?;
        }

        match upper {
            // Known ceiling: stop once the window reaches it.
            Some(upper) if to >= upper => break,
            // Unknown ceiling: stop on the first empty window (or u32 overflow).
            None if !fetched_any || to == u32::MAX => break,
            _ => {}
        }
        from = to.saturating_add(1);
    }

    Ok(high)
}

/// UIDVALIDITY rollover handling.
///
/// Task 2 only needs detection + cursor reset so a fresh first-sync works. The
/// full safe-ordered cleanup (drop stale locations) lands in Task 5; the call
/// to `messages` is kept here to keep the signature stable for that task.
// TODO(MK-7 Task 5): implement the full transactional rollover cleanup.
pub(crate) async fn handle_uidvalidity_change(
    folder_id: FolderId,
    new_uidvalidity: u32,
    folders: &Arc<dyn FolderService>,
    messages: &Arc<dyn MessageService>,
) -> Result<(), Error> {
    let _ = messages; // wired in Task 5
    folders.record_sync_progress(folder_id, new_uidvalidity, 0, Utc::now()).await?;
    tracing::info!(
        folder_id,
        new_uidvalidity,
        "UIDVALIDITY rollover detected: cursor reset (cleanup deferred to Task 5)"
    );
    Ok(())
}

/// Map async-imap FLAGS to our `MessageFlags`. `\Recent` and friends map to the
/// matching boolean; non-standard keywords land in `custom`. `MayCreate` (`\*`)
/// is mailbox-level metadata, not a per-message flag, so it is ignored.
fn map_flags<'a>(flags: impl Iterator<Item = async_imap::types::Flag<'a>>) -> MessageFlags {
    use async_imap::types::Flag;
    let mut f = MessageFlags::default();
    for flag in flags {
        match flag {
            Flag::Seen => f.seen = true,
            Flag::Answered => f.answered = true,
            Flag::Flagged => f.flagged = true,
            Flag::Draft => f.draft = true,
            Flag::Deleted => f.deleted = true,
            Flag::Recent => f.recent = true,
            Flag::Custom(s) => f.custom.push(s.to_string()),
            // `\*` (MayCreate) is mailbox metadata, not a per-message flag.
            Flag::MayCreate => {}
        }
    }
    f
}

#[cfg(test)]
mod tests {
    use std::borrow::Cow;

    use async_imap::types::Flag;

    use super::*;

    #[test]
    fn map_flags_sets_each_system_flag() {
        let flags = [Flag::Seen, Flag::Answered, Flag::Flagged, Flag::Draft, Flag::Deleted, Flag::Recent];
        let mapped = map_flags(flags.into_iter());
        assert!(mapped.seen);
        assert!(mapped.answered);
        assert!(mapped.flagged);
        assert!(mapped.draft);
        assert!(mapped.deleted);
        assert!(mapped.recent);
        assert!(mapped.custom.is_empty());
    }

    #[test]
    fn map_flags_empty_is_all_false() {
        let mapped = map_flags(std::iter::empty());
        assert!(!mapped.seen);
        assert!(!mapped.answered);
        assert!(!mapped.flagged);
        assert!(!mapped.draft);
        assert!(!mapped.deleted);
        assert!(!mapped.recent);
        assert!(mapped.custom.is_empty());
    }

    #[test]
    fn map_flags_collects_custom_keywords() {
        let flags = [Flag::Seen, Flag::Custom(Cow::Borrowed("$Important")), Flag::Custom(Cow::Borrowed("$Phishing"))];
        let mapped = map_flags(flags.into_iter());
        assert!(mapped.seen);
        assert_eq!(mapped.custom, vec!["$Important".to_string(), "$Phishing".to_string()]);
    }

    #[test]
    fn map_flags_ignores_may_create() {
        // `\*` (MayCreate) is mailbox metadata, not a per-message flag.
        let mapped = map_flags([Flag::MayCreate, Flag::Seen].into_iter());
        assert!(mapped.seen);
        assert!(mapped.custom.is_empty());
    }
}
