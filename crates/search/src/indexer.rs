//! The search indexer subsystem (tokio-graceful-shutdown).
//!
//! This is the **write side** of the search feature. It converges the Tantivy
//! index toward the database:
//!
//! - **Startup reconcile/rebuild.** When the on-disk schema version no longer
//!   matches [`crate::SCHEMA_VERSION`] (see [`needs_rebuild`]), the whole index
//!   is cleared, every message row is reset to `indexed = false`, and the
//!   version sidecar is rewritten. Otherwise the rows still flagged `indexed =
//!   false` are simply the pending backlog.
//! - **Idempotent drain.** [`SearchSubsystem::drain_once`] pulls a batch of
//!   `indexed = false` messages, decrypts each blob on demand, extracts its
//!   text body, and writes one document per message. Each write is a
//!   `delete_term(message_id)` immediately followed by `add_document`, so
//!   re-indexing a message (e.g. after a crash between `commit` and
//!   `mark_indexed`) never leaves a duplicate. Only after the batch commits are
//!   the rows flagged indexed.
//! - **Poison-pill tolerance vs transient faults.** A message whose blob is
//!   permanently missing ([`Error::BlobNotFound`]) or whose bytes will not
//!   parse is logged and *still* marked indexed (with no document added), so a
//!   single bad row can never wedge the drain loop forever. Any *other* storage
//!   error (assumed transient — I/O, decryption, a locked backend) is **not**
//!   marked: [`SearchSubsystem::drain_once`] returns `Err`, the batch is
//!   retried on the next tick, and no message is silently dropped from the
//!   index. A blob that is genuinely gone resurfaces as `BlobNotFound` on retry
//!   and is skipped correctly, so retrying strictly dominates skipping.
//!
//! # Writer discipline
//!
//! There is exactly one [`tantivy::IndexWriter`] per index directory — the
//! shared one owned by [`SearchIndex`]. Every critical section that holds its
//! `std::sync::Mutex` guard is fully synchronous: the guard is dropped before
//! any `.await`, so the subsystem future stays `Send` and callers never
//! deadlock on the directory lock.

use std::{borrow::Cow, sync::Arc, time::Duration};

use mail_parser::MessageParser;
use mk_core::{
    Error,
    message::{Message, MessageId},
    repository::{RepositoryService, read_only_transaction, transaction},
    storage::RawStorageService,
};
use tantivy::Term;
use tokio_graceful_shutdown::{IntoSubsystem, SubsystemHandle};

use crate::{
    SearchIndex,
    index::{needs_rebuild, write_version},
    schema::to_document,
};

/// Number of unindexed messages pulled and committed per drain batch. Bounds
/// the memory held (decrypted blobs + documents) between commits.
const BATCH: u32 = 128;

/// How often the subsystem re-drains once the backlog is empty. A crude poll;
/// wiring the job service's `wake_notify` to nudge the drain immediately is a
/// documented follow-up, not needed for v1.
const POLL_INTERVAL: Duration = Duration::from_secs(2);

/// Background indexer that keeps the Tantivy index in sync with the database.
pub struct SearchSubsystem {
    index: Arc<SearchIndex>,
    repository_service: Arc<RepositoryService>,
    raw_storage_service: Arc<dyn RawStorageService>,
}

impl std::fmt::Debug for SearchSubsystem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SearchSubsystem").field("index", &self.index).finish_non_exhaustive()
    }
}

impl SearchSubsystem {
    /// Construct the subsystem over the shared index and core services.
    #[must_use]
    pub fn new(index: Arc<SearchIndex>, repository_service: Arc<RepositoryService>, raw_storage_service: Arc<dyn RawStorageService>) -> Self {
        Self {
            index,
            repository_service,
            raw_storage_service,
        }
    }

    /// Startup reconcile: rebuild the index from source when the on-disk schema
    /// version is missing or stale.
    ///
    /// On a version mismatch this clears every document, resets every message
    /// row to `indexed = false`, and rewrites the version sidecar — leaving the
    /// entire corpus queued for the drain. When the version already matches it
    /// is a no-op; the pending `indexed = false` rows are the only work.
    async fn reconcile_on_startup(&self) -> Result<(), Error> {
        if !needs_rebuild(self.index.dir()) {
            return Ok(());
        }
        tracing::info!("search schema version changed; rebuilding index from source");

        // Clear the whole index (synchronous writer critical section).
        self.clear_index()?;

        // Re-queue every message for indexing.
        let repo = self.repository_service.repository().clone();
        let msg_repo = self.repository_service.message_repository().clone();
        let reset = transaction(&*repo, |tx| Box::pin(async move { msg_repo.reset_all_indexed(tx).await })).await?;
        tracing::info!(rows = reset, "reset all messages to unindexed for rebuild");

        // Only record the new version once the reset succeeded, so a failure
        // mid-rebuild retries on the next start rather than skipping the drain.
        write_version(self.index.dir()).map_err(|e| index_error(&e))?;
        Ok(())
    }

