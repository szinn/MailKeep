//! IMAP driven port: value types, the `ImapPort` trait, and
//! `ImapAccountService`. M3 shipped only `TlsMode`/`ImapServerConfig`; MK-6
//! adds the probe surface. The adapter lives in `crates/imap`. MK-7 fills the
//! lifecycle methods.

mod model;
mod port;
mod service;

pub use model::{
    FolderConfig, ImapConnectionParams, ImapCredentials, ImapServerConfig, RemoteFolder, SyncState, SyncStatus, TlsMode, special_use_from_attributes,
};
pub use port::ImapPort;
#[cfg(any(test, feature = "test-support"))]
pub use port::MockImapPort;
pub use service::ImapAccountService;
#[cfg(any(test, feature = "test-support"))]
pub use service::MockImapAccountService;
