//! [`TantivySearchService`]: the `mk_core::search::SearchService` adapter.
//!
//! This is the correctness- and security-critical boundary of the search
//! feature. It compiles the parsed query AST into a Tantivy query and — no
//! matter what the user typed — **always** constrains results to the requesting
//! user's own accounts. Account and folder names in the query are resolved only
//! within that user's accounts, so a name that belongs to another user (or to
//! no one) resolves to nothing and can never widen the scope.
//!
//! ## Query construction (manual, not `QueryParser`)
//!
//! Query building is manual rather than via `QueryParser`. Two reasons:
//!
//! 1. **Security is structural.** The top-level query is a [`BooleanQuery`]
//!    whose first clause is a mandatory ([`Occur::Must`]) account-scope
//!    sub-query. No residual clause — however malformed — can escape that
//!    `Must`, because a `MustNot`/`Should`/`Must` sibling cannot relax a
//!    `Must`. Feeding raw user text to `QueryParser` would risk field-injection
//!    or parse errors; manual construction sidesteps both.
//! 2. **Correctness of stemming.** Text values are tokenized through the very
//!    same `en_stem` analyzer registered on the index, so a query for `running`
//!    becomes the stemmed term `run` and matches the stemmed indexed tokens.
//!
//! ## Mutex choice
//!
//! Writes (`delete_account`) lock the [`SearchIndex`]'s single shared writer
//! via a `std::sync::Mutex`. The critical section is fully synchronous — a
//! delete followed by a commit — with **no `.await` while the guard is held**,
//! so a std mutex is the simplest correct choice; a `tokio::sync::Mutex` would
//! buy nothing here.
//!
//! ## `total` approximation
//!
//! `SearchResults::total` is the Tantivy count of the account-scoped query,
//! taken *before* the `folder:` DB post-filter. With no `folder:` constraint it
//! is exact. With one, it is an upper bound (it counts documents the folder
//! post-filter later drops); this is logged at debug. Approximate totals are an
//! accepted v1 tradeoff — the alternative (counting post-filtered survivors)
//! would require fetching every matching document, defeating pagination.

use std::{
    collections::{HashMap, HashSet},
    ops::Bound,
    sync::Arc,
};

// Re-export the AST term type under an unambiguous name; `Term` is Tantivy's.
use mk_core::search::query::Term as QueryTerm;
use mk_core::{
    Error,
    account::{Account, AccountId},
    folder::{Folder, FolderId},
    message::MessageId,
    repository::{RepositoryService, read_only_transaction},
    search::{
        SearchHit, SearchResults, SearchService,
        query::{Clause, DateBound, TextField, parse},
    },
    user::UserId,
};
use tantivy::{
    DateTime, DocAddress, TantivyDocument, Term,
    collector::{Count, TopDocs},
    query::{BooleanQuery, BoostQuery, EmptyQuery, Occur, PhraseQuery, Query, RangeQuery, TermQuery},
    schema::{Field, IndexRecordOption, Value},
    tokenizer::TokenStream,
};

use crate::{SearchIndex, schema::EN_STEM};

/// Relevance boost applied to a subject match over a body match for the same
/// bare term, so a term in the subject line outranks the same term buried in
/// the body.
const SUBJECT_BOOST: f32 = 2.0;

/// Multiplier applied to the requested page window when a `folder:` post-filter
/// is present, to leave headroom for documents the DB filter will drop.
const FOLDER_OVERFETCH: usize = 10;

/// Hard cap on how many documents a single search fetches from Tantivy,
/// bounding memory and latency regardless of `offset`/`limit`.
const MAX_FETCH: usize = 10_000;

/// Tantivy-backed implementation of the [`SearchService`] port.
pub struct TantivySearchService {
    index: Arc<SearchIndex>,
    repository_service: Arc<RepositoryService>,
}

impl std::fmt::Debug for TantivySearchService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TantivySearchService").field("index", &self.index).finish_non_exhaustive()
    }
}

impl TantivySearchService {
    /// Build a search service over the given shared index and repositories.
    #[must_use]
    pub fn new(index: Arc<SearchIndex>, repository_service: Arc<RepositoryService>) -> Self {
        Self { index, repository_service }
    }

    /// Tokenize `value` through the index's `en_stem` analyzer, yielding the
    /// stemmed, lowercased tokens exactly as they were indexed.
    fn stem_tokens(&self, value: &str) -> Vec<String> {
        let Some(mut analyzer) = self.index.index().tokenizers().get(EN_STEM) else {
            // Fails closed (no tokens → matches nothing), but log so a
            // misconfigured index does not silently return zero results.
            tracing::warn!(tokenizer = EN_STEM, "search tokenizer missing; query text will match nothing");
            return Vec::new();
        };
        let mut tokens = Vec::new();
        let mut stream = analyzer.token_stream(value);
        while stream.advance() {
            tokens.push(stream.token().text.clone());
        }
        tokens
    }

