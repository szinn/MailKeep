//! The crate-level IMAP subsystem (tokio-graceful-shutdown). It owns the
//! lifecycle of all enabled accounts: on startup it kicks off
//! `start_all_enabled`, then runs a periodic `reconcile_statuses` tick to keep
//! the persisted `AccountStatus` in sync with each account's live health, and
//! on shutdown it calls `stop_all`.

use std::{sync::Arc, time::Duration};

use mk_core::{Error, imap::ImapAccountService};
use tokio_graceful_shutdown::{IntoSubsystem, SubsystemHandle};

/// How often the subsystem reconciles persisted `AccountStatus` with live
/// `SyncState` (wiring decision 3).
const RECONCILE_INTERVAL: Duration = Duration::from_secs(30);

/// Top-level subsystem for the IMAP adapter (one per crate, per the
/// subsystem-composition convention). Started by `main.rs` as `"Imap"`.
pub struct ImapSubsystem {
    svc: Arc<dyn ImapAccountService>,
}

#[must_use]
pub fn create_imap_subsystem(svc: Arc<dyn ImapAccountService>) -> ImapSubsystem {
    ImapSubsystem { svc }
}

impl IntoSubsystem<Error> for ImapSubsystem {
    async fn run(self, subsys: &mut SubsystemHandle) -> Result<(), Error> {
        tracing::info!("ImapSubsystem starting...");

        if let Err(e) = self.svc.start_all_enabled().await {
            tracing::error!(error = %e, "start_all_enabled failed");
        }

        loop {
            tokio::select! {
                () = subsys.on_shutdown_requested() => break,
                () = tokio::time::sleep(RECONCILE_INTERVAL) => {
                    if let Err(e) = self.svc.reconcile_statuses().await {
                        tracing::warn!(error = %e, "reconcile_statuses failed");
                    }
                }
            }
        }

        let _ = self.svc.stop_all().await;
        tracing::info!("ImapSubsystem stopped");
        Ok(())
    }
}