    /// Drain one batch of unindexed messages into the index.
    ///
    /// Returns the number of messages processed (indexed or skipped). Returns
    /// `Ok(0)` when nothing was pending, which the caller uses to detect
    /// convergence. Each processed message — including skipped poison pills —
    /// is marked indexed so the loop always makes forward progress.
    async fn drain_once(&self) -> Result<usize, Error> {
        // ---- Read phase: pull the next batch of pending messages. ----
        let repo = self.repository_service.repository().clone();
        let msg_repo = self.repository_service.message_repository().clone();
        let messages: Vec<Message> = read_only_transaction(&*repo, |tx| Box::pin(async move { msg_repo.list_unindexed(tx, BATCH).await })).await?;
        if messages.is_empty() {
            return Ok(0);
        }

        // ---- Fetch phase: decrypt + extract each body (async, no lock). ----
        let fields = *self.index.fields();
        let mut ids: Vec<MessageId> = Vec::with_capacity(messages.len());
        let mut docs: Vec<(MessageId, tantivy::TantivyDocument)> = Vec::with_capacity(messages.len());
        for msg in &messages {
            match self.raw_storage_service.get(msg.account_id, &msg.content_hash).await {
                // Retrieved: index it if it parses.
                Ok(raw) => {
                    if let Some(body) = extract_body(&raw) {
                        ids.push(msg.id);
                        docs.push((msg.id, to_document(&fields, msg, &body)));
                    } else {
                        // Poison pill: bytes that will not parse can never
                        // succeed; skip the document but mark the row so it never
                        // re-drains.
                        tracing::warn!(
                            message_id = msg.id,
                            account_id = msg.account_id,
                            "skipping unparseable message; marking indexed to avoid re-drain",
                        );
                        ids.push(msg.id);
                    }
                }
                // Poison pill: a permanently-absent blob can never succeed; skip
                // the document but mark the row so it never re-drains.
                Err(e @ Error::BlobNotFound { .. }) => {
                    tracing::warn!(
                        message_id = msg.id,
                        account_id = msg.account_id,
                        error = %e,
                        "skipping message with missing blob; marking indexed to avoid re-drain",
                    );
                    ids.push(msg.id);
                }
                // Any other error is assumed transient: fail the batch so it
                // retries next tick rather than dropping the message. Nothing has
                // been written or marked yet, so aborting here leaves no partial
                // state; the delete-before-add write makes the retry idempotent.
                Err(e) => return Err(e),
            }
        }

        // ---- Write phase: delete-then-add per doc, one commit (sync). ----
        self.write_batch(docs)?;

        // ---- Mark phase: flip the watermark only after the commit succeeds. ----
        let count = ids.len();
        let repo = self.repository_service.repository().clone();
        let msg_repo = self.repository_service.message_repository().clone();
        transaction(&*repo, |tx| Box::pin(async move { msg_repo.mark_indexed(tx, &ids).await })).await?;

        Ok(count)
    }

    /// Drain repeatedly until the backlog is empty. A batch failure is logged
    /// and ends this pass; the next tick retries.
    async fn drain_to_empty(&self) {
        loop {
            match self.drain_once().await {
                Ok(0) => break,
                Ok(_) => {}
                Err(e) => {
                    tracing::warn!(error = %e, "search drain batch failed; retrying on next tick");
                    break;
                }
            }
        }
    }

    /// Delete every document from the index and commit. Synchronous writer
    /// critical section: the guard never crosses an `.await`.
    fn clear_index(&self) -> Result<(), Error> {
        let handle = self.index.writer();
        let mut writer = handle.lock().map_err(|_| lock_poisoned())?;
        writer.delete_all_documents().map_err(|e| tantivy_error(&e))?;
        writer.commit().map_err(|e| tantivy_error(&e))?;
        Ok(())
    }

