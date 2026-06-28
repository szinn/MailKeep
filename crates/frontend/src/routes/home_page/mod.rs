mod account_actions;
mod delete_account_modal;
mod edit_folders_modal;
mod format;

use account_actions::set_account_enabled;
use delete_account_modal::DeleteAccountModal;
use dioxus::prelude::*;
use edit_folders_modal::EditFoldersModal;
use format::{status_icon_color, status_tooltip};
#[cfg(feature = "server")]
use {crate::routes::server_helpers::authenticated_user, crate::server::AuthSession};

use crate::{
    Route,
    components::ACCOUNTS_REVISION,
    routes::account_add_page::{dtos::AccountSummaryDto, list_accounts},
};

#[get(
    "/api/v1/home/context",
    auth_session: axum::Extension<AuthSession>,
)]
async fn get_home_context() -> Result<(), ServerFnError> {
    authenticated_user(&auth_session)?;
    Ok(())
}

#[component]
pub(crate) fn HomePage() -> Element {
    let navigator = use_navigator();
    let auth = use_server_future(get_home_context)?;

    use_effect(move || {
        if let Some(Err(_)) = auth() {
            navigator.replace(Route::LandingPage { login_failed: None });
        }
    });

    let refresh = use_signal(|| 0u32);
    let accounts = use_resource(move || {
        let _ = refresh(); // subscribe: bumping refresh re-runs list_accounts
        let _ = ACCOUNTS_REVISION(); // MK-19: server-pushed account changes
        async move { list_accounts().await }
    });

    rsx! {
        div { class: "flex h-full flex-1",
            // Left panel — account list
            nav { class: "flex w-72 shrink-0 flex-col border-r border-gray-200 bg-white dark:border-slate-700 dark:bg-slate-800",
                div { class: "flex-1 overflow-auto py-2",
                    match accounts() {
                        None => rsx! {
                            div { class: "px-4 py-3 text-sm text-gray-400 dark:text-slate-500", "Loading…" }
                        },
                        Some(Err(e)) => rsx! {
                            div { class: "px-4 py-3 text-sm text-red-600 dark:text-red-400", "{e}" }
                        },
                        Some(Ok(rows)) if rows.is_empty() => rsx! {
                            div { class: "px-4 py-6 text-center",
                                p { class: "text-sm text-gray-400 dark:text-slate-500 mb-3", "No accounts yet." }
                                button {
                                    class: "rounded-lg bg-indigo-600 px-4 py-2 text-sm font-medium text-white hover:bg-indigo-700",
                                    onclick: move |_| { navigator.push(Route::AccountAddPage {}); },
                                    "Add your first account"
                                }
                            }
                        },
                        Some(Ok(rows)) => rsx! {
                            ul { class: "divide-y divide-gray-100 dark:divide-slate-700",
                                for acc in rows {
                                    AccountRow { account: acc, refresh }
                                }
                            }
                        },
                    }
                }
                div { class: "border-t border-gray-200 p-3 dark:border-slate-700",
                    button {
                        class: "w-full rounded-lg bg-indigo-600 px-4 py-2 text-sm font-medium text-white hover:bg-indigo-700",
                        onclick: move |_| { navigator.push(Route::AccountAddPage {}); },
                        "+ Add account"
                    }
                }
            }
            // Right panel — blank
            div { class: "flex-1 overflow-auto p-8" }
        }
    }
}

