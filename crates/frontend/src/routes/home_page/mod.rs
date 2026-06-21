use dioxus::prelude::*;
#[cfg(feature = "server")]
use {crate::routes::server_helpers::authenticated_user, crate::server::AuthSession};

use crate::{
    Route,
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

    let accounts = use_resource(move || async move { list_accounts().await });

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
                            div { class: "px-4 py-6 text-center text-sm text-gray-400 dark:text-slate-500", "No accounts yet." }
                        },
                        Some(Ok(rows)) => rsx! {
                            ul { class: "divide-y divide-gray-100 dark:divide-slate-700",
                                for acc in rows {
                                    AccountRow { account: acc }
                                }
                            }
                        },
                    }
                }
                div { class: "border-t border-gray-200 p-3 dark:border-slate-700",
                    button {
                        class: "w-full rounded-lg bg-indigo-600 px-4 py-2 text-sm font-medium text-white hover:bg-indigo-700",
                        onclick: move |_| {
                            navigator.push(Route::AccountAddPage {});
                        },
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
fn AccountRow(account: AccountSummaryDto) -> Element {
    rsx! {
        li { key: "{account.token}", class: "px-4 py-3",
            div { class: "text-sm font-medium text-gray-900 dark:text-slate-100", "{account.display_name}" }
            div { class: "text-xs text-gray-500 dark:text-slate-400", "{account.email}" }
        }
    }
}