    /// Write one batch of documents — `delete_term(message_id)` then
    /// `add_document` for each, followed by a single `commit`. The delete makes
    /// re-indexing idempotent: a message committed but not yet marked (e.g.
    /// after a crash) replaces its prior document instead of duplicating it.
    /// Synchronous writer critical section: the guard never crosses an
    /// `.await`.
    fn write_batch(&self, docs: Vec<(MessageId, tantivy::TantivyDocument)>) -> Result<(), Error> {
        let message_id_field = self.index.fields().message_id;
        let handle = self.index.writer();
        let mut writer = handle.lock().map_err(|_| lock_poisoned())?;
        for (id, doc) in docs {
            writer.delete_term(Term::from_field_u64(message_id_field, id));
            writer.add_document(doc).map_err(|e| tantivy_error(&e))?;
        }
        writer.commit().map_err(|e| tantivy_error(&e))?;
        Ok(())
    }
}

impl IntoSubsystem<Error> for SearchSubsystem {
    async fn run(self, subsys: &mut SubsystemHandle) -> Result<(), Error> {
        tracing::info!("SearchSubsystem starting...");

        if let Err(e) = self.reconcile_on_startup().await {
            tracing::error!(error = %e, "search startup reconcile failed; continuing with existing index state");
        }
        tracing::info!("SearchSubsystem started");

        // `interval`'s first tick fires immediately, so the initial convergence
        // runs on the first loop iteration — inside the `select!`, so even a
        // first-boot full rebuild stops promptly when shutdown is requested
        // rather than running past the graceful-shutdown timeout. `Delay` keeps a
        // slow drain from bursting a backlog of missed ticks afterwards.
        let mut ticker = tokio::time::interval(POLL_INTERVAL);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                () = subsys.on_shutdown_requested() => break,
                _ = ticker.tick() => {}
            }
            // Converge the backlog, but abort promptly on shutdown: `drain_to_empty`
            // only awaits inside `drain_once`, so `select!` cancels it between (or
            // within) batches. A mid-batch cancel is safe — the delete-before-add
            // write makes the re-drain idempotent.
            tokio::select! {
                () = subsys.on_shutdown_requested() => break,
                () = self.drain_to_empty() => {}
            }
        }

        tracing::info!("SearchSubsystem stopped");
        Ok(())
    }
}

/// Extract the searchable text body from raw `.eml` bytes: prefer `text/plain`,
/// fall back to tag-stripped `text/html`.
///
/// Returns `Some(body)` when the message parses — the body is an empty string
/// when it parses but has no text part, which is a valid (searchable) document.
/// Returns `None` only when the bytes do not parse as a message at all, which
/// the caller treats as a poison pill (skip + mark indexed).
fn extract_body(raw: &[u8]) -> Option<String> {
    let msg = MessageParser::default().parse(raw)?;
    Some(
        msg.body_text(0)
            .map(Cow::into_owned)
            .or_else(|| msg.body_html(0).map(|h| html_to_text(&h)))
            .unwrap_or_default(),
    )
}

/// Minimal HTML-to-text for the searchable body: drop tags, and drop the
/// *content* of `<script>` and `<style>` elements so JS/CSS text never pollutes
/// the index. Snippet-level fidelity only — no HTML entity decoding, matching
/// the parser crate's snippet path.
fn html_to_text(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    while let Some(lt) = rest.find('<') {
        out.push_str(&rest[..lt]);
        let tag = &rest[lt..];
        // ASCII-lowercase preserves byte length, so indices into `lower` map
        // 1:1 onto `tag`; only ASCII tag names matter here.
        let lower = tag.to_ascii_lowercase();
        let end_tag = if lower.starts_with("<script") {
            Some("</script")
        } else if lower.starts_with("<style") {
            Some("</style")
        } else {
            None
        };
        match end_tag {
            // Skip the whole element, including its text content, up to and
            // including the matching closing tag.
            Some(close) => match lower.find(close).and_then(|e| tag[e..].find('>').map(|g| e + g)) {
                Some(end) => rest = &tag[end + 1..],
                None => return out, // unterminated element: drop the remainder
            },
            // Ordinary tag: drop just the tag, keeping any surrounding text.
            None => match tag.find('>') {
                Some(gt) => rest = &tag[gt + 1..],
                None => return out, // unterminated tag: drop the remainder
            },
        }
    }
    out.push_str(rest);
    out
}

