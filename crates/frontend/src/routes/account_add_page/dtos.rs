use serde::{Deserialize, Serialize};

/// IMAP connection details as entered in the form. Crosses the wire; the server
/// maps `tls` (string) back to `TlsMode` and pairs it with the email as
/// username.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct ServerConfigDto {
    pub host: String,
    pub port: u16,
    /// "Tls" | "StartTls"
    pub tls: String,
}

/// One folder from `list_folders`, rendered by the picker.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct RemoteFolderDto {
    pub path: String,
    /// Lowercase SpecialUse string ("inbox"...) or None.
    pub special_use: Option<String>,
    pub has_children: bool,
    pub no_select: bool,
    pub delimiter: Option<String>,
}

/// A folder the user chose to sync. `no_select` carried so the server can apply
/// the safety filter (spec §6) as the authoritative gate.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct NewFolderDto {
    pub path: String,
    pub special_use: Option<String>,
    pub no_select: bool,
}

/// Final create payload — password re-sent (no server-side state between probe
/// and create, spec §7).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct NewAccountDto {
    pub display_name: String,
    pub email: String,
    pub server: ServerConfigDto,
    pub password: String,
    pub folders: Vec<NewFolderDto>,
}

/// Confirmation returned after create; also the home-list row shape.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct AccountSummaryDto {
    pub token: String,
    pub display_name: String,
    pub email: String,
    /// AccountStatus wire form ("PendingFirstSync" | "Syncing" | "Idle" |
    /// "Error" | "Disabled").
    pub status: String,
    /// Pre-formatted relative time ("2m ago") or None when never synced.
    pub last_synced: Option<String>,
    /// Present only when status == "Error".
    pub last_error: Option<String>,
}

/// One folder of an existing account, for the Edit Folders modal.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct AccountFolderDto {
    pub token: String,
    pub path: String,
    /// Lowercase special-use string ("inbox"…) or None.
    pub special_use: Option<String>,
    pub enabled: bool,
}

/// One folder's desired enabled state, sent from the modal on Save.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct FolderEnabledDto {
    pub token: String,
    pub enabled: bool,
}

#[cfg(feature = "server")]
pub(crate) use mapping::*;

#[cfg(feature = "server")]
mod mapping {
    use std::str::FromStr;

    use chrono::{DateTime, Utc};
    use mk_core::{
        account::Account,
        folder::{Folder, SpecialUse},
        imap::{ImapServerConfig, RemoteFolder, TlsMode},
    };

    use super::{AccountFolderDto, AccountSummaryDto, RemoteFolderDto, ServerConfigDto};

    /// Whole-unit relative time. Pure (testable) — caller supplies `now`.
    pub(crate) fn relative_time(from: DateTime<Utc>, now: DateTime<Utc>) -> String {
        let secs = (now - from).num_seconds();
        if secs < 60 {
            return "just now".to_string();
        }
        let mins = secs / 60;
        if mins < 60 {
            return format!("{mins}m ago");
        }
        let hours = mins / 60;
        if hours < 24 {
            return format!("{hours}h ago");
        }
        format!("{}d ago", hours / 24)
    }

    pub(crate) fn folder_to_account_folder(f: &Folder) -> AccountFolderDto {
        AccountFolderDto {
            token: f.token.to_string(),
            path: f.path.clone(),
            special_use: f.special_use.map(|s| s.as_str().to_string()),
            enabled: f.enabled,
        }
    }

    pub(crate) fn tls_from_string(s: &str) -> TlsMode {
        match s {
            "StartTls" => TlsMode::StartTls,
            _ => TlsMode::Tls, // default Implicit TLS (spec §5)
        }
    }

    pub(crate) fn special_use_to_string(s: Option<SpecialUse>) -> Option<String> {
        s.map(|v| v.as_str().to_string())
    }

    pub(crate) fn special_use_from_string(s: Option<&String>) -> Option<SpecialUse> {
        s.and_then(|v| SpecialUse::from_str(v).ok())
    }

    pub(crate) fn server_config_from_dto(d: &ServerConfigDto) -> ImapServerConfig {
        ImapServerConfig {
            host: d.host.clone(),
            port: d.port,
            tls: tls_from_string(&d.tls),
        }
    }

    pub(crate) fn remote_folder_to_dto(f: RemoteFolder) -> RemoteFolderDto {
        RemoteFolderDto {
            path: f.path,
            special_use: special_use_to_string(f.special_use),
            has_children: f.has_children,
            no_select: f.no_select,
            delimiter: f.delimiter,
        }
    }

    pub(crate) fn account_to_summary(a: &Account, last_synced_at: Option<DateTime<Utc>>) -> AccountSummaryDto {
        AccountSummaryDto {
            token: a.token.to_string(),
            display_name: a.display_name.clone(),
            email: a.email_address.as_str().to_string(),
            status: a.status.as_str().to_string(),
            last_synced: last_synced_at.map(|t| relative_time(t, Utc::now())),
            last_error: if a.status == mk_core::account::AccountStatus::Error {
                a.last_error.clone()
            } else {
                None
            },
        }
    }
}

#[cfg(all(test, feature = "server"))]
mod tests {
    use mk_core::{
        folder::SpecialUse,
        imap::{RemoteFolder, TlsMode},
    };

    use super::*;

    #[test]
    fn relative_time_buckets() {
        use chrono::{Duration, Utc};
        let now = Utc::now();
        assert_eq!(relative_time(now - Duration::seconds(5), now), "just now");
        assert_eq!(relative_time(now - Duration::minutes(2), now), "2m ago");
        assert_eq!(relative_time(now - Duration::hours(3), now), "3h ago");
        assert_eq!(relative_time(now - Duration::days(2), now), "2d ago");
    }

    #[test]
    fn tls_round_trips() {
        assert_eq!(tls_from_string("Tls"), TlsMode::Tls);
        assert_eq!(tls_from_string("StartTls"), TlsMode::StartTls);
        assert_eq!(tls_from_string("garbage"), TlsMode::Tls);
    }

    #[test]
    fn special_use_round_trips_and_rejects_unknown() {
        assert_eq!(special_use_to_string(Some(SpecialUse::Inbox)), Some("inbox".into()));
        assert_eq!(special_use_to_string(None), None);
        assert_eq!(special_use_from_string(Some(&"sent".to_string())), Some(SpecialUse::Sent));
        assert_eq!(special_use_from_string(Some(&"nope".to_string())), None);
        assert_eq!(special_use_from_string(None), None);
    }

    #[test]
    fn remote_folder_maps_to_dto() {
        let f = RemoteFolder {
            path: "[Gmail]/All Mail".into(),
            special_use: Some(SpecialUse::All),
            has_children: false,
            no_select: false,
            delimiter: Some("/".into()),
        };
        let dto = remote_folder_to_dto(f);
        assert_eq!(dto.path, "[Gmail]/All Mail");
        assert_eq!(dto.special_use, Some("all".into()));
        assert_eq!(dto.delimiter, Some("/".into()));
    }
}
