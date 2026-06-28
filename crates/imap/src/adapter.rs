//! [`ImapPort`] implementation. MK-6 ships `test_connection` and
//! `list_folders`; MK-7 (this task) adds the per-account sync lifecycle:
//! `start_account` spawns a background task that connects, SELECTs, and
//! fetches; `stop_account` cancels and drains it; `status` reports live health.

use std::{collections::HashMap, sync::Arc, time::Duration};

use async_imap::types::{Name, NameAttribute};
use async_trait::async_trait;
use futures::StreamExt;
use mk_core::{
    Error,
    account::AccountId,
    folder::FolderService,
    imap::{ImapConnectionParams, ImapCredentials, ImapPort, ImapServerConfig, RemoteFolder, SyncStatus, special_use_from_attributes},
    ingest::IngestService,
    message::MessageService,
};
use rustls::ClientConfig;
use tokio::{sync::Mutex, task::JoinSet};
use tokio_util::sync::CancellationToken;

use crate::{
    connect::{connect_and_login, production_client_config},
    sync::{AccountHandle, idle_task, initial_status, not_running_status, poll_task},
};

/// Production IMAP adapter over async-imap + tokio-rustls.
///
/// Owns the long-lived per-account sync tasks and calls back into the injected
/// core services. It deliberately holds service trait objects (not `database`,
/// `crypto`, or `storage`) so the hexagonal boundary is preserved.
pub struct ImapAdapter {
    tls_config: Arc<ClientConfig>,
    ingest_service: Arc<dyn IngestService>,
    folder_service: Arc<dyn FolderService>,
    message_service: Arc<dyn MessageService>,
    // Interval between poll passes over the non-IDLE folders.
    poll_interval: Duration,
    tracked: Mutex<HashMap<AccountId, AccountHandle>>,
}

impl ImapAdapter {
    /// Build the adapter with the production rustls config (webpki roots) and
    /// the injected core services.
    #[must_use]
    pub fn new(
        ingest_service: Arc<dyn IngestService>,
        folder_service: Arc<dyn FolderService>,
        message_service: Arc<dyn MessageService>,
        poll_interval: Duration,
    ) -> Self {
        Self {
            tls_config: production_client_config(),
            ingest_service,
            folder_service,
            message_service,
            poll_interval,
            tracked: Mutex::new(HashMap::new()),
        }
    }

    /// Probe-only constructor for the `mailkeep imap` diagnostic command, which
    /// uses only `test_connection`/`list_folders` (no sync services). Backed by
    /// nop services that panic if a sync method is reached. Never wire this
    /// into production sync.
    #[must_use]
    pub fn probe() -> Self {
        let (ingest_service, folder_service, message_service) = crate::probe::nop_services();
        Self {
            tls_config: production_client_config(),
            ingest_service,
            folder_service,
            message_service,
            poll_interval: Duration::from_secs(300),
            tracked: Mutex::new(HashMap::new()),
        }
    }

    /// Test-only constructor that injects a custom rustls config (e.g. one that
    /// trusts a self-signed greenmail cert) plus the sync services. Gated
    /// behind `test-support` so production builds cannot accidentally
    /// weaken trust. Used by the full-sync greenmail integration tests
    /// (MK-7 Task 7).
    #[cfg(feature = "test-support")]
    #[must_use]
    pub fn with_tls_config(
        ingest_service: Arc<dyn IngestService>,
        folder_service: Arc<dyn FolderService>,
        message_service: Arc<dyn MessageService>,
        poll_interval: Duration,
        tls_config: Arc<ClientConfig>,
    ) -> Self {
        Self {
            tls_config,
            ingest_service,
            folder_service,
            message_service,
            poll_interval,
            tracked: Mutex::new(HashMap::new()),
        }
    }

    /// Test-only probe constructor that injects a custom rustls config but uses
    /// nop sync services. For connectivity/LIST tests that never drive the sync
    /// lifecycle (e.g. the greenmail harness probe).
    #[cfg(feature = "test-support")]
    #[must_use]
    pub fn probe_with_tls_config(tls_config: Arc<ClientConfig>) -> Self {
        let (ingest_service, folder_service, message_service) = crate::probe::nop_services();
        Self {
            tls_config,
            ingest_service,
            folder_service,
            message_service,
            poll_interval: Duration::from_secs(300),
            tracked: Mutex::new(HashMap::new()),
        }
    }
}