/// A poisoned writer lock is unrecoverable infrastructure state.
fn lock_poisoned() -> Error {
    Error::Infrastructure("search index writer lock poisoned".to_string())
}

/// Map an index-management failure to a core error.
fn index_error(e: &crate::SearchIndexError) -> Error {
    Error::Infrastructure(format!("search index: {e}"))
}

/// Map a Tantivy engine failure to a core error.
fn tantivy_error(e: &tantivy::TantivyError) -> Error {
    Error::Infrastructure(format!("search engine: {e}"))
}

#[cfg(test)]
mod tests {
    use std::{
        collections::{HashMap, HashSet},
        sync::Mutex,
    };

    // `Arc` comes in via `use super::*`.
    use async_trait::async_trait;
    // `Arc`, `Error`, `Message`, `RepositoryService`, `read_only_transaction`,
    // `transaction`, `RawStorageService`, `SearchIndex`, `write_version`, and
    // `to_document` all come in via `use super::*`.
    use mk_core::{
        account::{AccountToken, NewAccount},
        crypto::Ciphertext,
        imap::{ImapServerConfig, TlsMode},
        message::{MessageToken, NewMessageRow},
        types::{ContentHash, EmailAddress},
        user::NewUser,
    };
    use mk_database::create_repository_service;
    use sea_orm::Database;
    use tantivy::{collector::TopDocs, query::QueryParser};
    use tempfile::TempDir;

    use super::*;

    /// A hand-rolled in-memory `RawStorageService`: canned plaintext keyed by
    /// `(account_id, content-hash hex)`. `get` returns the stored bytes,
    /// `Error::BlobNotFound` when absent (the permanent poison-pill signal), or
    /// a simulated *transient* `Error::Infrastructure` for keys registered via
    /// [`FakeRawStorage::fail_transient`]. Only `get` is exercised
    /// meaningfully.
    #[derive(Default)]
    struct FakeRawStorage {
        blobs: Mutex<HashMap<(u64, String), Vec<u8>>>,
        transient: Mutex<HashSet<(u64, String)>>,
    }

    impl FakeRawStorage {
        fn put(&self, account_id: u64, bytes: &[u8]) -> ContentHash {
            let hash = ContentHash::compute(bytes);
            self.blobs.lock().unwrap().insert((account_id, hash.as_hex()), bytes.to_vec());
            hash
        }

        /// Make `get` return a transient (non-`BlobNotFound`) error for this
        /// key until [`FakeRawStorage::clear_transient`] is called.
        fn fail_transient(&self, account_id: u64, hash: &ContentHash) {
            self.transient.lock().unwrap().insert((account_id, hash.as_hex()));
        }

        fn clear_transient(&self) {
            self.transient.lock().unwrap().clear();
        }
    }

    #[async_trait]
    impl RawStorageService for FakeRawStorage {
        async fn put_if_absent(&self, account_id: u64, plaintext: &[u8]) -> Result<ContentHash, Error> {
            Ok(self.put(account_id, plaintext))
        }

        async fn get(&self, account_id: u64, key: &ContentHash) -> Result<Vec<u8>, Error> {
            if self.transient.lock().unwrap().contains(&(account_id, key.as_hex())) {
                return Err(Error::Infrastructure("simulated transient storage error".to_string()));
            }
            self.blobs
                .lock()
                .unwrap()
                .get(&(account_id, key.as_hex()))
                .cloned()
                .ok_or_else(|| Error::BlobNotFound {
                    account_id,
                    hash: key.as_hex(),
                })
        }

        async fn exists(&self, account_id: u64, key: &ContentHash) -> Result<bool, Error> {
            Ok(self.blobs.lock().unwrap().contains_key(&(account_id, key.as_hex())))
        }

        async fn delete_account(&self, _account_id: u64) -> Result<(), Error> {
            Ok(())
        }
    }

    /// A fixture bundling a real in-memory repo, a temp-dir Tantivy index, a
    /// fake raw store, and a wired-up [`SearchSubsystem`].
    struct Fixture {
        _dir: TempDir,
        index: Arc<SearchIndex>,
        repo: Arc<RepositoryService>,
        storage: Arc<FakeRawStorage>,
        subsystem: SearchSubsystem,
    }

