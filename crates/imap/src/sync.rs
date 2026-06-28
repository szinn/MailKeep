//! Per-account sync task machinery and the shared fetch routine.
//!
//! Two long-lived tasks run per account, each on its own connection:
//! - `poll_task` (connection #2) re-SELECTs and syncs each non-idle folder
//!   every `poll_interval`.
//! - `idle_task` (connection #1) owns the single `idle_enabled` folder: it
//!   catch-up fetches, then parks in IMAP IDLE, waking on EXISTS/timeout to
//!   re-fetch and re-enter IDLE.
//!
//! Both share `sync_folder` (SELECT + UIDVALIDITY check + batched fetch +
//! checkpoint) and the `backoff_delay` exponential-backoff reconnect: retry
//! forever, set `SyncState::Error` after [`FAILURE_ERROR_THRESHOLD`]
//! consecutive failures, reset on success. Cancellation breaks both promptly
//! (IDLE sends DONE then logs out).

use std::{sync::Arc, time::Duration};

use bytes::Bytes;
use chrono::Utc;
use futures::StreamExt;
use mk_core::{
    Error,
    account::AccountId,
    folder::{FolderId, FolderService},
    imap::{FolderConfig, ImapCredentials, ImapServerConfig, SyncState, SyncStatus},
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

/// Number of consecutive failures after which a task reports
/// `SyncState::Error`. Retries continue regardless; this only surfaces
/// sustained trouble to the status reconciliation tick (and, via it, to
/// `AccountStatus`).
const FAILURE_ERROR_THRESHOLD: u32 = 5;

/// Exponential-backoff delay for the n-th consecutive failure:
/// `min(5s · 2^(n-1), 300s)`. `n` is 1-based (the first failure waits 5s).
/// This 1-based curve is intentional and gentler than the spec's `5s ·
/// 2^attempt` (first retry 5s, not 10s) — do not "correct" it back to a 0-based
/// exponent. A success resets the counter; both the poll and IDLE tasks use
/// this.
fn backoff_delay(consecutive_failures: u32) -> Duration {
    // Exponent is (failures - 1); guard the n == 0 case (no failure yet → 5s).
    let exponent = consecutive_failures.saturating_sub(1).min(6);
    let secs = 5u64.saturating_mul(1u64 << exponent);
    Duration::from_secs(secs.min(300))
}

/// Poll-sync an account's folders on its own connection.
///
/// Runs an initial pass immediately, then loops: a re-SELECT + fetch of each
/// polled folder followed by a cancellation-aware wait before the next pass.
/// On a successful pass it waits `poll_interval`; on a connect/sync failure it
/// waits `backoff_delay(consecutive_failures)` and reconnects, retrying
/// forever. After [`FAILURE_ERROR_THRESHOLD`] consecutive failures the status
/// is set to `Error`; a successful pass resets the counter and restores
/// `Idle`. The decrypted credentials live in `creds` and are dropped when this
/// future ends.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn poll_task(
    account_id: AccountId,
    server: ImapServerConfig,
    creds: ImapCredentials,
    folders_cfg: Vec<FolderConfig>,
    poll_interval: Duration,
    ingest: Arc<dyn IngestService>,
    folders: Arc<dyn FolderService>,
    messages: Arc<dyn MessageService>,
    tls: Arc<ClientConfig>,
    status: Arc<Mutex<SyncStatus>>,
    cancel: CancellationToken,
) {
    if folders_cfg.is_empty() {
        return;
    }

    let mut failures: u32 = 0;

    loop {
        {
            let mut s = status.lock().await;
            s.state = SyncState::Connecting;
            s.last_sync_started_at = Some(Utc::now());
        };

        // One full pass: connect, sync every folder, logout. Any error short-
        // circuits to a reconnect with backoff.
        let pass: Result<(), Error> = async {
            let mut session = connect_and_login(&server, &creds, tls.clone()).await?;
            for folder in &folders_cfg {
                if cancel.is_cancelled() {
                    break;
                }
                sync_folder(&mut session, account_id, folder, &ingest, &folders, &messages, &status).await?;
            }
            // Best-effort logout; failure here is benign.
            if let Err(e) = session.logout().await {
                tracing::debug!(account_id, ?e, "IMAP logout failed after poll pass");
            }
            Ok(())
        }
        .await;

        let wait = match pass {
            Ok(()) => {
                // Success resets the failure counter and restores Idle.
                failures = 0;
                let mut s = status.lock().await;
                s.state = SyncState::Idle;
                s.last_sync_finished_at = Some(Utc::now());
                poll_interval
            }
            Err(e) => {
                failures = failures.saturating_add(1);
                tracing::warn!(account_id, failures, error = %e, "poll pass failed");
                {
                    let mut s = status.lock().await;
                    if failures >= FAILURE_ERROR_THRESHOLD {
                        s.state = SyncState::Error;
                    }
                    s.last_error = Some(e.to_string());
                };
                backoff_delay(failures)
            }
        };

        // Cancellation-aware wait before the next pass.
        tokio::select! {
            () = cancel.cancelled() => break,
            () = tokio::time::sleep(wait) => {}
        }
    }
}

