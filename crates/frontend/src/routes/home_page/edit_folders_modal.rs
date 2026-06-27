use dioxus::prelude::*;

use crate::routes::{
    account_add_page::{dtos::FolderEnabledDto, folder_picker::icon_for},
    home_page::account_actions::{get_account_folders, set_account_folders_enabled},
};

#[component]
pub(crate) fn EditFoldersModal(token: String, on_close: EventHandler<()>, on_saved: EventHandler<()>) -> Element {
    let load_token = token.clone();
    let folders = use_resource(move || {
        let t = load_token.clone();
        async move { get_account_folders(t).await }
    });

    // Edited enabled-state, keyed by folder token. Seeded from the load.
    let mut edited: Signal<Vec<FolderEnabledDto>> = use_signal(Vec::new);
    let mut busy = use_signal(|| false);
    let mut error_msg: Signal<Option<String>> = use_signal(|| None);

    // Seed `edited` once folders arrive.
    use_effect(move || {
        if let Some(Ok(list)) = folders()
            && edited.read().is_empty()
        {
            edited.set(
                list.iter()
                    .map(|f| FolderEnabledDto {
                        token: f.token.clone(),
                        enabled: f.enabled,
                    })
                    .collect(),
            );
        }
    });

    let any_enabled = edited.read().iter().any(|f| f.enabled);

    rsx! {
        div { class: "fixed inset-0 z-50 flex items-center justify-center bg-black/40", onclick: move |_| on_close(()),
            div { class: "bg-white dark:bg-slate-800 rounded-xl shadow-xl w-full max-w-md mx-4 p-6", onclick: |e| e.stop_propagation(),
                h2 { class: "text-lg font-semibold text-gray-900 dark:text-slate-100 mb-3", "Edit folders" }
                if let Some(msg) = error_msg() {
                    div { class: "mb-3 p-3 bg-red-50 dark:bg-red-950 border border-red-200 dark:border-red-800 text-red-700 dark:text-red-300 rounded-lg text-sm", "{msg}" }
                }
                div { class: "max-h-80 overflow-auto rounded-lg border border-gray-200 divide-y divide-gray-100 dark:border-slate-700 dark:divide-slate-700",
                    match folders() {
                        None => rsx! { div { class: "px-3 py-2 text-sm text-gray-400", "Loading…" } },
                        Some(Err(e)) => rsx! { div { class: "px-3 py-2 text-sm text-red-600", "{e}" } },
                        Some(Ok(list)) => rsx! {
                            for (idx, f) in list.iter().enumerate() {
                                div { key: "{f.token}", class: "flex items-center gap-2 px-3 py-2",
                                    input {
                                        r#type: "checkbox",
                                        class: "h-4 w-4 rounded border-gray-300 text-indigo-600",
                                        checked: edited.read().get(idx).map_or(f.enabled, |e| e.enabled),
                                        onchange: move |evt| {
                                            let mut v = edited.write();
                                            if let Some(item) = v.get_mut(idx) { item.enabled = evt.checked(); }
                                        },
                                    }
                                    span { class: "text-base", { icon_for(f.special_use.as_deref()) } }
                                    span { class: "text-sm text-gray-900 dark:text-slate-100 truncate", "{f.path}" }
                                }
                            }
                        },
                    }
                }
                div { class: "mt-4 flex justify-end gap-2",
                    button {
                        class: "px-4 py-2 text-sm rounded-lg border border-gray-300 dark:border-slate-600 text-gray-700 dark:text-slate-200",
                        disabled: busy(),
                        onclick: move |_| on_close(()),
                        "Cancel"
                    }
                    button {
                        class: "px-4 py-2 text-sm rounded-lg bg-indigo-600 hover:bg-indigo-700 disabled:bg-indigo-400 text-white font-medium",
                        disabled: busy() || !any_enabled,
                        onclick: {
                            let token = token.clone();
                            move |_| {
                                let token = token.clone();
                                let payload = edited.read().clone();
                                busy.set(true);
                                error_msg.set(None);
                                spawn(async move {
                                    match set_account_folders_enabled(token, payload).await {
                                        Ok(()) => on_saved(()),
                                        Err(e) => { error_msg.set(Some(e.to_string())); busy.set(false); }
                                    }
                                });
                            }
                        },
                        if busy() { "Saving…" } else { "Save" }
                    }
                }
            }
        }
    }
}