    async fn fixture() -> Fixture {
        let db = Database::connect("sqlite::memory:").await.unwrap();
        let repo = create_repository_service(db).await.unwrap();
        let dir = TempDir::new().unwrap();
        let index = Arc::new(SearchIndex::open_or_create(dir.path()).unwrap());
        let storage = Arc::new(FakeRawStorage::default());
        let subsystem = SearchSubsystem::new(Arc::clone(&index), Arc::clone(&repo), storage.clone() as Arc<dyn RawStorageService>);
        Fixture {
            _dir: dir,
            index,
            repo,
            storage,
            subsystem,
        }
    }

    async fn make_user(repo: &RepositoryService, username: &str, email: &str) -> u64 {
        let tx = repo.repository().begin().await.unwrap();
        let new_user = NewUser::new(username, "hash", email, std::collections::HashSet::new(), "Test", false).unwrap();
        let user = repo.user_repository().add_user(&*tx, new_user).await.unwrap();
        tx.commit().await.unwrap();
        user.id
    }

    async fn make_account(repo: &RepositoryService, user_id: u64, display_name: &str) -> u64 {
        let tx = repo.repository().begin().await.unwrap();
        let token = AccountToken::generate();
        let na = NewAccount {
            user_id,
            display_name: display_name.to_string(),
            email_address: EmailAddress::new(format!("user-{}@example.com", token.id())).unwrap(),
            server: ImapServerConfig {
                host: "imap.example.com".to_string(),
                port: 993,
                tls: TlsMode::Tls,
            },
            username: "user@example.com".to_string(),
            credentials: Ciphertext::from_raw(vec![0u8; 28]),
            token,
        };
        let acct = repo.account_repository().insert(&*tx, na).await.unwrap();
        tx.commit().await.unwrap();
        acct.id
    }

    /// A raw `.eml` with `subject` in the header and `body` as the plain-text
    /// body.
    fn eml(subject: &str, body: &str) -> Vec<u8> {
        format!(
            "Message-ID: <{subject}@example.com>\r\nFrom: Alice <alice@example.com>\r\nTo: bob@example.com\r\nSubject: {subject}\r\nDate: Tue, 1 Nov 2022 \
             10:00:00 +0000\r\n\r\n{body}\r\n"
        )
        .into_bytes()
    }

    /// Persist a message row whose `content_hash` matches `raw`, returning its
    /// id. The stored subject mirrors the header so index docs are
    /// searchable.
    async fn seed_message(repo: &RepositoryService, account_id: u64, subject: &str, raw: &[u8]) -> u64 {
        let tx = repo.repository().begin().await.unwrap();
        let row = NewMessageRow {
            token: MessageToken::generate(),
            account_id,
            rfc822_message_id: format!("<{subject}@example.com>"),
            content_hash: ContentHash::compute(raw),
            subject: Some(subject.to_string()),
            from_address: EmailAddress::new("alice@example.com").unwrap(),
            from_name: None,
            to_addresses: vec![],
            cc_addresses: vec![],
            bcc_addresses: vec![],
            reply_to_addresses: vec![],
            sent_date: None,
            in_reply_to: None,
            references: vec![],
            snippet: "snippet".to_string(),
            size_bytes: raw.len() as i64,
            has_attachments: false,
            attachment_count: 0,
        };
        let msg = repo.message_repository().create(&*tx, row).await.unwrap();
        tx.commit().await.unwrap();
        msg.id
    }

    /// Ids of every message still flagged `indexed = false`.
    async fn unindexed_ids(repo: &RepositoryService) -> Vec<u64> {
        let r = repo.repository().clone();
        let mr = repo.message_repository().clone();
        read_only_transaction(&*r, |tx| Box::pin(async move { mr.list_unindexed(tx, 1000).await }))
            .await
            .unwrap()
            .into_iter()
            .map(|m| m.id)
            .collect()
    }

    /// Add a raw document straight to the index and commit, bypassing the
    /// drain. Used to plant a doc that must survive (or be wiped by) a
    /// later operation.
    fn add_raw_doc(index: &SearchIndex, msg: &Message, body: &str) {
        let handle = index.writer();
        let mut writer = handle.lock().unwrap();
        writer.add_document(to_document(index.fields(), msg, body)).unwrap();
        writer.commit().unwrap();
    }

