//! [`ImapPort`] implementation. MK-6 ships `test_connection` and
//! `list_folders`; the lifecycle methods return [`Error::Unimplemented`] until
//! MK-7.

use std::sync::Arc;

use async_imap::types::{Name, NameAttribute};
use async_trait::async_trait;
use futures::StreamExt;
use mk_core::{
    Error,
    account::AccountId,
    imap::{ImapConnectionParams, ImapCredentials, ImapPort, ImapServerConfig, RemoteFolder, SyncStatus, special_use_from_attributes},
};
use rustls::ClientConfig;

use crate::connect::{connect_and_login, production_client_config};

/// Production IMAP adapter over async-imap + tokio-rustls.
#[derive(Debug)]
pub struct ImapAdapter {
    tls_config: Arc<ClientConfig>,
}

impl ImapAdapter {
    /// Build the adapter with the production rustls config (webpki roots).
    #[must_use]
    pub fn new() -> Self {
        Self {
            tls_config: production_client_config(),
        }
    }

    /// Test-only constructor that injects a custom rustls config (e.g. one that
    /// trusts a self-signed greenmail cert). Gated behind `test-support` so
    /// production builds cannot accidentally weaken trust. Used by the
    /// greenmail integration tests in the integration-tests crate.
    #[cfg(feature = "test-support")]
    #[must_use]
    pub fn with_tls_config(tls_config: Arc<ClientConfig>) -> Self {
        Self { tls_config }
    }
}

impl Default for ImapAdapter {
    fn default() -> Self {
        Self::new()
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

    async fn start_account(&self, _account_id: AccountId, _params: ImapConnectionParams) -> Result<(), Error> {
        Err(Error::Unimplemented("ImapAdapter::start_account (MK-7)"))
    }

    async fn stop_account(&self, _account_id: AccountId) -> Result<(), Error> {
        Err(Error::Unimplemented("ImapAdapter::stop_account (MK-7)"))
    }

    async fn status(&self, _account_id: AccountId) -> Result<SyncStatus, Error> {
        Err(Error::Unimplemented("ImapAdapter::status (MK-7)"))
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
}
