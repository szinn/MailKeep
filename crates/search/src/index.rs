//! On-disk Tantivy index lifecycle: open/create, reader/writer handles, and the
//! schema-version sidecar that drives rebuilds.

use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use tantivy::{
    Index, IndexReader, IndexWriter, ReloadPolicy, TantivyError,
    directory::{MmapDirectory, error::OpenDirectoryError},
};

use crate::schema::{Fields, SCHEMA_VERSION, build_schema, en_stem_analyzer};

/// Name of the plain-text sidecar file holding the on-disk [`SCHEMA_VERSION`].
const VERSION_FILE: &str = "schema_version";

/// Arena memory budget for the single shared [`IndexWriter`], in bytes.
///
/// Tantivy permits exactly one writer per index directory (a filesystem lock),
/// so the budget is fixed at construction rather than per-call.
const WRITER_MEM_BUDGET_BYTES: usize = 15_000_000;

/// Startup / index-management errors for the search adapter.
///
/// These never cross a `mk_core` port; they surface only from the factory and
/// index-management calls, where the binary converts them via `?`.
#[derive(thiserror::Error, Debug)]
pub enum SearchIndexError {
    #[error("creating search index directory {dir}: {source}")]
    CreateDir { dir: PathBuf, source: std::io::Error },

    #[error("opening search index directory {dir}: {source}")]
    OpenDir { dir: PathBuf, source: OpenDirectoryError },

    #[error("opening search index at {dir}: {source}")]
    OpenIndex { dir: PathBuf, source: TantivyError },

    #[error("creating index writer: {source}")]
    Writer { source: TantivyError },

    #[error("creating index reader: {source}")]
    Reader { source: TantivyError },

    #[error("writing schema version sidecar {path}: {source}")]
    WriteVersion { path: PathBuf, source: std::io::Error },
}

/// A handle to the on-disk Tantivy index plus its resolved [`Fields`].
///
/// Owns the **single shared [`IndexWriter`]** for the index directory. Tantivy
/// allows only one writer per directory (a lock), and multiple subsystems need
/// to write — the indexer's drain loop and the search service's
/// `delete_account`. Both obtain the same writer via [`SearchIndex::writer`]
/// and serialize their commits through its [`Mutex`]. Readers, by contrast, are
/// cheap and created on demand via [`SearchIndex::reader`].
///
/// The `en_stem` tokenizer is registered on the underlying [`Index`] at open
/// time, so any reader/writer/query built from it tokenizes the stemmed fields
/// correctly.
pub struct SearchIndex {
    index: Index,
    fields: Fields,
    dir: PathBuf,
    /// The one writer permitted for this directory, shared across subsystems.
    writer: Arc<Mutex<IndexWriter>>,
}

impl std::fmt::Debug for SearchIndex {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SearchIndex").field("dir", &self.dir).finish_non_exhaustive()
    }
}

impl SearchIndex {
    /// Open the index at `dir`, creating the directory and a fresh index with
    /// the current schema when none exists. Registers the `en_stem` tokenizer.
    ///
    /// # Errors
    ///
    /// Returns [`SearchIndexError`] if the directory cannot be created/opened
    /// or the index cannot be opened or created.
    pub fn open_or_create(dir: &Path) -> Result<Self, SearchIndexError> {
        std::fs::create_dir_all(dir).map_err(|source| SearchIndexError::CreateDir {
            dir: dir.to_path_buf(),
            source,
        })?;

        let (schema, fields) = build_schema();
        let mmap = MmapDirectory::open(dir).map_err(|source| SearchIndexError::OpenDir {
            dir: dir.to_path_buf(),
            source,
        })?;
        let index = Index::open_or_create(mmap, schema).map_err(|source| SearchIndexError::OpenIndex {
            dir: dir.to_path_buf(),
            source,
        })?;

        index.tokenizers().register(crate::schema::EN_STEM, en_stem_analyzer());

        // Construct the one-and-only writer up front so the directory lock is
        // held for the life of this handle and shared by every writer.
        let writer = index.writer(WRITER_MEM_BUDGET_BYTES).map_err(|source| SearchIndexError::Writer { source })?;

        Ok(Self {
            index,
            fields,
            dir: dir.to_path_buf(),
            writer: Arc::new(Mutex::new(writer)),
        })
    }

    /// Resolved field handles for this index.
    #[must_use]
    pub fn fields(&self) -> &Fields {
        &self.fields
    }

