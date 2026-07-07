use dioxus::prelude::*;
#[cfg(feature = "server")]
use {
    crate::components::message_to_row,
    crate::routes::server_helpers::{authenticated_user, to_server_err},
    crate::server::AuthSession,
    mk_core::{
        CoreServices,
        account::{AccountId, AccountToken},
    },
    std::sync::Arc,
};

use crate::components::{MessageRow, MessageRowDto};

/// Default page size for the account message list.
pub(crate) const PAGE_SIZE: u32 = 50;

/// List one page of the selected account's messages (newest first), scoped to
/// the authenticated user. Mirrors `get_account_folders`: the ownership gate
/// (`get_account`) also 404s a foreign/unknown account before any data is read.
#[post(
    "/api/v1/home/messages",
    auth_session: axum::Extension<AuthSession>,
    core_services: axum::Extension<Arc<CoreServices>>
)]
pub(crate) async fn list_messages(account_token: String, limit: u32, offset: u32) -> Result<Vec<MessageRowDto>, ServerFnError> {
    let user = authenticated_user(&auth_session)?;
    let account_id: AccountId = account_token.parse::<AccountToken>().map_err(to_server_err)?.id();
    // Ownership gate (also 404s a foreign/unknown account).
    core_services.account_service.get_account(user.id(), account_id).await.map_err(to_server_err)?;
    let messages = core_services
        .message_service
        .list_messages_for_account(account_id, limit, offset)
        .await
        .map_err(to_server_err)?;
    Ok(messages.iter().map(message_to_row).collect())
}

/// Fetch one page, flattening the server error to a display string.
async fn fetch_page(account_token: String, offset: u32) -> Result<Vec<MessageRowDto>, String> {
    list_messages(account_token, PAGE_SIZE, offset).await.map_err(|e| e.to_string())
}

/// Right-panel message list for the selected account. HomePage keys this
/// component by the account token, so selecting a different account remounts it
/// and re-runs the initial load.
#[component]
pub(crate) fn MessageList(account_token: String) -> Element {
    let mut rows = use_signal(Vec::<MessageRowDto>::new);
    let mut error = use_signal(|| Option::<String>::None);
    let mut loading = use_signal(|| true);
    let mut reached_end = use_signal(|| false);

    // Initial load on mount.
    {
        let token = account_token.clone();
        use_future(move || {
            let token = token.clone();
            async move {
                loading.set(true);
                match fetch_page(token, 0).await {
                    Ok(page) => {
                        reached_end.set((page.len() as u32) < PAGE_SIZE);
                        rows.set(page);
                    }
                    Err(e) => error.set(Some(e)),
                }
                loading.set(false);
            }
        })
    };

    let load_more = {
        let token = account_token.clone();
        move |_: MouseEvent| {
            if loading() {
                return;
            }
            loading.set(true);
            error.set(None);
            let token = token.clone();
            let offset = rows.read().len() as u32;
            spawn(async move {
                match fetch_page(token, offset).await {
                    Ok(page) => {
                        reached_end.set((page.len() as u32) < PAGE_SIZE);
                        rows.write().extend(page); // append — do not blank existing rows
                    }
                    Err(e) => error.set(Some(e)),
                }
                loading.set(false);
            });
        }
    };

    let refresh = {
        let token = account_token.clone();
        move |_: MouseEvent| {
            if loading() {
                return;
            }
            loading.set(true);
            error.set(None);
            let token = token.clone();
            spawn(async move {
                match fetch_page(token, 0).await {
                    Ok(page) => {
                        reached_end.set((page.len() as u32) < PAGE_SIZE);
                        rows.set(page);
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
                h2 { class: "text-sm font-semibold text-gray-900 dark:text-slate-100", "Messages" }
                button {
                    class: "rounded p-1 text-gray-400 hover:bg-gray-100 hover:text-gray-600 dark:hover:bg-slate-700",
                    title: "Refresh",
                    disabled: loading(),
                    onclick: refresh,
                    "⟳"
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
                            div { class: "px-4 py-6 text-center text-sm text-gray-400 dark:text-slate-500", "No messages." }
                        }
                    } else if is_empty {
                        // Empty because the load errored — the error banner above says enough.
                        rsx! {}
                    } else {
                        rsx! {
                            ul { class: "divide-y divide-gray-100 dark:divide-slate-700",
                                for row in rows.read().iter().cloned() {
                                    MessageRow {
                                        key: "{row.token}",
                                        row,
                                        on_open: move |token: String| {
                                            *crate::components::OPEN_MESSAGE.write() = Some(token);
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
