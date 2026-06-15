//! MailKeep IMAP adapter, implementing `mk_core::imap::ImapPort` over
//! async-imap and tokio-rustls. MK-6 ships `test_connection` and
//! `list_folders`, while the lifecycle methods return `Error::Unimplemented`
//! until MK-7.

mod adapter;
mod connect;

use std::sync::Arc;

pub use adapter::ImapAdapter;
pub use connect::production_client_config;
use mk_core::imap::{ImapPort, ImapPortFactory};

/// Factory consumed by `mk_core`'s external-services wiring. MK-6 ignores the
/// ingest/folder/message services (no sync traffic yet); they are wired in
/// MK-7.
#[must_use]
pub fn create_imap_port_factory() -> ImapPortFactory {
    Box::new(|_ingest, _folders, _messages| Arc::new(ImapAdapter::new()) as Arc<dyn ImapPort>)
}
