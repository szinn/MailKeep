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
//!
//! The indexer subsystem is added in a later task.
//!
//! [Tantivy]: https://github.com/quickwit-oss/tantivy

pub mod index;
pub mod schema;
pub mod service;

pub use index::{SearchIndex, SearchIndexError, needs_rebuild, read_version, write_version};
pub use schema::{Fields, SCHEMA_VERSION, build_schema, to_document};
pub use service::TantivySearchService;