    /// A query matching `value` against a single stemmed text `field`.
    ///
    /// A single token becomes a [`TermQuery`]; multiple tokens become a
    /// [`PhraseQuery`] (positions are indexed on every stemmed field). A value
    /// that tokenizes to nothing becomes an [`EmptyQuery`] — a required clause
    /// that stems away therefore matches nothing.
    fn text_field_query(&self, field: Field, value: &str) -> Box<dyn Query> {
        let tokens = self.stem_tokens(value);
        match tokens.as_slice() {
            [] => Box::new(EmptyQuery),
            [one] => Box::new(TermQuery::new(Term::from_field_text(field, one), IndexRecordOption::Basic)),
            many => {
                let terms: Vec<Term> = many.iter().map(|t| Term::from_field_text(field, t)).collect();
                Box::new(PhraseQuery::new(terms))
            }
        }
    }

    /// A bare full-text query over `subject` OR `body`, with the subject side
    /// boosted so subject-line matches rank above body-only matches.
    fn bare_query(&self, value: &str) -> Box<dyn Query> {
        let fields = self.index.fields();
        let subject = self.text_field_query(fields.subject, value);
        let body = self.text_field_query(fields.body, value);
        Box::new(BooleanQuery::new(vec![
            (Occur::Should, Box::new(BoostQuery::new(subject, SUBJECT_BOOST)) as Box<dyn Query>),
            (Occur::Should, body),
        ]))
    }

    /// Compile one residual clause (never account/folder) into an occurrence +
    /// Tantivy sub-query.
    fn clause_query(&self, clause: &Clause) -> (Occur, Box<dyn Query>) {
        let fields = self.index.fields();
        let occur = if clause.negated { Occur::MustNot } else { Occur::Must };
        let query: Box<dyn Query> = match &clause.term {
            QueryTerm::Bare(value) => self.bare_query(value),
            QueryTerm::Text { field, value } => {
                let f = match field {
                    TextField::Subject => fields.subject,
                    TextField::From => fields.from,
                    TextField::To => fields.to,
                };
                self.text_field_query(f, value)
            }
            QueryTerm::Date { bound, date } => date_range_query(fields.sent_date, *bound, *date),
            QueryTerm::Attachments(has) => Box::new(TermQuery::new(
                Term::from_field_u64(fields.has_attachments, u64::from(*has)),
                IndexRecordOption::Basic,
            )),
            // Account/folder terms are split off before compilation.
            QueryTerm::Account(_) | QueryTerm::Folder(_) => Box::new(EmptyQuery),
        };
        (occur, query)
    }

    /// Assemble the full Tantivy query: a mandatory account-scope clause plus
    /// every residual clause. The scope clause is the security boundary — it is
    /// always `Occur::Must`, so nothing outside `scope` can be returned.
    fn build_query(&self, scope: &[AccountId], residual: &[Clause]) -> BooleanQuery {
        let fields = self.index.fields();

        let scope_terms: Vec<(Occur, Box<dyn Query>)> = scope
            .iter()
            .map(|id| {
                let q: Box<dyn Query> = Box::new(TermQuery::new(Term::from_field_u64(fields.account_id, *id), IndexRecordOption::Basic));
                (Occur::Should, q)
            })
            .collect();
        let scope_query = BooleanQuery::new(scope_terms);

        let mut subqueries: Vec<(Occur, Box<dyn Query>)> = Vec::with_capacity(residual.len() + 1);
        subqueries.push((Occur::Must, Box::new(scope_query)));
        for clause in residual {
            subqueries.push(self.clause_query(clause));
        }
        BooleanQuery::new(subqueries)
    }
}

/// Build a `sent_date` range query for a date bound. Days are half-open UTC
/// intervals `[00:00 of D, 00:00 of D+1)`:
/// - `date:D`   → `[D, D+1)` (the whole day),
/// - `before:D` → `(-inf, D)` (strictly before day D),
/// - `after:D`  → `[D+1, +inf)` (strictly after day D).
///
/// The three bounds therefore partition the timeline without overlap.
fn date_range_query(field: Field, bound: DateBound, date: chrono::NaiveDate) -> Box<dyn Query> {
    let start = day_start(date);
    let next = date.succ_opt().map_or(start, day_start);
    let (lower, upper) = match bound {
        DateBound::On => (
            Bound::Included(Term::from_field_date(field, start)),
            Bound::Excluded(Term::from_field_date(field, next)),
        ),
        DateBound::Before => (Bound::Unbounded, Bound::Excluded(Term::from_field_date(field, start))),
        DateBound::After => (Bound::Included(Term::from_field_date(field, next)), Bound::Unbounded),
    };
    Box::new(RangeQuery::new(lower, upper))
}

