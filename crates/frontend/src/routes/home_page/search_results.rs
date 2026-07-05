use dioxus::prelude::*;
#[cfg(feature = "server")]
use {
    crate::components::message_to_row,
    crate::routes::server_helpers::{authenticated_user, to_server_err},
    crate::server::AuthSession,
    mk_core::{
        CoreServices,
        message::{Message, MessageId},
        search::SearchHit,
    },
    std::{collections::HashMap, sync::Arc},
};

use crate::components::{MessageRow, MessageRowDto};

/// Default page size for a search results page.
pub(crate) const SEARCH_PAGE_SIZE: u32 = 25;

/// Join ranked `hits` to fetched `messages`, preserving Tantivy rank order and
/// dropping any hit whose message could not be resolved (deleted between index
/// and fetch). Pure — unit-tested without axum plumbing.
#[cfg(feature = "server")]
fn order_rows_by_hits(hits: &[SearchHit], messages: Vec<Message>) -> Vec<MessageRowDto> {
    let by_id: HashMap<MessageId, Message> = messages.into_iter().map(|m| (m.id, m)).collect();
    hits.iter().filter_map(|h| by_id.get(&h.message_id)).map(message_to_row).collect()
}

/// Execute one page of full-text search for the authenticated user and return
/// ranked message rows. Per-user scoping is enforced by
/// `SearchService::search`; the `get_messages_by_ids` join is additionally
/// ownership-scoped.
#[post(
    "/api/v1/home/search",
    auth_session: axum::Extension<AuthSession>,
    core_services: axum::Extension<Arc<CoreServices>>
)]
pub(crate) async fn search_messages(query: String, limit: u32, offset: u32) -> Result<(Vec<MessageRowDto>, u32), ServerFnError> {
    let user = authenticated_user(&auth_session)?;
    let results = core_services
        .search_service
        .search(user.id(), &query, limit, offset)
        .await
        .map_err(to_server_err)?;
    let hit_count = results.hits.len() as u32;
    let ids: Vec<MessageId> = results.hits.iter().map(|h| h.message_id).collect();
    let msgs = core_services
        .message_service
        .get_messages_by_ids(user.id(), &ids)
        .await
        .map_err(to_server_err)?;
    Ok((order_rows_by_hits(&results.hits, msgs), hit_count))
}

/// Fetch one page, flattening the server error to a display string.
async fn fetch_page(query: String, offset: u32) -> Result<(Vec<MessageRowDto>, u32), String> {
    search_messages(query, SEARCH_PAGE_SIZE, offset).await.map_err(|e| e.to_string())
}