/// Normalize a single LIST attribute to the lowercased, backslash-stripped name
/// that `core::special_use_from_attributes` expects (e.g. `\Sent` → `"sent"`,
/// `\Noselect` → `"noselect"`). async-imap parses RFC 6154 special-use flags
/// into typed variants; unknown flags arrive as `Extension`.
fn normalize_attribute(attr: &NameAttribute<'_>) -> String {
    match attr {
        NameAttribute::NoInferiors => "noinferiors".to_string(),
        NameAttribute::NoSelect => "noselect".to_string(),
        NameAttribute::Marked => "marked".to_string(),
        NameAttribute::Unmarked => "unmarked".to_string(),
        NameAttribute::All => "all".to_string(),
        NameAttribute::Archive => "archive".to_string(),
        NameAttribute::Drafts => "drafts".to_string(),
        NameAttribute::Flagged => "flagged".to_string(),
        NameAttribute::Junk => "junk".to_string(),
        NameAttribute::Sent => "sent".to_string(),
        NameAttribute::Trash => "trash".to_string(),
        // Extension flags carry the raw token, sometimes still backslash-prefixed
        // (e.g. `\HasChildren`); strip the leading backslash and lowercase.
        NameAttribute::Extension(name) => name.trim_start_matches('\\').to_ascii_lowercase(),
        // `NameAttribute` is `#[non_exhaustive]`; treat any future flag as an
        // uncategorized structural attribute (its Debug rendering, normalized).
        other => format!("{other:?}").trim_start_matches('\\').to_ascii_lowercase(),
    }
}

/// Map an async-imap `Name` (one LIST entry) to our `RemoteFolder`.
fn remote_folder_from_name(name: &Name) -> RemoteFolder {
    let attrs: Vec<String> = name.attributes().iter().map(normalize_attribute).collect();
    let path = name.name().to_string();

    RemoteFolder {
        special_use: special_use_from_attributes(&path, &attrs),
        no_select: attrs.iter().any(|a| a == "noselect"),
        has_children: attrs.iter().any(|a| a == "haschildren"),
        delimiter: name.delimiter().map(str::to_string),
        path,
    }
}

#[async_trait]
impl ImapPort for ImapAdapter {
    async fn test_connection(&self, server: &ImapServerConfig, creds: &ImapCredentials) -> Result<(), Error> {
        let mut session = connect_and_login(server, creds, self.tls_config.clone()).await?;
        // Best-effort logout; a successful login already proves connectivity.
        if let Err(e) = session.logout().await {
            tracing::debug!(?e, "IMAP logout failed after test_connection");
        }
        Ok(())
    }

    async fn list_folders(&self, server: &ImapServerConfig, creds: &ImapCredentials) -> Result<Vec<RemoteFolder>, Error> {
        let mut session = connect_and_login(server, creds, self.tls_config.clone()).await?;

        let mut out = Vec::new();
        {
            let mut stream = session
                .list(Some(""), Some("*"))
                .await
                .map_err(|e| Error::Infrastructure(format!("IMAP LIST failed: {e}")))?;

            while let Some(item) = stream.next().await {
                let name = item.map_err(|e| Error::Infrastructure(format!("IMAP LIST item error: {e}")))?;
                tracing::debug!(folder = name.name(), attributes = ?name.attributes(), "IMAP LIST entry");
                out.push(remote_folder_from_name(&name));
            }
        }

        if let Err(e) = session.logout().await {
            tracing::debug!(?e, "IMAP logout failed after list_folders");
        }
        Ok(out)
    }

    async fn start_account(&self, account_id: AccountId, params: ImapConnectionParams) -> Result<(), Error> {
        // Restart-idempotent: tear down any existing task before spawning a new one.
        let _ = self.stop_account(account_id).await;

        let cancel = CancellationToken::new();
        let status = Arc::new(Mutex::new(initial_status()));
        let mut tasks = JoinSet::new();

        // Partition the enabled folders into the single `idle_enabled` folder
        // (owned by the dedicated IDLE task on connection #1) and the rest (the
        // poll set on connection #2). At most one folder is IDLE-enabled; if a
        // server somehow advertises more, the extras fall back to the poll set
        // so they still get synced.
        let (idle_folders, poll_folders): (Vec<_>, Vec<_>) = params.folders.iter().cloned().partition(|f| f.idle_enabled);
        let mut idle_folders = idle_folders.into_iter();
        let idle_folder = idle_folders.next();
        // Any surplus IDLE-enabled folders join the poll set.
        let poll_folders: Vec<_> = poll_folders.into_iter().chain(idle_folders).collect();

        // Spawn the dedicated IDLE task for the single idle folder, if any.
        if let Some(idle_folder) = idle_folder {
            let ingest = self.ingest_service.clone();
            let folders = self.folder_service.clone();
            let messages = self.message_service.clone();
            let tls = self.tls_config.clone();
            let server = params.server.clone();
            let creds = params.credentials.clone();
            let status_for_task = status.clone();
            let cancel_for_task = cancel.clone();
            tasks.spawn(async move {
                idle_task(
                    account_id,
                    server,
                    creds,
                    idle_folder,
                    ingest,
                    folders,
                    messages,
                    tls,
                    status_for_task,
                    cancel_for_task,
                )
                .await;
            });
        }

        // Spawn the poll task for the non-idle folders, if any. `poll_task`
        // returns immediately on an empty set, but skip the spawn to avoid
        // spawning a task that immediately returns.
        if !poll_folders.is_empty() {
            let ingest = self.ingest_service.clone();
            let folders = self.folder_service.clone();
            let messages = self.message_service.clone();
            let tls = self.tls_config.clone();
            let poll_interval = self.poll_interval;
            let server = params.server.clone();
            let creds = params.credentials.clone();
            let status_for_task = status.clone();
            let cancel_for_task = cancel.clone();
            tasks.spawn(async move {
                poll_task(
                    account_id,
                    server,
                    creds,
                    poll_folders,
                    poll_interval,
                    ingest,
                    folders,
                    messages,
                    tls,
                    status_for_task,
                    cancel_for_task,
                )
                .await;
            });
        }

        self.tracked.lock().await.insert(account_id, AccountHandle { cancel, tasks, status });
        Ok(())
    }