    /// Count of index documents whose `body`/`subject` matches `term`.
    fn count_body_hits(index: &SearchIndex, field: tantivy::schema::Field, term: &str) -> usize {
        let reader = index.reader().unwrap();
        let searcher = reader.searcher();
        let parser = QueryParser::for_index(index.index(), vec![field]);
        let query = parser.parse_query(term).unwrap();
        searcher.search(&query, &TopDocs::with_limit(1000).order_by_score()).unwrap().len()
    }

    #[tokio::test]
    async fn drain_indexes_all_unindexed_messages() {
        let fx = fixture().await;
        let user = make_user(&fx.repo, "alice", "alice@example.com").await;
        let account = make_account(&fx.repo, user, "Primary").await;

        let raw1 = eml("alpha", "the running quarterly report");
        let raw2 = eml("beta", "monthly financials overview");
        fx.storage.put(account, &raw1);
        fx.storage.put(account, &raw2);
        let id1 = seed_message(&fx.repo, account, "alpha", &raw1).await;
        let id2 = seed_message(&fx.repo, account, "beta", &raw2).await;

        // Both are pending before the drain.
        let mut before = unindexed_ids(&fx.repo).await;
        before.sort_unstable();
        let mut expected = vec![id1, id2];
        expected.sort_unstable();
        assert_eq!(before, expected);

        let drained = fx.subsystem.drain_once().await.unwrap();
        assert_eq!(drained, 2, "both messages processed in one pass");

        // Rows are now flagged indexed and the docs are searchable by body.
        assert!(unindexed_ids(&fx.repo).await.is_empty(), "no rows remain unindexed");
        let fields = fx.index.fields();
        assert_eq!(count_body_hits(&fx.index, fields.body, "running"), 1);
        assert_eq!(count_body_hits(&fx.index, fields.body, "financials"), 1);
    }

    #[tokio::test]
    async fn reindexing_a_message_produces_no_duplicate() {
        let fx = fixture().await;
        let user = make_user(&fx.repo, "alice", "alice@example.com").await;
        let account = make_account(&fx.repo, user, "Primary").await;

        let raw = eml("gamma", "the running quarterly report");
        fx.storage.put(account, &raw);
        let id = seed_message(&fx.repo, account, "gamma", &raw).await;

        // First drain indexes it.
        assert_eq!(fx.subsystem.drain_once().await.unwrap(), 1);
        let fields = fx.index.fields();
        assert_eq!(count_body_hits(&fx.index, fields.body, "running"), 1);

        // Reset the row's flag and drain again — delete-before-add must replace
        // the doc rather than add a second one. (Only one row exists here.)
        let r = fx.repo.repository().clone();
        let mr = fx.repo.message_repository().clone();
        let reset = transaction(&*r, |tx| Box::pin(async move { mr.reset_all_indexed(tx).await })).await.unwrap();
        assert_eq!(reset, 1);
        assert_eq!(unindexed_ids(&fx.repo).await, vec![id]);

        assert_eq!(fx.subsystem.drain_once().await.unwrap(), 1);
        assert_eq!(
            count_body_hits(&fx.index, fields.body, "running"),
            1,
            "re-index must not duplicate the document"
        );
    }

    #[tokio::test]
    async fn poison_pill_message_is_marked_and_skipped() {
        let fx = fixture().await;
        let user = make_user(&fx.repo, "alice", "alice@example.com").await;
        let account = make_account(&fx.repo, user, "Primary").await;

        // Seed a message row but deliberately do NOT store its blob.
        let raw = eml("delta", "unreachable body");
        let id = seed_message(&fx.repo, account, "delta", &raw).await;
        assert_eq!(unindexed_ids(&fx.repo).await, vec![id]);

        // The drain processes it (count 1) without erroring or looping forever.
        let drained = fx.subsystem.drain_once().await.unwrap();
        assert_eq!(drained, 1, "poison pill is counted as processed");
        assert!(unindexed_ids(&fx.repo).await.is_empty(), "poison pill is marked indexed so it can't re-drain");

        // No document was indexed for it.
        let fields = fx.index.fields();
        assert_eq!(count_body_hits(&fx.index, fields.body, "unreachable"), 0);
        // A follow-up drain finds nothing pending.
        assert_eq!(fx.subsystem.drain_once().await.unwrap(), 0);
    }

