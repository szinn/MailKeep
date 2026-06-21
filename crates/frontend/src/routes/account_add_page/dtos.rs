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
}

#[cfg(feature = "server")]
pub(crate) use mapping::*;

#[cfg(feature = "server")]
mod mapping {
    use std::str::FromStr;

    use mk_core::{
        account::Account,
        folder::SpecialUse,
        imap::{ImapServerConfig, RemoteFolder, TlsMode},
    };

    use super::*;

    pub(crate) fn tls_from_string(s: &str) -> TlsMode {
        match s {
            "StartTls" => TlsMode::StartTls,
            _ => TlsMode::Tls, // default Implicit TLS (spec §5)
        }
    }

    pub(crate) fn tls_to_string(t: TlsMode) -> String {
        match t {
            TlsMode::Tls => "Tls".to_string(),
            TlsMode::StartTls => "StartTls".to_string(),
        }
    }

    pub(crate) fn special_use_to_string(s: Option<SpecialUse>) -> Option<String> {
        s.map(|v| v.as_str().to_string())
    }

    pub(crate) fn special_use_from_string(s: &Option<String>) -> Option<SpecialUse> {
        s.as_deref().and_then(|v| SpecialUse::from_str(v).ok())
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

    pub(crate) fn account_to_summary(a: &Account) -> AccountSummaryDto {
        AccountSummaryDto {
            token: a.token.to_string(),
            display_name: a.display_name.clone(),
            email: a.email_address.as_str().to_string(),
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
    fn tls_round_trips() {
        assert_eq!(tls_from_string("Tls"), TlsMode::Tls);
        assert_eq!(tls_from_string("StartTls"), TlsMode::StartTls);
        assert_eq!(tls_from_string("garbage"), TlsMode::Tls);
        assert_eq!(tls_to_string(TlsMode::Tls), "Tls");
        assert_eq!(tls_to_string(TlsMode::StartTls), "StartTls");
    }

    #[test]
    fn special_use_round_trips_and_rejects_unknown() {
        assert_eq!(special_use_to_string(Some(SpecialUse::Inbox)), Some("inbox".into()));
        assert_eq!(special_use_to_string(None), None);
        assert_eq!(special_use_from_string(&Some("sent".into())), Some(SpecialUse::Sent));
        assert_eq!(special_use_from_string(&Some("nope".into())), None);
        assert_eq!(special_use_from_string(&None), None);
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