    /// The underlying Tantivy index (for building query parsers, etc.).
    #[must_use]
    pub fn index(&self) -> &Index {
        &self.index
    }

    /// The directory backing this index.
    #[must_use]
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Build a near-real-time reader that reloads shortly after each commit.
    ///
    /// # Errors
    ///
    /// Returns [`SearchIndexError::Reader`] if the reader cannot be
    /// constructed.
    pub fn reader(&self) -> Result<IndexReader, SearchIndexError> {
        self.index
            .reader_builder()
            .reload_policy(ReloadPolicy::OnCommitWithDelay)
            .try_into()
            .map_err(|source| SearchIndexError::Reader { source })
    }

    /// A clone of the handle to the single shared [`IndexWriter`].
    ///
    /// Lock the returned [`Mutex`] to add/delete documents and commit. The
    /// writer is created once in [`SearchIndex::open_or_create`]; every caller
    /// shares it, so concurrent writers cannot deadlock on the directory lock.
    /// Keep critical sections short and never hold the guard across an
    /// `.await`.
    #[must_use]
    pub fn writer(&self) -> Arc<Mutex<IndexWriter>> {
        Arc::clone(&self.writer)
    }
}

/// Path to the schema-version sidecar within `dir`.
fn version_path(dir: &Path) -> PathBuf {
    dir.join(VERSION_FILE)
}

/// Read the persisted schema version from `<dir>/schema_version`, if present
/// and parseable.
#[must_use]
pub fn read_version(dir: &Path) -> Option<u32> {
    std::fs::read_to_string(version_path(dir)).ok()?.trim().parse().ok()
}

/// Persist the current [`SCHEMA_VERSION`] to `<dir>/schema_version`.
///
/// # Errors
///
/// Returns [`SearchIndexError::WriteVersion`] if the sidecar cannot be written.
pub fn write_version(dir: &Path) -> Result<(), SearchIndexError> {
    let path = version_path(dir);
    std::fs::write(&path, SCHEMA_VERSION.to_string()).map_err(|source| SearchIndexError::WriteVersion { path, source })
}

/// Whether the index at `dir` must be rebuilt from source.
///
/// True when the sidecar is missing, unreadable, unparseable, or does not equal
/// the current [`SCHEMA_VERSION`].
#[must_use]
pub fn needs_rebuild(dir: &Path) -> bool {
    read_version(dir) != Some(SCHEMA_VERSION)
}

#[cfg(test)]
mod tests {
    use mk_core::{
        message::{Message, MessageBuilder, MessageToken},
        types::{ContentHash, EmailAddress},
    };
    use tantivy::{TantivyDocument, collector::TopDocs, query::QueryParser, schema::Value};
    use tempfile::TempDir;

    use super::*;
    use crate::schema::{EN_STEM, to_document};

    fn message(id: u64, subject: &str, snippet: &str) -> Message {
        MessageBuilder::default()
            .id(id)
            .version(1u64)
            .token(MessageToken::generate())
            .account_id(1u64)
            .rfc822_message_id(format!("<{id}@example.com>"))
            .content_hash(ContentHash::compute(subject.as_bytes()))
            .subject(Some(subject.to_string()))
            .from_address(EmailAddress::new("alice@example.com").unwrap())
            .snippet(snippet.to_string())
            .size_bytes(1)
            .build()
            .unwrap()
    }

    #[test]
    fn open_or_create_fresh_then_reopen() {
        let dir = TempDir::new().unwrap();
        let first = SearchIndex::open_or_create(dir.path());
        assert!(first.is_ok(), "fresh open failed: {first:?}");
        // The eager shared writer holds the directory's writer lock, so only one
        // live `SearchIndex` may exist per directory. Drop the first before
        // reopening — the realistic single-handle lifecycle.
        drop(first);
        let second = SearchIndex::open_or_create(dir.path());
        assert!(second.is_ok(), "reopen after drop failed: {second:?}");
    }

    #[test]
    fn open_or_create_registers_en_stem_tokenizer() {
        let dir = TempDir::new().unwrap();
        let si = SearchIndex::open_or_create(dir.path()).unwrap();
        assert!(si.index().tokenizers().get(EN_STEM).is_some());
    }

    #[test]
    fn version_sidecar_round_trips() {
        let dir = TempDir::new().unwrap();
        assert_eq!(read_version(dir.path()), None);
        write_version(dir.path()).unwrap();
        assert_eq!(read_version(dir.path()), Some(SCHEMA_VERSION));
    }

