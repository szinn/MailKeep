//! Full-text search adapter for MailKeep, backed by [Tantivy].
//!
//! This crate owns the Tantivy dependency and nothing in the domain depends on
//! it: it depends on `mk-core` only (for the [`mk_core::message::Message`] it
//! maps into search documents), never the reverse.
//!
//! Contents at this stage (schema + index plumbing only):
//! - [`schema`]: the versioned Tantivy schema ([`SCHEMA_VERSION`]), the
//!   resolved [`Fields`] handles, and the `Message → document` mapping
//!   ([`to_document`]).
//! - [`index`]: [`SearchIndex`] open/create, reader/writer handles, and the
//!   on-disk schema-version sidecar ([`needs_rebuild`], [`write_version`]).
//! - [`service`]: [`TantivySearchService`], the
//!   `mk_core::search::SearchService` adapter — query compilation, per-user
//!   account scoping, the `folder:` DB post-filter, and `delete_account`.
//! - [`indexer`]: [`SearchSubsystem`], the background write side — startup
//!   schema-version rebuild, an idempotent drain of `indexed = false` rows, and
//!   decrypt-on-demand body extraction.
//!
//! The public wiring surface is the three factories below:
//! [`open_search_index`] (shared handle), [`create_search_service`] (read
//! side), and [`create_search_subsystem`] (write side). `mailkeep`'s `main.rs`
//! calls them.
//!
//! [Tantivy]: https://github.com/quickwit-oss/tantivy

pub mod index;
pub mod indexer;
pub mod schema;
pub mod service;

use std::{path::Path, sync::Arc};

pub use index::{SearchIndex, SearchIndexError, needs_rebuild, read_version, write_version};
pub use indexer::SearchSubsystem;
use mk_core::{CoreServices, repository::RepositoryService, search::SearchService};
pub use schema::{Fields, SCHEMA_VERSION, build_schema, to_document};
pub use service::TantivySearchService;

/// Open (or create) the on-disk Tantivy index at `dir`, returning a shared
/// handle. Both the read service and the indexer subsystem take a clone of the
/// same [`Arc`], so they share the single writer the index owns.
///
/// # Errors
///
/// Returns [`SearchIndexError`] if the directory or index cannot be opened.
pub fn open_search_index(dir: &Path) -> Result<Arc<SearchIndex>, SearchIndexError> {
    Ok(Arc::new(SearchIndex::open_or_create(dir)?))
}

/// Build the read-side [`SearchService`] adapter over the shared index.
#[must_use]
pub fn create_search_service(index: Arc<SearchIndex>, repository_service: Arc<RepositoryService>) -> Arc<dyn SearchService> {
    Arc::new(TantivySearchService::new(index, repository_service))
}

/// Build the write-side [`SearchSubsystem`], capturing the repository and raw
/// storage services from `core` plus the shared index.
#[must_use]
pub fn create_search_subsystem(core: &Arc<CoreServices>, index: Arc<SearchIndex>) -> SearchSubsystem {
    SearchSubsystem::new(index, core.repository_service.clone(), core.raw_storage_service.clone())
}