    #[tokio::test]
    async fn startup_rebuild_clears_index_and_requeues_all_rows() {
        let fx = fixture().await;
        let user = make_user(&fx.repo, "alice", "alice@example.com").await;
        let account = make_account(&fx.repo, user, "Primary").await;

        // Seed two messages and index them so the index and DB agree, then mark
        // them indexed to simulate a fully-drained prior run.
        let raw1 = eml("epsilon", "the running quarterly report");
        let raw2 = eml("zeta", "monthly financials overview");
        fx.storage.put(account, &raw1);
        fx.storage.put(account, &raw2);
        let id1 = seed_message(&fx.repo, account, "epsilon", &raw1).await;
        let id2 = seed_message(&fx.repo, account, "zeta", &raw2).await;
        assert_eq!(fx.subsystem.drain_once().await.unwrap(), 2);
        assert!(unindexed_ids(&fx.repo).await.is_empty());

        // Put a stale doc directly in the index that no longer has a DB row, to
        // prove the rebuild wipes the whole index, not just known rows.
        let ghost = Message {
            id: 999_999,
            version: 1,
            token: MessageToken::new(999_999),
            account_id: account,
            rfc822_message_id: "<ghost@example.com>".to_string(),
            content_hash: ContentHash::compute(b"ghost"),
            subject: Some("ghost".to_string()),
            from_address: EmailAddress::new("alice@example.com").unwrap(),
            from_name: None,
            to_addresses: vec![],
            cc_addresses: vec![],
            bcc_addresses: vec![],
            reply_to_addresses: vec![],
            sent_date: None,
            in_reply_to: None,
            references: vec![],
            snippet: "ghost".to_string(),
            size_bytes: 1,
            has_attachments: false,
            attachment_count: 0,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };
        add_raw_doc(&fx.index, &ghost, "haunting the index");

        // Force a version mismatch by writing a stale sidecar version.
        std::fs::write(fx.index.dir().join("schema_version"), (crate::SCHEMA_VERSION + 1).to_string()).unwrap();
        assert!(needs_rebuild(fx.index.dir()));

        // Rebuild: index cleared, all rows re-queued, sidecar restored.
        fx.subsystem.reconcile_on_startup().await.unwrap();

        let fields = fx.index.fields();
        assert_eq!(count_body_hits(&fx.index, fields.body, "haunting"), 0, "ghost doc must be wiped");
        assert_eq!(count_body_hits(&fx.index, fields.body, "running"), 0, "real docs cleared pending re-drain");
        let mut requeued = unindexed_ids(&fx.repo).await;
        requeued.sort_unstable();
        let mut expected = vec![id1, id2];
        expected.sort_unstable();
        assert_eq!(requeued, expected, "every row reset to unindexed");
        assert_eq!(
            crate::index::read_version(fx.index.dir()),
            Some(crate::SCHEMA_VERSION),
            "sidecar now equals current version"
        );
        assert!(!needs_rebuild(fx.index.dir()));

        // Re-drain rebuilds the documents from source.
        assert_eq!(fx.subsystem.drain_once().await.unwrap(), 2);
        assert_eq!(count_body_hits(&fx.index, fields.body, "running"), 1);
        assert_eq!(count_body_hits(&fx.index, fields.body, "financials"), 1);
    }

    /// `write_version` alone makes `needs_rebuild` false, so reconcile is a
    /// no-op and leaves the existing index and flags untouched.
    #[tokio::test]
    async fn reconcile_is_noop_when_version_matches() {
        let fx = fixture().await;
        let user = make_user(&fx.repo, "alice", "alice@example.com").await;
        let account = make_account(&fx.repo, user, "Primary").await;

        let raw = eml("eta", "the running quarterly report");
        fx.storage.put(account, &raw);
        let id = seed_message(&fx.repo, account, "eta", &raw).await;
        assert_eq!(fx.subsystem.drain_once().await.unwrap(), 1);

        write_version(fx.index.dir()).unwrap();
        assert!(!needs_rebuild(fx.index.dir()));

        fx.subsystem.reconcile_on_startup().await.unwrap();

        // Row stays indexed (not re-queued) and the doc stays searchable.
        assert!(unindexed_ids(&fx.repo).await.is_empty(), "matching version must not re-queue rows");
        let fields = fx.index.fields();
        assert_eq!(count_body_hits(&fx.index, fields.body, "running"), 1);
        let _ = id;
    }

