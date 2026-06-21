use dioxus::prelude::*;

use super::folder_tree::FolderTree;

/// Presentation component for the folder selection tree. The page owns the
/// `Signal<FolderTree>` and the mutation logic; this component renders rows and
/// emits toggle / select-all events upward.
///
/// Dioxus has no reactive `indeterminate` attribute on `<input>`, so the
/// tri-state checkbox visual is applied imperatively via `document::eval` in a
/// `use_effect` keyed on the tree.
#[component]
pub(crate) fn FolderPicker(tree: Signal<FolderTree>, dimmed: bool, on_toggle: EventHandler<usize>, on_select_all: EventHandler<bool>) -> Element {
    // Imperatively set each checkbox's `indeterminate` DOM property. We set it
    // explicitly (true or false) for every node each pass so stale states are
    // cleared. Checkbox ids are stable: `folder-cb-{idx}`.
    use_effect(move || {
        use std::fmt::Write as _;
        let t = tree();
        let mut js = String::new();
        for idx in 0..t.nodes.len() {
            let ind = t.is_indeterminate(idx);
            let _ = write!(
                js,
                "{{var el=document.getElementById('folder-cb-{idx}'); if(el){{el.indeterminate={};}}}}",
                if ind { "true" } else { "false" }
            );
        }
        if !js.is_empty() {
            spawn(async move {
                let _ = document::eval(&js).await;
            });
        }
    });

    let t = tree();
    let count = t.selected_count();
    let container = if dimmed { "mt-6 opacity-50 pointer-events-none" } else { "mt-6" };

    rsx! {
        div { class: "{container}",
            div { class: "flex items-center justify-between mb-2",
                h3 { class: "text-sm font-semibold text-gray-900 dark:text-slate-100", "Folders to archive" }
                div { class: "flex items-center gap-3 text-xs",
                    span { class: "text-gray-500 dark:text-slate-400", "{count} selected" }
                    button {
                        class: "text-indigo-600 hover:underline dark:text-indigo-400",
                        r#type: "button",
                        onclick: move |_| on_select_all.call(true),
                        "Select all"
                    }
                    button {
                        class: "text-indigo-600 hover:underline dark:text-indigo-400",
                        r#type: "button",
                        onclick: move |_| on_select_all.call(false),
                        "Select none"
                    }
                }
            }
            div { class: "rounded-lg border border-gray-200 divide-y divide-gray-100 max-h-80 overflow-auto dark:border-slate-700 dark:divide-slate-700",
                for (idx, node) in t.nodes.iter().enumerate() {
                    {
                        let pad = format!("padding-left: {}px", 12 + node.depth * 20);
                        let selected = node.selected;
                        let cb_id = format!("folder-cb-{idx}");
                        let badge = badge_for(node.special_use.as_deref());
                        rsx! {
                            div {
                                key: "{node.path}",
                                class: "flex items-center gap-2 px-3 py-2",
                                style: "{pad}",
                                input {
                                    id: "{cb_id}",
                                    r#type: "checkbox",
                                    class: "h-4 w-4 rounded border-gray-300 text-indigo-600 focus:ring-indigo-500 dark:border-slate-600",
                                    checked: selected,
                                    onchange: move |_| on_toggle.call(idx),
                                }
                                span { class: "text-base", { icon_for(node.special_use.as_deref()) } }
                                span { class: "text-sm text-gray-900 dark:text-slate-100", "{node.name}" }
                                if let Some(b) = badge {
                                    span { class: "ml-1 text-xs px-1.5 py-0.5 rounded bg-gray-100 text-gray-600 dark:bg-slate-700 dark:text-slate-300",
                                        "{b}"
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

/// Maps a folder's special-use attribute to a distinct glyph. Unknown / none
/// folders get a generic folder icon.
fn icon_for(su: Option<&str>) -> &'static str {
    match su {
        Some("inbox") => "📥",
        Some("sent") => "📤",
        Some("drafts") => "📝",
        Some("trash") => "🗑",
        Some("archive") => "🗄",
        Some("junk") => "⚠",
        Some("all") => "🗂",
        _ => "📁",
    }
}

/// Badge label for known special-use folders; `None` for plain folders (no
/// badge shown).
fn badge_for(su: Option<&str>) -> Option<&'static str> {
    match su {
        Some("inbox") => Some("Inbox"),
        Some("sent") => Some("Sent"),
        Some("drafts") => Some("Drafts"),
        Some("trash") => Some("Trash"),
        Some("archive") => Some("Archive"),
        Some("junk") => Some("Junk"),
        Some("all") => Some("All"),
        _ => None,
    }
}