    async fn stop_account(&self, account_id: AccountId) -> Result<(), Error> {
        let handle = self.tracked.lock().await.remove(&account_id);
        if let Some(mut handle) = handle {
            handle.cancel.cancel();
            while handle.tasks.join_next().await.is_some() {}
        }
        Ok(())
    }

    async fn status(&self, account_id: AccountId) -> Result<SyncStatus, Error> {
        match self.tracked.lock().await.get(&account_id) {
            Some(handle) => Ok(handle.status.lock().await.clone()),
            None => Ok(not_running_status()),
        }
    }

    async fn tracked_accounts(&self) -> Vec<AccountId> {
        self.tracked.lock().await.keys().copied().collect()
    }
}

#[cfg(test)]
mod tests {
    use std::borrow::Cow;

    use mk_core::folder::SpecialUse;

    use super::*;

    #[test]
    fn normalize_typed_special_use_attributes() {
        assert_eq!(normalize_attribute(&NameAttribute::Sent), "sent");
        assert_eq!(normalize_attribute(&NameAttribute::Drafts), "drafts");
        assert_eq!(normalize_attribute(&NameAttribute::Trash), "trash");
        assert_eq!(normalize_attribute(&NameAttribute::Archive), "archive");
        assert_eq!(normalize_attribute(&NameAttribute::Junk), "junk");
        assert_eq!(normalize_attribute(&NameAttribute::All), "all");
    }

    #[test]
    fn normalize_structural_attributes() {
        assert_eq!(normalize_attribute(&NameAttribute::NoSelect), "noselect");
        assert_eq!(normalize_attribute(&NameAttribute::NoInferiors), "noinferiors");
        assert_eq!(normalize_attribute(&NameAttribute::Marked), "marked");
        assert_eq!(normalize_attribute(&NameAttribute::Unmarked), "unmarked");
    }

    #[test]
    fn normalize_extension_strips_backslash_and_lowercases() {
        // Servers that don't have first-class parsing land structural flags such
        // as \HasChildren in Extension; ensure they normalize for our checks.
        assert_eq!(normalize_attribute(&NameAttribute::Extension(Cow::Borrowed("\\HasChildren"))), "haschildren");
        assert_eq!(
            normalize_attribute(&NameAttribute::Extension(Cow::Borrowed("\\HasNoChildren"))),
            "hasnochildren"
        );
        assert_eq!(normalize_attribute(&NameAttribute::Extension(Cow::Borrowed("\\Sent"))), "sent");
    }

    #[test]
    fn sent_extension_flows_through_core_special_use() {
        // A \Sent arriving as Extension must still resolve to SpecialUse::Sent
        // once normalized and handed to the core helper.
        let attrs = vec![normalize_attribute(&NameAttribute::Extension(Cow::Borrowed("\\Sent")))];
        assert_eq!(special_use_from_attributes("Sent", &attrs), Some(SpecialUse::Sent));
    }

    #[test]
    fn typed_sent_flows_through_core_special_use() {
        let attrs = vec![normalize_attribute(&NameAttribute::Sent)];
        assert_eq!(special_use_from_attributes("[Gmail]/Sent Mail", &attrs), Some(SpecialUse::Sent));
    }

    #[tokio::test]
    async fn fresh_adapter_tracks_no_accounts() {
        let adapter = ImapAdapter::probe();
        assert!(adapter.tracked_accounts().await.is_empty());
    }
}
