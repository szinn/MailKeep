//! M3 stub — Task 2 of MK-3 plan fills this out with the production types.

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum TlsMode {
    Tls,
    StartTls,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ImapServerConfig {
    pub host: String,
    pub port: u16,
    pub tls: TlsMode,
}