/// Right-panel search results, keyed by the submitted query so a new search
/// remounts and re-runs the initial load. Near-clone of `MessageList` with a
/// query header and a "clear search" affordance.
#[component]
pub(crate) fn SearchResults(query: String) -> Element {
    let mut rows = use_signal(Vec::<MessageRowDto>::new);
    let mut error = use_signal(|| Option::<String>::None);
    let mut loading = use_signal(|| true);
    let mut reached_end = use_signal(|| false);
    let mut hit_offset = use_signal(|| 0u32); // FTS hit cursor — independent of resolved-row count

    // Initial load on mount.
    {
        let q = query.clone();
        use_future(move || {
            let q = q.clone();
            async move {
                loading.set(true);
                match fetch_page(q, 0).await {
                    Ok((page, hit_count)) => {
                        reached_end.set(hit_count < SEARCH_PAGE_SIZE);
                        hit_offset.set(hit_count);
                        rows.set(page);
                    }
                    Err(e) => error.set(Some(e)),
                }
                loading.set(false);
            }
        })
    };

    let load_more = {
        let q = query.clone();
        move |_: MouseEvent| {
            if loading() {
                return;
            }
            loading.set(true);
            error.set(None);
            let q = q.clone();
            let offset = hit_offset();
            spawn(async move {
                match fetch_page(q, offset).await {
                    Ok((page, hit_count)) => {
                        reached_end.set(hit_count < SEARCH_PAGE_SIZE);
                        hit_offset.with_mut(|o| *o += hit_count);
                        rows.write().extend(page); // append — do not blank existing rows
                    }
                    Err(e) => error.set(Some(e)),
                }
                loading.set(false);
            });
        }
    };

    rsx! {
        div { class: "flex h-full flex-col",
            div { class: "flex items-center justify-between border-b border-gray-200 px-4 py-3 dark:border-slate-700",
                h2 { class: "truncate text-sm font-semibold text-gray-900 dark:text-slate-100",
                    "Search: "
                    span { class: "font-normal text-gray-500 dark:text-slate-400", "{query}" }
                }
                button {
                    class: "shrink-0 rounded p-1 text-gray-400 hover:bg-gray-100 hover:text-gray-600 dark:hover:bg-slate-700",
                    title: "Clear search",
                    onclick: move |_| {
                        *crate::components::ACTIVE_SEARCH.write() = None;
                        *crate::components::SEARCH_QUERY.write() = String::new();
                    },
                    "✕"
                }
            }
            div { class: "flex-1 overflow-auto",
                if let Some(e) = error() {
                    div { class: "px-4 py-3 text-sm text-red-600 dark:text-red-400", "{e}" }
                }
                {
                    let is_empty = rows.read().is_empty();
                    let has_error = error().is_some();
                    if is_empty && loading() {
                        rsx! {
                            div { class: "px-4 py-6 text-sm text-gray-400 dark:text-slate-500", "Loading…" }
                        }
                    } else if is_empty && !has_error {
                        rsx! {
                            div { class: "px-4 py-6 text-center text-sm text-gray-400 dark:text-slate-500", "No matches." }
                        }
                    } else if is_empty {
                        rsx! {}
                    } else {
                        rsx! {
                            ul { class: "divide-y divide-gray-100 dark:divide-slate-700",
                                for row in rows.read().iter().cloned() {
                                    MessageRow {
                                        key: "{row.token}",
                                        row,
                                        on_open: move |_token: String| {
                                            // MK-23 viewer navigation seam — no-op for now.
                                        },
                                    }
                                }
                            }
                            if !reached_end() {
                                div { class: "p-3",
                                    button {
                                        class: "w-full rounded-lg border border-gray-200 px-4 py-2 text-sm text-gray-600 hover:bg-gray-50 dark:border-slate-700 dark:text-slate-300 dark:hover:bg-slate-700",
                                        disabled: loading(),
                                        onclick: load_more,
                                        if loading() { "Loading…" } else { "Load more" }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[cfg(all(test, feature = "server"))]
mod tests {
    use mk_core::{
        message::{MessageBuilder, MessageToken},
        types::{ContentHash, EmailAddress},
    };

    use super::*;

    fn msg(id: u64) -> Message {
        MessageBuilder::default()
            .id(id)
            .version(1u64)
            .token(MessageToken::new(id))
            .account_id(1u64)
            .rfc822_message_id(format!("<{id}@x.com>"))
            .content_hash(ContentHash::compute(id.to_string().as_bytes()))
            .from_address(EmailAddress::new("a@x.com").unwrap())
            .snippet(String::new())
            .size_bytes(1i64)
            .build()
            .unwrap()
    }

    fn hit(message_id: u64) -> SearchHit {
        SearchHit {
            message_id,
            account_id: 1,
            snippet: String::new(),
        }
    }

    #[test]
    fn preserves_hit_order_and_drops_unresolved() {
        // Hits ranked 3,1,2 plus a hit (99) with no matching message.
        let hits = vec![hit(3), hit(1), hit(2), hit(99)];
        // Messages returned out of order and missing id 99.
        let messages = vec![msg(1), msg(2), msg(3)];
        let rows = order_rows_by_hits(&hits, messages);
        let tokens: Vec<String> = rows.iter().map(|r| r.token.clone()).collect();
        assert_eq!(
            tokens,
            vec![
                MessageToken::new(3).to_string(),
                MessageToken::new(1).to_string(),
                MessageToken::new(2).to_string(),
            ],
            "output is in hit rank order and drops the unresolved id 99"
        );
    }
}