/// Midnight-UTC [`DateTime`] at the start of `date`.
fn day_start(date: chrono::NaiveDate) -> DateTime {
    let ts = date.and_hms_opt(0, 0, 0).expect("midnight is always a valid time").and_utc().timestamp();
    DateTime::from_timestamp_secs(ts)
}

/// Split parsed clauses into account hints, folder hints, and residual (scoring
/// / non-name-resolving) clauses.
type AccountHint = (bool, String);
type FolderHint = (bool, String);

fn split_clauses(clauses: Vec<Clause>) -> (Vec<AccountHint>, Vec<FolderHint>, Vec<Clause>) {
    let mut accounts = Vec::new();
    let mut folders = Vec::new();
    let mut residual = Vec::new();
    for clause in clauses {
        match clause.term {
            QueryTerm::Account(name) => accounts.push((clause.negated, name)),
            QueryTerm::Folder(name) => folders.push((clause.negated, name)),
            term => residual.push(Clause { negated: clause.negated, term }),
        }
    }
    (accounts, folders, residual)
}

/// Resolve the account scope from the user's owned accounts and the `account:`
/// hints. Returns the set of account ids the search may touch — **never**
/// anything the user does not own.
fn resolve_scope(accounts: &[Account], hints: &[AccountHint]) -> HashSet<AccountId> {
    let mut name_map: HashMap<String, AccountId> = HashMap::new();
    for a in accounts {
        // Last write wins on duplicate display names; a rare edge, acceptable
        // for v1 name resolution.
        name_map.insert(a.display_name.to_lowercase(), a.id);
    }
    let mut scope: HashSet<AccountId> = accounts.iter().map(|a| a.id).collect();
    for (negated, name) in hints {
        let resolved = name_map.get(&name.to_lowercase()).copied();
        if *negated {
            if let Some(id) = resolved {
                scope.remove(&id);
            }
        } else {
            // Intersect scope with the single resolved id. An unknown name (or
            // one owned by another user) resolves to None and empties the scope.
            match resolved {
                Some(id) if scope.contains(&id) => scope = HashSet::from([id]),
                _ => scope.clear(),
            }
        }
    }
    scope
}