/// IDLE-sync the single `idle_enabled` folder on its own dedicated connection.
///
/// Connects, SELECTs the folder (re-checking UIDVALIDITY through
/// [`sync_folder`]), fetches everything `> last_uid`, then enters IMAP IDLE.
/// IDLE wakes on an `EXISTS`/`* ...` server notification or the 29-minute
/// timeout; either way the loop re-SELECTs and fetches new mail, then re-enters
/// IDLE. Cancellation while parked in IDLE breaks out cleanly: it sends `DONE`
/// (recovering the session), logs out, and returns.
///
/// Connection/protocol failures reconnect with the shared exponential backoff
/// ([`backoff_delay`]); after [`FAILURE_ERROR_THRESHOLD`] consecutive failures
/// the status is set to `Error` (retries continue), and a successful reconnect
/// resets the counter. The decrypted credentials in `creds` drop when this
/// future ends.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn idle_task(
    account_id: AccountId,
    server: ImapServerConfig,
    creds: ImapCredentials,
    folder: FolderConfig,
    ingest: Arc<dyn IngestService>,
    folders: Arc<dyn FolderService>,
    messages: Arc<dyn MessageService>,
    tls: Arc<ClientConfig>,
    status: Arc<Mutex<SyncStatus>>,
    cancel: CancellationToken,
) {
    // Longest a single IDLE may sit before we DONE and re-issue it. RFC 2177
    // advises ≤29 minutes to avoid being logged off by inactivity timeouts.
    const IDLE_TIMEOUT: Duration = Duration::from_mins(29);

    let mut failures: u32 = 0;

    loop {
        if cancel.is_cancelled() {
            break;
        }

        {
            let mut s = status.lock().await;
            s.state = SyncState::Connecting;
            s.last_sync_started_at = Some(Utc::now());
        };

        // One connection's lifetime: connect, then loop fetch→IDLE→fetch until
        // cancelled or an error forces a reconnect. Returns `Ok(())` when the
        // loop exited because of cancellation (clean shutdown); `Err` forces a
        // backoff + reconnect.
        let outcome: Result<(), Error> = async {
            let mut session = connect_and_login(&server, &creds, tls.clone()).await?;
            // Local cursor, kept accurate across re-SELECTs from sync_folder's
            // returned (high_uid, uidvalidity).
            let mut cur = folder.clone();

            loop {
                if cancel.is_cancelled() {
                    let _ = session.logout().await;
                    return Ok(());
                }

                let (high, server_uidvalidity) = sync_folder(&mut session, account_id, &cur, &ingest, &folders, &messages, &status).await?;
                cur.last_uid = high;
                cur.uidvalidity = Some(server_uidvalidity);

                // A fetch pass succeeded: reset failures and report Idle while
                // parked in IDLE.
                failures = 0;
                {
                    let mut s = status.lock().await;
                    s.state = SyncState::Idle;
                    s.last_sync_finished_at = Some(Utc::now());
                };

                // Enter IDLE. `idle()` consumes the session; `done()` returns it.
                let mut handle = session.idle();
                handle.init().await.map_err(|e| Error::Infrastructure(format!("IMAP IDLE init failed: {e}")))?;

                // `wait_with_timeout` borrows `handle` mutably (the future is
                // `+ '_`), so the future and stop source must be dropped before
                // we can call `handle.done()` (which consumes `handle`).
                let cancelled = {
                    let (idle_fut, stop) = handle.wait_with_timeout(IDLE_TIMEOUT);
                    tokio::pin!(idle_fut);
                    let cancelled = tokio::select! {
                        res = &mut idle_fut => {
                            // EXISTS / new data / keepalive-timeout: re-fetch.
                            if let Err(e) = res {
                                drop(stop);
                                return Err(Error::Infrastructure(format!("IMAP IDLE wait failed: {e}")));
                            }
                            false
                        }
                        () = cancel.cancelled() => true,
                    };
                    drop(stop);
                    cancelled
                };

                // Recover the session by sending DONE.
                session = handle.done().await.map_err(|e| Error::Infrastructure(format!("IMAP IDLE done failed: {e}")))?;

                if cancelled {
                    let _ = session.logout().await;
                    return Ok(());
                }
                // Otherwise loop: re-SELECT + fetch + re-enter IDLE.
            }
        }
        .await;

        match outcome {
            Ok(()) => break, // cancelled cleanly
            Err(e) => {
                failures = failures.saturating_add(1);
                tracing::warn!(account_id, failures, folder = %folder.path, error = %e, "IDLE connection failed");
                {
                    let mut s = status.lock().await;
                    if failures >= FAILURE_ERROR_THRESHOLD {
                        s.state = SyncState::Error;
                    }
                    s.last_error = Some(e.to_string());
                };
                tokio::select! {
                    () = cancel.cancelled() => break,
                    () = tokio::time::sleep(backoff_delay(failures)) => {}
                }
            }
        }
    }
}