    #[test]
    fn needs_rebuild_true_when_missing_and_mismatched() {
        let dir = TempDir::new().unwrap();
        // Missing sidecar.
        assert!(needs_rebuild(dir.path()));
        // Mismatched version.
        std::fs::write(version_path(dir.path()), (SCHEMA_VERSION + 1).to_string()).unwrap();
        assert!(needs_rebuild(dir.path()));
        // Garbage that fails to parse.
        std::fs::write(version_path(dir.path()), "not-a-number").unwrap();
        assert!(needs_rebuild(dir.path()));
    }

    #[test]
    fn needs_rebuild_false_when_equal() {
        let dir = TempDir::new().unwrap();
        write_version(dir.path()).unwrap();
        assert!(!needs_rebuild(dir.path()));
    }

    #[test]
    fn index_a_document_and_search_it_back() {
        let dir = TempDir::new().unwrap();
        let si = SearchIndex::open_or_create(dir.path()).unwrap();
        let fields = *si.fields();

        let writer_handle = si.writer();
        let mut writer = writer_handle.lock().unwrap();
        let msg = message(42, "Quarterly running report", "preview text");
        writer.add_document(to_document(&fields, &msg, "the body content")).unwrap();
        writer.commit().unwrap();
        drop(writer);

        let reader = si.reader().unwrap();
        let searcher = reader.searcher();
        let parser = QueryParser::for_index(si.index(), vec![fields.subject]);
        let query = parser.parse_query("running").unwrap();

        let hits = searcher.search(&query, &TopDocs::with_limit(10).order_by_score()).unwrap();
        assert_eq!(hits.len(), 1);

        let (_score, addr) = hits[0];
        let doc: TantivyDocument = searcher.doc(addr).unwrap();
        assert_eq!(doc.get_first(fields.message_id).and_then(|v| v.as_u64()), Some(42));
        assert_eq!(doc.get_first(fields.snippet).and_then(|v| v.as_str()), Some("preview text"));
    }

    #[test]
    fn committed_documents_persist_across_reopen() {
        let dir = TempDir::new().unwrap();

        // Session 1: index and commit a document, then drop everything.
        let si = SearchIndex::open_or_create(dir.path()).unwrap();
        let fields = *si.fields();
        let writer_handle = si.writer();
        let mut writer = writer_handle.lock().unwrap();
        let msg = message(77, "Persisted quarterly report", "durable preview");
        writer.add_document(to_document(&fields, &msg, "on-disk body")).unwrap();
        writer.commit().unwrap();
        drop(writer);
        drop(writer_handle);
        drop(si);

        // Session 2: a fresh open of the same directory must see the committed doc.
        let si = SearchIndex::open_or_create(dir.path()).unwrap();
        let fields = *si.fields();
        let reader = si.reader().unwrap();
        let searcher = reader.searcher();
        let parser = QueryParser::for_index(si.index(), vec![fields.subject]);
        let query = parser.parse_query("quarterly").unwrap();

        let hits = searcher.search(&query, &TopDocs::with_limit(10).order_by_score()).unwrap();
        assert_eq!(hits.len(), 1, "committed doc did not survive drop + reopen");

        let (_score, addr) = hits[0];
        let doc: TantivyDocument = searcher.doc(addr).unwrap();
        assert_eq!(doc.get_first(fields.message_id).and_then(|v| v.as_u64()), Some(77));
        assert_eq!(doc.get_first(fields.snippet).and_then(|v| v.as_str()), Some("durable preview"));
    }

    #[test]
    fn stemmer_matches_word_variants() {
        let dir = TempDir::new().unwrap();
        let si = SearchIndex::open_or_create(dir.path()).unwrap();
        let fields = *si.fields();

        let writer_handle = si.writer();
        let mut writer = writer_handle.lock().unwrap();
        let msg = message(1, "irrelevant", "snip");
        writer.add_document(to_document(&fields, &msg, "running quickly through the park")).unwrap();
        writer.commit().unwrap();
        drop(writer);

        let reader = si.reader().unwrap();
        let searcher = reader.searcher();
        let parser = QueryParser::for_index(si.index(), vec![fields.body]);
        // "run" stems the same as the indexed "running".
        let query = parser.parse_query("run").unwrap();

        let hits = searcher.search(&query, &TopDocs::with_limit(10).order_by_score()).unwrap();
        assert_eq!(hits.len(), 1);
    }
}