#[async_trait::async_trait]
impl SearchService for TantivySearchService {
    async fn search(&self, user_id: UserId, query: &str, limit: u32, offset: u32) -> Result<SearchResults, Error> {
        let (account_hints, folder_hints, residual) = split_clauses(parse(query).clauses);

        // ---- DB phase 1: resolve account scope + folder ids (one read txn) ----
        let account_repo = self.repository_service.account_repository().clone();
        let folder_repo = self.repository_service.folder_repository().clone();
        let repo = self.repository_service.repository().clone();

        let (scope, folder_constraints): (Vec<AccountId>, Vec<(bool, Vec<FolderId>)>) = read_only_transaction(&*repo, |tx| {
            Box::pin(async move {
                let accounts = account_repo.list_for_user(tx, user_id).await?;
                let scope = resolve_scope(&accounts, &account_hints);
                if scope.is_empty() {
                    return Ok((Vec::new(), Vec::new()));
                }

                // Gather every folder in the scoped accounts once, then resolve
                // each `folder:` hint against path or display name.
                let mut scoped_folders: Vec<Folder> = Vec::new();
                for id in &scope {
                    let mut fs = folder_repo.list_for_account(tx, *id).await?;
                    scoped_folders.append(&mut fs);
                }
                let folder_constraints: Vec<(bool, Vec<FolderId>)> = folder_hints
                    .iter()
                    .map(|(negated, name)| {
                        // Case-insensitive substring match on the folder path or
                        // display name, so `folder:photography` resolves a nested
                        // folder like `Hobbies/Photography`.
                        let needle = name.to_lowercase();
                        let ids: Vec<FolderId> = scoped_folders
                            .iter()
                            .filter(|f| {
                                f.path.to_lowercase().contains(&needle) || f.display_name.as_deref().is_some_and(|d| d.to_lowercase().contains(&needle))
                            })
                            .map(|f| f.id)
                            .collect();
                        (*negated, ids)
                    })
                    .collect();

                Ok((scope.into_iter().collect(), folder_constraints))
            })
        })
        .await?;

        // SECURITY: an empty scope (unknown/cross-user account name) returns
        // nothing without ever touching the index.
        if scope.is_empty() {
            return Ok(SearchResults { total: 0, hits: Vec::new() });
        }

        // ---- Tantivy phase: compile + execute the scoped query ----
        let tantivy_query = self.build_query(&scope, &residual);
        let reader = self.index.reader().map_err(|e| index_error(&e))?;
        let searcher = reader.searcher();

        let want = (offset as usize).saturating_add(limit as usize);
        let fetch = if folder_constraints.is_empty() {
            want
        } else {
            want.saturating_mul(FOLDER_OVERFETCH)
        }
        .clamp(1, MAX_FETCH);
        if fetch == MAX_FETCH {
            tracing::debug!(
                fetch,
                "search fetch hit MAX_FETCH cap; deep pagination or wide folder over-fetch may truncate results"
            );
        }

        // One pass: the tuple collector yields the total count and the ranked
        // page from a single search rather than executing the query twice.
        let (total, top): (usize, Vec<(f32, DocAddress)>) = searcher
            .search(&tantivy_query, &(Count, TopDocs::with_limit(fetch).order_by_score()))
            .map_err(|e| tantivy_error(&e))?;
        if !folder_constraints.is_empty() {
            tracing::debug!(total, "search total is an upper bound: folder post-filter may drop matches");
        }

        let fields = self.index.fields();
        // account_id is a FAST (columnar) field, not stored. Resolve each
        // segment's column once and reuse it for every hit in that segment.
        let mut account_cols = std::collections::HashMap::new();
        let mut hits: Vec<SearchHit> = Vec::with_capacity(top.len());
        for (score, addr) in top {
            let doc: TantivyDocument = searcher.doc(addr).map_err(|e| tantivy_error(&e))?;
            let message_id = doc
                .get_first(fields.message_id)
                .and_then(|v| v.as_u64())
                .ok_or_else(|| Error::Infrastructure("search hit missing message_id".to_string()))?;
            let snippet = doc.get_first(fields.snippet).and_then(|v| v.as_str()).unwrap_or_default().to_string();
            let account_col = match account_cols.entry(addr.segment_ord) {
                std::collections::hash_map::Entry::Occupied(e) => e.into_mut(),
                std::collections::hash_map::Entry::Vacant(e) => e.insert(
                    searcher
                        .segment_reader(addr.segment_ord)
                        .fast_fields()
                        .u64("account_id")
                        .map_err(|err| tantivy_error(&err))?,
                ),
            };
            let account_id = account_col
                .first(addr.doc_id)
                .ok_or_else(|| Error::Infrastructure("search hit missing account_id".to_string()))?;
            hits.push(SearchHit {
                message_id,
                account_id,
                score,
                snippet,
            });
        }

        // ---- DB phase 2: folder post-filter (preserves rank order) ----
        if !folder_constraints.is_empty() {
            let ranked_ids: Vec<MessageId> = hits.iter().map(|h| h.message_id).collect();
            let loc_repo = self.repository_service.message_location_repository().clone();
            let repo2 = self.repository_service.repository().clone();
            let keep: HashSet<MessageId> = read_only_transaction(&*repo2, |tx| {
                Box::pin(async move {
                    let mut keep: HashSet<MessageId> = ranked_ids.iter().copied().collect();
                    for (negated, folder_ids) in &folder_constraints {
                        if folder_ids.is_empty() {
                            // A required folder that matched no real folder can
                            // never be satisfied → drop everything. A negated
                            // one excludes nothing.
                            if !negated {
                                keep.clear();
                                break;
                            }
                            continue;
                        }
                        let matched: HashSet<MessageId> = loc_repo.filter_message_ids_in_folders(tx, &ranked_ids, folder_ids).await?.into_iter().collect();
                        if *negated {
                            keep.retain(|id| !matched.contains(id));
                        } else {
                            keep.retain(|id| matched.contains(id));
                        }
                    }
                    Ok(keep)
                })
            })
            .await?;
            hits.retain(|h| keep.contains(&h.message_id));
        }

        let hits: Vec<SearchHit> = hits.into_iter().skip(offset as usize).take(limit as usize).collect();
        Ok(SearchResults { total, hits })
    }

    async fn delete_account(&self, account_id: AccountId) -> Result<(), Error> {
        let fields = self.index.fields();
        let handle = self.index.writer();
        // Short, fully-synchronous critical section: no `.await` while holding
        // the writer lock (see module docs on the std-mutex choice).
        let mut writer = handle
            .lock()
            .map_err(|_| Error::Infrastructure("search index writer lock poisoned".to_string()))?;
        writer.delete_term(Term::from_field_u64(fields.account_id, account_id));
        writer.commit().map_err(|e| tantivy_error(&e))?;
        Ok(())
    }
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
    use chrono::{DateTime, TimeZone, Utc};
    use mk_core::{
        account::{AccountToken, NewAccount},
        crypto::Ciphertext,
        folder::{FolderToken, NewFolderRow, SpecialUse},
        imap::{ImapServerConfig, TlsMode},
        message::{Message, MessageFlags, MessageLocationToken, MessageToken, NewMessageLocationRow, NewMessageRow},
        types::{ContentHash, EmailAddress},
        user::NewUser,
    };
    use mk_database::create_repository_service;
    use sea_orm::Database;
    use tempfile::TempDir;

