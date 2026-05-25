//! IMAP value types shared across `core::account` and (in MK-6) the IMAP
//! adapter. M3 ships only the value types; MK-6 adds `ImapPort`,
//! `ImapCredentials`, `ImapConnectionParams`, and sync-status types.

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
}
