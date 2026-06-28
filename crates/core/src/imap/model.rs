use chrono::{DateTime, Utc};
use secrecy::SecretString;

use crate::folder::{FolderId, SpecialUse};

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum TlsMode {
    /// Implicit TLS — connect on a TLS-only port (typically 993).
    Tls,
    /// Opportunistic TLS — connect plaintext, upgrade via STARTTLS.
    StartTls,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ImapServerConfig {
    pub host: String,
    pub port: u16,
    pub tls: TlsMode,
}

/// IMAP login credentials. `password` is held in a zeroizing `SecretString` and
/// is never emitted by `Debug`.
#[derive(Clone)]
pub struct ImapCredentials {
    pub username: String,
    pub password: SecretString,
}

impl std::fmt::Debug for ImapCredentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ImapCredentials")
            .field("username", &self.username)
            .field("password", &"<redacted>")
            .finish()
    }
}

/// Per-account connection parameters. Defined now; the `folders` list is only
/// exercised by the MK-7 sync task.
#[derive(Debug, Clone)]
pub struct ImapConnectionParams {
    pub server: ImapServerConfig,
    pub credentials: ImapCredentials,
    pub folders: Vec<FolderConfig>,
    /// Human-readable account label for log lines (see `Account::log_label`).
    pub account_label: String,
}

/// One enabled folder's sync cursor. Defined now, exercised in MK-7.
#[derive(Debug, Clone)]
pub struct FolderConfig {
    pub id: FolderId,
    pub path: String,
    pub idle_enabled: bool,
    pub uidvalidity: Option<u32>,
    pub last_uid: u32,
}

/// A folder as reported by the remote server's `LIST` response.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteFolder {
    pub path: String,
    pub special_use: Option<SpecialUse>,
    pub has_children: bool,
    pub no_select: bool,
    /// Server-reported hierarchy delimiter for this entry (e.g. `/` or `.`).
    /// `None` for flat mailboxes. Used by the frontend to build the folder
    /// tree.
    pub delimiter: Option<String>,
}

/// Live sync status. Defined now, exercised in MK-7.
#[derive(Debug, Clone)]
pub struct SyncStatus {
    pub state: SyncState,
    pub last_sync_started_at: Option<DateTime<Utc>>,
    pub last_sync_finished_at: Option<DateTime<Utc>>,
    pub last_error: Option<String>,
    pub messages_ingested_session: u64,
}

#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncState {
    NotRunning,
    Connecting,
    Idle,
    Syncing,
    Error,
}

/// Maps a folder's `LIST` attributes (RFC 6154 special-use + RFC 3501
/// INBOX-by-name) to our `SpecialUse`. `mailbox_path` is the folder path;
/// `attributes` are the backslash-stripped, lowercased attribute names (e.g.
/// `["sent", "hasnochildren"]`). Returns `None` for folders we do not
/// categorize (incl. `\Flagged`, `\Noselect`).
#[must_use]
pub fn special_use_from_attributes(mailbox_path: &str, attributes: &[String]) -> Option<SpecialUse> {
    if mailbox_path.eq_ignore_ascii_case("inbox") {
        return Some(SpecialUse::Inbox);
    }
    for attr in attributes {
        match attr.as_str() {
            "sent" => return Some(SpecialUse::Sent),
            "drafts" => return Some(SpecialUse::Drafts),
            "trash" => return Some(SpecialUse::Trash),
            "archive" => return Some(SpecialUse::Archive),
            "junk" => return Some(SpecialUse::Junk),
            "all" => return Some(SpecialUse::All),
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tls_mode_serde_round_trip_tls() {
        let json = serde_json::to_string(&TlsMode::Tls).unwrap();
        let back: TlsMode = serde_json::from_str(&json).unwrap();
        assert_eq!(back, TlsMode::Tls);
    }

    #[test]
    fn tls_mode_serde_round_trip_starttls() {
        let json = serde_json::to_string(&TlsMode::StartTls).unwrap();
        let back: TlsMode = serde_json::from_str(&json).unwrap();
        assert_eq!(back, TlsMode::StartTls);
    }

    #[test]
    fn server_config_serde_round_trip() {
        let cfg = ImapServerConfig {
            host: "imap.example.com".into(),
            port: 993,
            tls: TlsMode::Tls,
        };
        let json = serde_json::to_string(&cfg).unwrap();
        let back: ImapServerConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(back, cfg);
    }

    #[test]
    fn inbox_matched_by_name_case_insensitive() {
        assert_eq!(special_use_from_attributes("INBOX", &[]), Some(SpecialUse::Inbox));
        assert_eq!(special_use_from_attributes("inbox", &[]), Some(SpecialUse::Inbox));
        assert_eq!(special_use_from_attributes("InBox", &[]), Some(SpecialUse::Inbox));
    }

    #[test]
    fn special_use_flags_map() {
        let cases = [
            ("Sent", "sent", SpecialUse::Sent),
            ("Drafts", "drafts", SpecialUse::Drafts),
            ("Trash", "trash", SpecialUse::Trash),
            ("Archive", "archive", SpecialUse::Archive),
            ("Junk", "junk", SpecialUse::Junk),
            ("All Mail", "all", SpecialUse::All),
        ];
        for (path, attr, expected) in cases {
            assert_eq!(
                special_use_from_attributes(path, &[attr.to_string()]),
                Some(expected),
                "path={path} attr={attr}"
            );
        }
    }

    #[test]
    fn uncategorized_attributes_return_none() {
        assert_eq!(special_use_from_attributes("Flagged", &["flagged".into()]), None);
        assert_eq!(special_use_from_attributes("Noselect", &["noselect".into()]), None);
        assert_eq!(special_use_from_attributes("Custom", &["hasnochildren".into()]), None);
        assert_eq!(special_use_from_attributes("Custom", &[]), None);
    }

    #[test]
    fn credentials_debug_redacts_password() {
        let creds = ImapCredentials {
            username: "alice".into(),
            password: SecretString::from("hunter2"),
        };
        let rendered = format!("{creds:?}");
        assert!(rendered.contains("alice"));
        assert!(rendered.contains("<redacted>"));
        assert!(!rendered.contains("hunter2"));
    }
}
