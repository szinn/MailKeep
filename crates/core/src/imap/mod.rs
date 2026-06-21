//! IMAP driven port: value types, the `ImapPort` trait, and
//! `ImapAccountService`. M3 shipped only `TlsMode`/`ImapServerConfig`; MK-6
//! adds the probe surface. The adapter lives in `crates/imap`. MK-7 fills the
//! lifecycle methods.

mod model;
mod port;
mod service;

use std::sync::Arc;

use crate::{folder::FolderService, ingest::IngestService, message::MessageService};

/// Factory that constructs the IMAP adapter inside `CoreServices::new`, given
/// the core services it depends on. Keeps `core` adapter-agnostic: the concrete
/// `ImapPort` is built by the binary's wiring (or a test nop) without `core`
/// depending on the adapter crate. Consumed exactly once.
///
/// Takes `IngestService` (raw-message ingestion), `FolderService` (sync-cursor
/// persistence), and `MessageService` (location cleanup on UIDVALIDITY
/// rollover).
pub type ImapPortFactory = Box<dyn FnOnce(Arc<dyn IngestService>, Arc<dyn FolderService>, Arc<dyn MessageService>) -> Arc<dyn ImapPort> + Send>;

pub use model::{
    FolderConfig, ImapConnectionParams, ImapCredentials, ImapServerConfig, RemoteFolder, SyncState, SyncStatus, TlsMode, special_use_from_attributes,
};
pub use port::ImapPort;
#[cfg(any(test, feature = "test-support"))]
pub use port::MockImapPort;
#[cfg(any(test, feature = "test-support"))]
pub use service::MockImapAccountService;
pub use service::{ImapAccountService, ImapAccountServiceImpl, create_imap_account_service};