/// SELECT a folder, check UIDVALIDITY, fetch everything `> last_uid` in
/// batches, ingest each message, checkpoint per batch. Returns the new
/// high-water UID together with the server's current UIDVALIDITY, so a caller
/// (the IDLE loop) can keep an accurate cursor across re-SELECTs.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn sync_folder(
    session: &mut ImapSession,
    account_id: AccountId,
    folder: &FolderConfig,
    ingest: &Arc<dyn IngestService>,
    folders: &Arc<dyn FolderService>,
    messages: &Arc<dyn MessageService>,
    status: &Arc<Mutex<SyncStatus>>,
) -> Result<(u32, u32), Error> {
    let mailbox = session
        .select(&folder.path)
        .await
        .map_err(|e| Error::Infrastructure(format!("SELECT {} failed: {e}", folder.path)))?;
    let server_uidvalidity = mailbox
        .uid_validity
        .ok_or_else(|| Error::Infrastructure(format!("server returned no UIDVALIDITY for {}", folder.path)))?;

    // UIDVALIDITY rollover: if the server's value changed, the old UIDs are
    // meaningless. `handle_uidvalidity_change` resets the cursor and drops the
    // stale message locations; restart this pass from UID 1.
    let mut last_uid = folder.last_uid;
    if let Some(known) = folder.uidvalidity
        && known != server_uidvalidity
    {
        handle_uidvalidity_change(folder.id, server_uidvalidity, folders, messages).await?;
        last_uid = 0;
    }

    let start_uid = last_uid;
    let mut ingested: u32 = 0;

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
        tracing::debug!(account_id, folder = %folder.path, last_uid = high, "folder sync: no new messages");
        return Ok((high, server_uidvalidity));
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
                ingested += 1;
                status.lock().await.messages_ingested_session += 1;
                tracing::debug!(account_id, folder = %folder.path, uid, "ingested message");
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

    if ingested > 0 {
        tracing::info!(
            account_id,
            folder = %folder.path,
            new = ingested,
            last_uid_from = start_uid,
            last_uid_to = high,
            "folder sync: new messages"
        );
    } else {
        tracing::debug!(account_id, folder = %folder.path, last_uid = high, "folder sync: no new messages");
    }
    Ok((high, server_uidvalidity))
}

/// UIDVALIDITY rollover handling: a safe-ordered cleanup invoked from every
/// SELECT path (poll and IDLE) whenever the server's UIDVALIDITY differs from
/// the one we last recorded for this folder.
///
/// **Ordering matters.** We reset the folder cursor *first*
/// (`record_sync_progress(.., new_uidvalidity, 0, ..)`), then drop the
/// now-stale message locations. If the process crashes between the two steps,
/// `last_uid` is already `0`, so the next pass simply re-fetches everything
/// from UID 1 and re-upserts locations idempotently — no harm done. The reverse
/// order would be unsafe: a crash after deleting locations but before resetting
/// the cursor would leave `last_uid` high with the locations gone, so those
/// messages would be skipped on the next pass and the archive view would lose
/// their locations.
///
/// Only `message_locations` rows are dropped; the `Message`/attachment rows are
/// untouched, and re-ingest is idempotent. The end-to-end rollover behaviour is
/// proven against greenmail in Task 7.
pub(crate) async fn handle_uidvalidity_change(
    folder_id: FolderId,
    new_uidvalidity: u32,
    folders: &Arc<dyn FolderService>,
    messages: &Arc<dyn MessageService>,
) -> Result<(), Error> {
    // Safe order: reset the cursor first (a crash here just re-fetches all and
    // re-upserts locations idempotently), then drop the stale locations.
    folders.record_sync_progress(folder_id, new_uidvalidity, 0, Utc::now()).await?;
    let dropped = messages.delete_locations_for_folder(folder_id).await?;
    tracing::info!(
        folder_id,
        new_uidvalidity,
        dropped,
        "UIDVALIDITY rollover: reset cursor, dropped stale locations"
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

    #[test]
    fn backoff_delay_follows_5s_doubling_with_300s_cap() {
        // n == 0 (no failure yet) is treated as the base 5s delay.
        assert_eq!(backoff_delay(0), Duration::from_secs(5));
        // 1-based: 5s · 2^(n-1) → 5, 10, 20, 40, 80, 160, then capped at 300.
        assert_eq!(backoff_delay(1), Duration::from_secs(5));
        assert_eq!(backoff_delay(2), Duration::from_secs(10));
        assert_eq!(backoff_delay(3), Duration::from_secs(20));
        assert_eq!(backoff_delay(4), Duration::from_secs(40));
        assert_eq!(backoff_delay(5), Duration::from_secs(80));
        assert_eq!(backoff_delay(6), Duration::from_secs(160));
        // 5 · 2^6 = 320 → capped at 300.
        assert_eq!(backoff_delay(7), Duration::from_secs(300));
        // Far beyond the exponent cap stays at 300 (no overflow/panic).
        assert_eq!(backoff_delay(100), Duration::from_secs(300));
        assert_eq!(backoff_delay(u32::MAX), Duration::from_secs(300));
    }
}