    #[tokio::test]
    async fn drain_to_empty_converges_across_multiple_batches() {
        let fx = fixture().await;
        let user = make_user(&fx.repo, "alice", "alice@example.com").await;
        let account = make_account(&fx.repo, user, "Primary").await;

        // Seed more than one BATCH (128) so `drain_to_empty` must loop across at
        // least two `drain_once` passes to converge.
        let total = usize::try_from(BATCH).unwrap() + 2;
        for i in 0..total {
            let subject = format!("msg{i}");
            let raw = eml(&subject, &format!("widget report number {i}"));
            fx.storage.put(account, &raw);
            seed_message(&fx.repo, account, &subject, &raw).await;
        }
        assert_eq!(unindexed_ids(&fx.repo).await.len(), total, "all seeded rows start unindexed");

        // A single drain_once only clears one batch...
        assert_eq!(fx.subsystem.drain_once().await.unwrap(), usize::try_from(BATCH).unwrap());
        assert_eq!(unindexed_ids(&fx.repo).await.len(), total - usize::try_from(BATCH).unwrap());

        // ...but drain_to_empty loops until nothing is left.
        fx.subsystem.drain_to_empty().await;
        assert!(unindexed_ids(&fx.repo).await.is_empty(), "every batch drained");
        let fields = fx.index.fields();
        assert_eq!(count_body_hits(&fx.index, fields.body, "widget"), total, "all {total} docs are searchable");
    }

    #[tokio::test]
    async fn transient_get_error_fails_batch_and_leaves_row_unindexed() {
        let fx = fixture().await;
        let user = make_user(&fx.repo, "alice", "alice@example.com").await;
        let account = make_account(&fx.repo, user, "Primary").await;

        // The blob IS stored, but get is forced to return a transient error.
        let raw = eml("theta", "the running quarterly report");
        let hash = fx.storage.put(account, &raw);
        let id = seed_message(&fx.repo, account, "theta", &raw).await;
        fx.storage.fail_transient(account, &hash);

        // Unlike a poison pill, a transient error fails the batch and does NOT
        // mark the row — so it stays queued for retry.
        let err = fx.subsystem.drain_once().await.unwrap_err();
        assert!(matches!(err, Error::Infrastructure(_)), "transient error propagates, got {err:?}");
        assert_eq!(unindexed_ids(&fx.repo).await, vec![id], "transient failure must not drop the row");
        let fields = fx.index.fields();
        assert_eq!(count_body_hits(&fx.index, fields.body, "running"), 0, "nothing indexed on transient failure");

        // Once the transient condition clears, the retry indexes it normally.
        fx.storage.clear_transient();
        assert_eq!(fx.subsystem.drain_once().await.unwrap(), 1);
        assert!(unindexed_ids(&fx.repo).await.is_empty());
        assert_eq!(count_body_hits(&fx.index, fields.body, "running"), 1);
    }

    #[test]
    fn html_to_text_drops_tags_and_script_style_content() {
        let out = html_to_text("<style>.a{color:crimson}</style><p>Visible body</p><script>var secrettoken = 1;</script>");
        assert!(out.contains("Visible body"), "visible text is kept: {out:?}");
        assert!(!out.contains("secrettoken"), "script content must be dropped: {out:?}");
        assert!(!out.contains("crimson"), "style content must be dropped: {out:?}");
    }

    #[tokio::test]
    async fn html_only_message_body_is_searchable_without_script_text() {
        let fx = fixture().await;
        let user = make_user(&fx.repo, "alice", "alice@example.com").await;
        let account = make_account(&fx.repo, user, "Primary").await;

        // A text/html-only message: the visible text must be indexed; the
        // <script> content must not.
        let raw = b"Message-ID: <htmlonly@example.com>\r\nFrom: Alice <alice@example.com>\r\nTo: bob@example.com\r\nSubject: newsletter\r\nContent-Type: text/html; charset=utf-8\r\nDate: Tue, 1 Nov 2022 10:00:00 +0000\r\n\r\n<html><body><p>quarterlreport highlights</p><script>var trackerword = 42;</script></body></html>\r\n".to_vec();
        fx.storage.put(account, &raw);
        seed_message(&fx.repo, account, "newsletter", &raw).await;

        assert_eq!(fx.subsystem.drain_once().await.unwrap(), 1);
        let fields = fx.index.fields();
        assert_eq!(count_body_hits(&fx.index, fields.body, "quarterlreport"), 1, "html body text is searchable");
        assert_eq!(count_body_hits(&fx.index, fields.body, "trackerword"), 0, "script content is not indexed");
    }
}
