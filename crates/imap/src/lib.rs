//! MailKeep IMAP adapter, implementing `mk_core::imap::ImapPort` over
//! async-imap and tokio-rustls. MK-6 ships `test_connection` and
//! `list_folders`; MK-7 adds the per-account sync lifecycle (connect, SELECT,
//! batched fetch + checkpoint) driven by the injected core services.

mod adapter;
mod connect;
mod probe;
mod subsystem;
mod sync;

use std::{sync::Arc, time::Duration};

pub use adapter::ImapAdapter;
pub use connect::production_client_config;
use mk_core::imap::{ImapPort, ImapPortFactory};
pub use subsystem::{ImapSubsystem, create_imap_subsystem};

/// Factory consumed by `mk_core`'s external-services wiring. The
/// `poll_interval` is baked into the closure here (per the MK-7 wiring
/// decision) so `main.rs` stays adapter-agnostic and the value never lands on
/// `ExternalServices`.
#[must_use]
pub fn create_imap_port_factory(poll_interval: Duration) -> ImapPortFactory {
    Box::new(move |ingest, folders, messages| Arc::new(ImapAdapter::new(ingest, folders, messages, poll_interval)) as Arc<dyn ImapPort>)
}
