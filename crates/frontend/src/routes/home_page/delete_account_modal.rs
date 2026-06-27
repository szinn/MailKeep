use dioxus::prelude::*;

use crate::routes::home_page::account_actions::delete_account;

#[component]
pub(crate) fn DeleteAccountModal(token: String, display_name: String, on_close: EventHandler<()>, on_deleted: EventHandler<()>) -> Element {
    let mut busy = use_signal(|| false);
    let mut error_msg: Signal<Option<String>> = use_signal(|| None);

    rsx! {
        div {
            class: "fixed inset-0 z-50 flex items-center justify-center bg-black/40",
            onclick: move |_| on_close(()),
            div {
                class: "bg-white dark:bg-slate-800 rounded-xl shadow-xl w-full max-w-md mx-4 p-6",
                onclick: |e| e.stop_propagation(),
                h2 { class: "text-lg font-semibold text-gray-900 dark:text-slate-100 mb-2", "Delete account" }
                p { class: "text-sm text-gray-600 dark:text-slate-300 mb-4",
                    "Permanently delete "{display_name}"? This removes its database rows and all archived message data on disk. This cannot be undone."
                }
                if let Some(msg) = error_msg() {
                    div { class: "mb-3 p-3 bg-red-50 dark:bg-red-950 border border-red-200 dark:border-red-800 text-red-700 dark:text-red-300 rounded-lg text-sm", "{msg}" }
                }
                div { class: "flex justify-end gap-2",
                    button {
                        class: "px-4 py-2 text-sm rounded-lg border border-gray-300 dark:border-slate-600 text-gray-700 dark:text-slate-200",
                        disabled: busy(),
                        onclick: move |_| on_close(()),
                        "Cancel"
                    }
                    button {
                        class: "px-4 py-2 text-sm rounded-lg bg-red-600 hover:bg-red-700 disabled:bg-red-400 text-white font-medium",
                        disabled: busy(),
                        onclick: {
                            let token = token.clone();
                            move |_| {
                                let token = token.clone();
                                busy.set(true);
                                error_msg.set(None);
                                spawn(async move {
                                    match delete_account(token).await {
                                        Ok(()) => on_deleted(()),
                                        Err(e) => { error_msg.set(Some(e.to_string())); busy.set(false); }
                                    }
                                });
                            }
                        },
                        if busy() { "Deleting…" } else { "Delete" }
                    }
                }
            }
        }
    }
}