    use super::*;
    use crate::schema::to_document;

    /// A test fixture bundling a fresh in-memory repo and a temp-dir Tantivy
    /// index behind a live [`TantivySearchService`].
    struct Fixture {
        _dir: TempDir,
        index: Arc<SearchIndex>,
        repo: Arc<RepositoryService>,
        service: TantivySearchService,
    }

    async fn fixture() -> Fixture {
        let db = Database::connect("sqlite::memory:").await.unwrap();
        let repo = create_repository_service(db).await.unwrap();
        let dir = TempDir::new().unwrap();
        let index = Arc::new(SearchIndex::open_or_create(dir.path()).unwrap());
        let service = TantivySearchService::new(Arc::clone(&index), Arc::clone(&repo));
        Fixture {
            _dir: dir,
            index,
            repo,
            service,
        }
    }

    async fn make_user(repo: &RepositoryService, username: &str, email: &str) -> u64 {
        let tx = repo.repository().begin().await.unwrap();
        let new_user = NewUser::new(username, "hash", email, HashSet::new(), "Test", false).unwrap();
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

    async fn make_folder(repo: &RepositoryService, account_id: u64, path: &str) -> u64 {
        let tx = repo.repository().begin().await.unwrap();
        let row = NewFolderRow {
            token: FolderToken::generate(),
            account_id,
            path: path.to_string(),
            display_name: None,
            special_use: Some(SpecialUse::Inbox),
            idle_enabled: true,
            uidvalidity: None,
        };
        let folders = repo.folder_repository().create_many(&*tx, account_id, vec![row]).await.unwrap();
        tx.commit().await.unwrap();
        folders[0].id
    }

    /// A minimal, all-defaults message row for `account_id`; tests mutate the
    /// fields they care about before handing it to [`create_and_index`].
    fn base_row(account_id: u64, rfc_id: &str) -> NewMessageRow {
        NewMessageRow {
            token: MessageToken::generate(),
            account_id,
            rfc822_message_id: rfc_id.to_string(),
            content_hash: ContentHash::compute(rfc_id.as_bytes()),
            subject: None,
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
            size_bytes: 10,
            has_attachments: false,
            attachment_count: 0,
        }
    }

    /// Persist `row` to the DB, then index the resulting message with `body`.
    /// Returns the assigned message id (shared by the DB row and the index
    /// doc).
    async fn create_and_index(fx: &Fixture, row: NewMessageRow, body: &str) -> u64 {
        let tx = fx.repo.repository().begin().await.unwrap();
        let msg: Message = fx.repo.message_repository().create(&*tx, row).await.unwrap();
        tx.commit().await.unwrap();
        index_message(&fx.index, &msg, body);
        msg.id
    }

    fn index_message(index: &SearchIndex, msg: &Message, body: &str) {
        let handle = index.writer();
        let mut writer = handle.lock().unwrap();
        writer.add_document(to_document(index.fields(), msg, body)).unwrap();
        writer.commit().unwrap();
    }

    async fn make_location(repo: &RepositoryService, message_id: u64, folder_id: u64, uid: u32) {
        let tx = repo.repository().begin().await.unwrap();
        let row = NewMessageLocationRow {
            token: MessageLocationToken::generate(),
            message_id,
            folder_id,
            uid,
            uidvalidity: 100,
            flags: MessageFlags::default(),
            internal_date: Utc::now(),
        };
        repo.message_location_repository().upsert(&*tx, row).await.unwrap();
        tx.commit().await.unwrap();
    }

    fn ids(results: &SearchResults) -> Vec<u64> {
        results.hits.iter().map(|h| h.message_id).collect()
    }

    fn day(y: i32, m: u32, d: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(y, m, d, 12, 0, 0).unwrap()
    }

    #[tokio::test]
    async fn bare_term_subject_outranks_body() {
        let fx = fixture().await;
        let user = make_user(&fx.repo, "alice", "alice@example.com").await;
        let account = make_account(&fx.repo, user, "Primary").await;

        let mut subj = base_row(account, "<subj@x>");
        subj.subject = Some("Quarterly running report".to_string());
        let subject_hit = create_and_index(&fx, subj, "unrelated body text").await;

        let body = base_row(account, "<body@x>");
        let body_hit = create_and_index(&fx, body, "the running totals are here").await;

        let results = fx.service.search(user, "running", 10, 0).await.unwrap();
        assert_eq!(results.hits.len(), 2, "both docs match");
        assert_eq!(results.hits[0].message_id, subject_hit, "subject match must rank first");
        assert_eq!(results.hits[1].message_id, body_hit);
        assert!(results.hits[0].score > results.hits[1].score, "subject boost must raise the score");
    }

    #[tokio::test]
    async fn subject_and_from_field_filters() {
        let fx = fixture().await;
        let user = make_user(&fx.repo, "alice", "alice@example.com").await;
        let account = make_account(&fx.repo, user, "Primary").await;

        let mut amazon = base_row(account, "<amazon@x>");
        amazon.subject = Some("Amazon order confirmation".to_string());
        amazon.from_address = EmailAddress::new("orders@amazon.com").unwrap();
        let amazon_id = create_and_index(&fx, amazon, "body").await;

        let mut news = base_row(account, "<news@x>");
        news.subject = Some("Weekly newsletter".to_string());
        news.from_address = EmailAddress::new("bob@example.com").unwrap();
        let news_id = create_and_index(&fx, news, "body").await;

        let by_subject = fx.service.search(user, "subject:Amazon", 10, 0).await.unwrap();
        assert_eq!(ids(&by_subject), vec![amazon_id]);

        let by_from = fx.service.search(user, "from:bob", 10, 0).await.unwrap();
        assert_eq!(ids(&by_from), vec![news_id]);
    }

    #[tokio::test]
    async fn negated_term_excludes() {
        let fx = fixture().await;
        let user = make_user(&fx.repo, "alice", "alice@example.com").await;
        let account = make_account(&fx.repo, user, "Primary").await;

        let plain = base_row(account, "<plain@x>");
        let plain_id = create_and_index(&fx, plain, "the report summary").await;

        let filtered = base_row(account, "<filtered@x>");
        let _filtered_id = create_and_index(&fx, filtered, "the report about filters").await;

        let results = fx.service.search(user, "report !filters", 10, 0).await.unwrap();
        assert_eq!(ids(&results), vec![plain_id]);
    }

    #[tokio::test]
    async fn date_range_bounds() {
        let fx = fixture().await;
        let user = make_user(&fx.repo, "alice", "alice@example.com").await;
        let account = make_account(&fx.repo, user, "Primary").await;

        let mut winter = base_row(account, "<winter@x>");
        winter.subject = Some("newsletter".to_string());
        winter.sent_date = Some(day(2024, 1, 10));
        let january_id = create_and_index(&fx, winter, "b").await;

        let mut summer = base_row(account, "<summer@x>");
        summer.subject = Some("newsletter".to_string());
        summer.sent_date = Some(day(2024, 6, 15));
        let june_id = create_and_index(&fx, summer, "b").await;

        let mut holidays = base_row(account, "<holidays@x>");
        holidays.subject = Some("newsletter".to_string());
        holidays.sent_date = Some(day(2024, 12, 20));
        let december_id = create_and_index(&fx, holidays, "b").await;

        let after = fx.service.search(user, "newsletter after:2024-06-01", 10, 0).await.unwrap();
        let mut after_ids = ids(&after);
        after_ids.sort_unstable();
        let mut expected = vec![june_id, december_id];
        expected.sort_unstable();
        assert_eq!(after_ids, expected, "after: keeps June and December");

        let before = fx.service.search(user, "newsletter before:2024-06-01", 10, 0).await.unwrap();
        assert_eq!(ids(&before), vec![january_id], "before: keeps only January");

        let on = fx.service.search(user, "newsletter date:2024-06-15", 10, 0).await.unwrap();
        assert_eq!(ids(&on), vec![june_id], "date: keeps only that day");
    }

    #[tokio::test]
    async fn attachment_filters() {
        let fx = fixture().await;
        let user = make_user(&fx.repo, "alice", "alice@example.com").await;
        let account = make_account(&fx.repo, user, "Primary").await;

        let mut with = base_row(account, "<with@x>");
        with.subject = Some("invoice".to_string());
        with.has_attachments = true;
        let with_id = create_and_index(&fx, with, "b").await;

        let mut without = base_row(account, "<without@x>");
        without.subject = Some("invoice".to_string());
        without.has_attachments = false;
        let without_id = create_and_index(&fx, without, "b").await;

        let has = fx.service.search(user, "invoice has:attachment", 10, 0).await.unwrap();
        assert_eq!(ids(&has), vec![with_id]);

        let none = fx.service.search(user, "invoice attachments:none", 10, 0).await.unwrap();
        assert_eq!(ids(&none), vec![without_id]);
    }

    #[tokio::test]
    async fn cross_user_scoping_blocks_other_users_docs() {
        let fx = fixture().await;
        let user_a = make_user(&fx.repo, "alice", "alice@example.com").await;
        let account_a = make_account(&fx.repo, user_a, "Alice Mail").await;
        let user_b = make_user(&fx.repo, "bob", "bob@example.com").await;
        let account_b = make_account(&fx.repo, user_b, "Bob Mail").await;

        let mut a_msg = base_row(account_a, "<a@x>");
        a_msg.subject = Some("secret".to_string());
        let a_id = create_and_index(&fx, a_msg, "b").await;

        let mut b_msg = base_row(account_b, "<b@x>");
        b_msg.subject = Some("secret".to_string());
        let _b_id = create_and_index(&fx, b_msg, "b").await;

        // A bare search as A returns only A's document.
        let plain = fx.service.search(user_a, "secret", 10, 0).await.unwrap();
        assert_eq!(ids(&plain), vec![a_id], "user A must not see user B's message");

        // Naming B's account by display name cannot widen A's scope: it resolves
        // to nothing within A's accounts, emptying the scope.
        let hinted = fx.service.search(user_a, "account:\"Bob Mail\" secret", 10, 0).await.unwrap();
        assert_eq!(hinted.total, 0);
        assert!(hinted.hits.is_empty(), "cross-user account hint must leak nothing");
    }

    #[tokio::test]
    async fn folder_post_filter_includes_and_excludes() {
        let fx = fixture().await;
        let user = make_user(&fx.repo, "alice", "alice@example.com").await;
        let account = make_account(&fx.repo, user, "Primary").await;
        let inbox = make_folder(&fx.repo, account, "INBOX").await;
        let archive = make_folder(&fx.repo, account, "Archive").await;

        let mut in_inbox = base_row(account, "<inbox@x>");
        in_inbox.subject = Some("meeting notes".to_string());
        let inbox_id = create_and_index(&fx, in_inbox, "b").await;
        make_location(&fx.repo, inbox_id, inbox, 1).await;

        let mut in_archive = base_row(account, "<archive@x>");
        in_archive.subject = Some("meeting notes".to_string());
        let archive_id = create_and_index(&fx, in_archive, "b").await;
        make_location(&fx.repo, archive_id, archive, 2).await;

        // Without a folder constraint, both match.
        let all = fx.service.search(user, "meeting", 10, 0).await.unwrap();
        assert_eq!(all.hits.len(), 2);

        // folder:INBOX keeps only the in-folder message.
        let only_inbox = fx.service.search(user, "meeting folder:INBOX", 10, 0).await.unwrap();
        assert_eq!(ids(&only_inbox), vec![inbox_id]);

        // Negated folder excludes the in-folder message.
        let not_inbox = fx.service.search(user, "meeting !folder:INBOX", 10, 0).await.unwrap();
        assert_eq!(ids(&not_inbox), vec![archive_id]);
    }

    #[tokio::test]
    async fn folder_hint_matches_path_substring_case_insensitively() {
        let fx = fixture().await;
        let user = make_user(&fx.repo, "alice", "alice@example.com").await;
        let account = make_account(&fx.repo, user, "Primary").await;
        // A nested folder whose leaf name is "Photography".
        let photography = make_folder(&fx.repo, account, "Hobbies/Photography").await;
        let work = make_folder(&fx.repo, account, "Work").await;

        let mut in_photography = base_row(account, "<photo@x>");
        in_photography.subject = Some("weekend shoot".to_string());
        let photo_id = create_and_index(&fx, in_photography, "b").await;
        make_location(&fx.repo, photo_id, photography, 1).await;

        let mut in_work = base_row(account, "<work@x>");
        in_work.subject = Some("weekend shoot".to_string());
        let work_id = create_and_index(&fx, in_work, "b").await;
        make_location(&fx.repo, work_id, work, 2).await;

        // A case-insensitive *substring* of the path resolves the folder:
        // "photography" matches "Hobbies/Photography" but not "Work".
        let hit = fx.service.search(user, "shoot folder:photography", 10, 0).await.unwrap();
        assert_eq!(ids(&hit), vec![photo_id], "folder hint matches the path substring, case-insensitively");

        // Sanity: a substring matching neither folder resolves to nothing.
        let none = fx.service.search(user, "shoot folder:gardening", 10, 0).await.unwrap();
        assert!(none.hits.is_empty(), "an unmatched folder substring keeps no messages");
    }

    #[tokio::test]
    async fn delete_account_removes_docs() {
        let fx = fixture().await;
        let user = make_user(&fx.repo, "alice", "alice@example.com").await;
        let account = make_account(&fx.repo, user, "Primary").await;

        let mut msg = base_row(account, "<hello@x>");
        msg.subject = Some("hello world".to_string());
        create_and_index(&fx, msg, "b").await;

        let before = fx.service.search(user, "hello", 10, 0).await.unwrap();
        assert_eq!(before.hits.len(), 1);

        fx.service.delete_account(account).await.unwrap();

        let after = fx.service.search(user, "hello", 10, 0).await.unwrap();
        assert!(after.hits.is_empty(), "deleted account's docs must be unsearchable");
        assert_eq!(after.total, 0);
    }

    #[tokio::test]
    async fn pagination_pages_partition_results() {
        let fx = fixture().await;
        let user = make_user(&fx.repo, "alice", "alice@example.com").await;
        let account = make_account(&fx.repo, user, "Primary").await;

        let mut all_ids = Vec::new();
        for i in 0..5 {
            let mut row = base_row(account, &format!("<r{i}@x>"));
            row.subject = Some("report".to_string());
            all_ids.push(create_and_index(&fx, row, "b").await);
        }

        // Page through 2 at a time. total is stable across pages; the pages
        // partition the full result set with no overlap or gaps.
        let p0 = fx.service.search(user, "report", 2, 0).await.unwrap();
        let p1 = fx.service.search(user, "report", 2, 2).await.unwrap();
        let p2 = fx.service.search(user, "report", 2, 4).await.unwrap();

        assert_eq!((p0.total, p1.total, p2.total), (5, 5, 5), "total is stable across pages");
        assert_eq!(p0.hits.len(), 2);
        assert_eq!(p1.hits.len(), 2);
        assert_eq!(p2.hits.len(), 1, "last page holds the remainder");

        let mut seen: Vec<u64> = ids(&p0).into_iter().chain(ids(&p1)).chain(ids(&p2)).collect();
        seen.sort_unstable();
        let mut expected = all_ids.clone();
        expected.sort_unstable();
        assert_eq!(seen, expected, "pages partition the result set");

        // An offset past the end yields an empty page but the true total.
        let past = fx.service.search(user, "report", 2, 99).await.unwrap();
        assert!(past.hits.is_empty());
        assert_eq!(past.total, 5);
    }

    #[tokio::test]
    async fn empty_query_lists_all_scoped_docs_only() {
        let fx = fixture().await;
        let user_a = make_user(&fx.repo, "alice", "alice@example.com").await;
        let account_a = make_account(&fx.repo, user_a, "Primary").await;
        let user_b = make_user(&fx.repo, "bob", "bob@example.com").await;
        let account_b = make_account(&fx.repo, user_b, "Bob").await;

        let mut m1 = base_row(account_a, "<m1@x>");
        m1.subject = Some("anything".to_string());
        let m1_id = create_and_index(&fx, m1, "b").await;
        let mut m2 = base_row(account_a, "<m2@x>");
        m2.subject = Some("other".to_string());
        let m2_id = create_and_index(&fx, m2, "b").await;

        // B's doc must never surface for A's empty (list-all) query.
        let mut b = base_row(account_b, "<b@x>");
        b.subject = Some("bsecret".to_string());
        create_and_index(&fx, b, "b").await;

        let results = fx.service.search(user_a, "", 10, 0).await.unwrap();
        let mut got = ids(&results);
        got.sort_unstable();
        let mut expected = vec![m1_id, m2_id];
        expected.sort_unstable();
        assert_eq!(got, expected, "empty query lists exactly the user's own docs");
        assert_eq!(results.total, 2, "scope-only Must still bounds the count");
    }

    #[tokio::test]
    async fn date_filter_midnight_boundary_is_half_open() {
        let fx = fixture().await;
        let user = make_user(&fx.repo, "alice", "alice@example.com").await;
        let account = make_account(&fx.repo, user, "Primary").await;

        // Sent at exactly 00:00:00 UTC on 2024-06-15 — the boundary most likely
        // to regress under the half-open [D, D+1) day intervals.
        let midnight = Utc.with_ymd_and_hms(2024, 6, 15, 0, 0, 0).unwrap();
        let mut row = base_row(account, "<mid@x>");
        row.subject = Some("newsletter".to_string());
        row.sent_date = Some(midnight);
        let id = create_and_index(&fx, row, "b").await;

        // Midnight of D belongs to day D.
        let on = fx.service.search(user, "newsletter date:2024-06-15", 10, 0).await.unwrap();
        assert_eq!(ids(&on), vec![id], "00:00 of D is inside date:D");
        let before = fx.service.search(user, "newsletter before:2024-06-15", 10, 0).await.unwrap();
        assert!(before.hits.is_empty(), "00:00 of D is not strictly before D");
        let after_prev = fx.service.search(user, "newsletter after:2024-06-14", 10, 0).await.unwrap();
        assert_eq!(ids(&after_prev), vec![id], "00:00 of D is after D-1");
        let on_prev = fx.service.search(user, "newsletter date:2024-06-14", 10, 0).await.unwrap();
        assert!(on_prev.hits.is_empty(), "00:00 of D is not inside date:D-1");
    }
}