#[component]
fn AccountRow(account: AccountSummaryDto, refresh: Signal<u32>) -> Element {
    let tooltip = status_tooltip(&account.status, account.last_synced.as_deref(), account.last_error.as_deref());

    let mut menu_open = use_signal(|| false);
    let mut busy = use_signal(|| false);
    let mut row_error: Signal<Option<String>> = use_signal(|| None);
    let mut show_delete = use_signal(|| false);
    let mut show_edit = use_signal(|| false);
    let enabled = account.status != "Disabled";
    let token = account.token.clone();

    rsx! {
        li { key: "{account.token}", class: "flex items-center gap-3 px-4 py-3",
            StatusIcon { status: account.status.clone(), tooltip }
            div { class: "flex-1 min-w-0",
                div { class: "text-sm font-medium text-gray-900 dark:text-slate-100 truncate", "{account.display_name}" }
                div { class: "text-xs text-gray-500 dark:text-slate-400 truncate", "{account.email}" }
                if let Some(msg) = row_error() {
                    div { class: "mt-1 text-xs text-red-600 dark:text-red-400", "{msg}" }
                }
            }
            div { class: "relative shrink-0",
                button {
                    class: "rounded p-1 text-gray-400 hover:text-gray-600 hover:bg-gray-100 dark:hover:bg-slate-700",
                    title: "Actions",
                    disabled: busy(),
                    onclick: move |_| menu_open.toggle(),
                    "⋯"
                }
                if menu_open() {
                    div { class: "fixed inset-0 z-40", onclick: move |_| menu_open.set(false) }
                    div { class: "absolute right-0 top-full mt-1 w-40 bg-white dark:bg-slate-800 rounded-lg shadow-lg py-1 z-50 border dark:border-slate-700 text-sm",
                        button {
                            class: "w-full text-left px-4 py-2 text-gray-700 dark:text-slate-200 hover:bg-gray-100 dark:hover:bg-slate-700",
                            onclick: {
                                let token = token.clone();
                                move |_| {
                                    let token = token.clone();
                                    menu_open.set(false);
                                    busy.set(true);
                                    row_error.set(None);
                                    spawn(async move {
                                        match set_account_enabled(token, !enabled).await {
                                            Ok(()) => { refresh += 1; }
                                            Err(e) => row_error.set(Some(e.to_string())),
                                        }
                                        busy.set(false);
                                    });
                                }
                            },
                            if enabled { "Disable" } else { "Enable" }
                        }
                        button {
                            class: "w-full text-left px-4 py-2 text-gray-700 dark:text-slate-200 hover:bg-gray-100 dark:hover:bg-slate-700",
                            onclick: move |_| { menu_open.set(false); show_edit.set(true); },
                            "Edit Folders"
                        }
                        button {
                            class: "w-full text-left px-4 py-2 text-red-600 dark:text-red-400 hover:bg-gray-100 dark:hover:bg-slate-700",
                            onclick: move |_| { menu_open.set(false); show_delete.set(true); },
                            "Delete"
                        }
                    }
                }
            }
        }
        if show_delete() {
            DeleteAccountModal {
                token: account.token.clone(),
                display_name: account.display_name.clone(),
                on_close: move |()| show_delete.set(false),
                on_deleted: move |()| { show_delete.set(false); refresh += 1; },
            }
        }
        if show_edit() {
            EditFoldersModal {
                token: account.token.clone(),
                on_close: move |()| show_edit.set(false),
                on_saved: move |()| { show_edit.set(false); refresh += 1; },
            }
        }
    }
}

/// Per-state status glyph: color + shape (Heroicons outline). Syncing spins.
/// The wrapping `span` carries the hover tooltip via the native `title`.
#[component]
fn StatusIcon(status: String, tooltip: String) -> Element {
    let color = status_icon_color(&status);
    let spin = if status == "Syncing" { "animate-spin" } else { "" };
    rsx! {
        span { class: "shrink-0", title: "{tooltip}",
            match status.as_str() {
                "Idle" => rsx! {
                    svg { class: "h-4 w-4 {color}", fill: "none", view_box: "0 0 24 24", stroke_width: "1.5", stroke: "currentColor",
                        path { stroke_linecap: "round", stroke_linejoin: "round", d: "M9 12.75 11.25 15 15 9.75M21 12a9 9 0 1 1-18 0 9 9 0 0 1 18 0Z" }
                    }
                },
                "Syncing" => rsx! {
                    svg { class: "h-4 w-4 {color} {spin}", fill: "none", view_box: "0 0 24 24", stroke_width: "1.5", stroke: "currentColor",
                        path { stroke_linecap: "round", stroke_linejoin: "round", d: "M16.023 9.348h4.992v-.001M2.985 19.644v-4.992m0 0h4.992m-4.993 0 3.181 3.183a8.25 8.25 0 0 0 13.803-3.7M4.031 9.865a8.25 8.25 0 0 1 13.803-3.7l3.181 3.182m0-4.991v4.99" }
                    }
                },
                "Error" => rsx! {
                    svg { class: "h-4 w-4 {color}", fill: "none", view_box: "0 0 24 24", stroke_width: "1.5", stroke: "currentColor",
                        path { stroke_linecap: "round", stroke_linejoin: "round", d: "M12 9v3.75m-9.303 3.376c-.866 1.5.217 3.374 1.948 3.374h14.71c1.73 0 2.813-1.874 1.948-3.374L13.949 3.378c-.866-1.5-3.032-1.5-3.898 0L2.697 16.126ZM12 15.75h.007v.008H12v-.008Z" }
                    }
                },
                "Disabled" => rsx! {
                    svg { class: "h-4 w-4 {color}", fill: "none", view_box: "0 0 24 24", stroke_width: "1.5", stroke: "currentColor",
                        path { stroke_linecap: "round", stroke_linejoin: "round", d: "M14.25 9v6m-4.5 0V9M21 12a9 9 0 1 1-18 0 9 9 0 0 1 18 0Z" }
                    }
                },
                // PendingFirstSync and any unknown state → clock
                _ => rsx! {
                    svg { class: "h-4 w-4 {color}", fill: "none", view_box: "0 0 24 24", stroke_width: "1.5", stroke: "currentColor",
                        path { stroke_linecap: "round", stroke_linejoin: "round", d: "M12 6v6h4.5m4.5 0a9 9 0 1 1-18 0 9 9 0 0 1 18 0Z" }
                    }
                },
            }
        }
    }
}
