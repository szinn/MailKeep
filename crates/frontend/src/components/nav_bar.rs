use dioxus::prelude::*;

#[cfg(feature = "server")]
use crate::server::AuthSession;
use crate::{
    Route,
    components::{
        SELECTED_ACCOUNT, THEME_MODE,
        search::{ACTIVE_SEARCH, PLACEHOLDER_TIPS, SEARCH_QUERY, apply_completion, compute_completion, next_cycle_input},
        set_theme_preference,
    },
};

#[get("/api/v1/user/is_admin", auth_session: axum::Extension<AuthSession>)]
pub(crate) async fn get_is_admin() -> Result<bool, ServerFnError> {
    let Some(user) = auth_session.current_user.as_ref().filter(|u| !u.username.is_empty()) else {
        return Ok(false);
    };
    let is_super_admin = user.permissions.contains("SuperAdmin");
    Ok(is_super_admin || user.permissions.contains("Admin"))
}

#[put("/api/v1/logout", auth_session: axum::Extension<AuthSession>)]
async fn logout() -> Result<(), ServerFnError> {
    auth_session.logout_user();
    Ok(())
}

/// Settings gear icon — only rendered for admin / super-admin users.
///
/// Uses the same `SuspenseBoundary` isolation pattern as `IncomingBadge` so
/// the icon is simply absent for non-admins without affecting NavBar layout.
#[component]
fn AdminSettingsButton() -> Element {
    let navigator = use_navigator();
    let is_admin = use_server_future(get_is_admin)?;
    let admin = is_admin().and_then(|r: Result<bool, ServerFnError>| r.ok()).unwrap_or(false);

    if !admin {
        return rsx! {};
    }

    rsx! {
        button {
            class: "flex items-center hover:text-indigo-200 cursor-pointer",
            title: "Settings",
            onclick: move |_| { navigator.push(Route::SettingsPage {}); },
            svg {
                class: "w-5 h-5",
                fill: "none",
                view_box: "0 0 24 24",
                stroke_width: "1.5",
                stroke: "currentColor",
                path {
                    stroke_linecap: "round",
                    stroke_linejoin: "round",
                    d: "M9.594 3.94c.09-.542.56-.94 1.11-.94h2.593c.55 0 1.02.398 1.11.94l.213 1.281c.063.374.313.686.645.87.074.04.147.083.22.127.325.196.72.257 1.075.124l1.217-.456a1.125 1.125 0 0 1 1.37.49l1.296 2.247a1.125 1.125 0 0 1-.26 1.431l-1.003.827c-.293.241-.438.613-.43.992a7.723 7.723 0 0 1 0 .255c-.008.378.137.75.43.991l1.004.827c.424.35.534.955.26 1.43l-1.298 2.247a1.125 1.125 0 0 1-1.369.491l-1.217-.456c-.355-.133-.75-.072-1.076.124a6.47 6.47 0 0 1-.22.128c-.331.183-.581.495-.644.869l-.213 1.281c-.09.543-.56.94-1.11.94h-2.594c-.55 0-1.019-.398-1.11-.94l-.213-1.281c-.062-.374-.312-.686-.644-.87a6.52 6.52 0 0 1-.22-.127c-.325-.196-.72-.257-1.076-.124l-1.217.456a1.125 1.125 0 0 1-1.369-.49l-1.297-2.247a1.125 1.125 0 0 1 .26-1.431l1.004-.827c.292-.24.437-.613.43-.991a6.932 6.932 0 0 1 0-.255c.007-.38-.138-.751-.43-.992l-1.004-.827a1.125 1.125 0 0 1-.26-1.43l1.297-2.247a1.125 1.125 0 0 1 1.37-.491l1.216.456c.356.133.751.072 1.076-.124.072-.044.146-.086.22-.128.332-.183.582-.495.644-.869l.214-1.28Z",
                }
                path {
                    stroke_linecap: "round",
                    stroke_linejoin: "round",
                    d: "M15 12a3 3 0 1 1-6 0 3 3 0 0 1 6 0Z",
                }
            }
        }
    }
}

// ── About modal
// ─────────────────────────────────────────────────────────────────────────────

/// Modal showing app version and library statistics.
///
/// Stats are fetched when the modal mounts and fill in asynchronously;
/// the modal itself appears immediately without waiting for the response.
#[component]
fn AboutModal(on_close: EventHandler<()>) -> Element {
    rsx! {
        div {
            class: "fixed inset-0 z-50 flex items-center justify-center bg-black/40",
            tabindex: -1,
            onmounted: move |e| async move { let _ = e.set_focus(true).await; },
            onclick: move |_| on_close(()),
            onkeydown: move |e| { if e.key() == Key::Escape { on_close(()); } },
            div {
                class: "bg-white dark:bg-slate-800 rounded-xl shadow-xl w-full max-w-5xl mx-4",
                onclick: |e| e.stop_propagation(),
                // Header
                div { class: "flex items-center justify-between px-6 pt-5 pb-2",
                    h2 { class: "text-lg font-semibold text-gray-900 dark:text-slate-100", "About" }
                    button {
                        class: "text-gray-400 dark:text-slate-500 hover:text-gray-600 dark:hover:text-slate-300 cursor-pointer",
                        onclick: move |_| on_close(()),
                        svg {
                            class: "w-5 h-5",
                            fill: "none",
                            view_box: "0 0 24 24",
                            stroke_width: "1.5",
                            stroke: "currentColor",
                            path {
                                stroke_linecap: "round",
                                stroke_linejoin: "round",
                                d: "M6 18 18 6M6 6l12 12",
                            }
                        }
                    }
                }
                // Body
                div { class: "px-6 pb-6",
                    img {
                        src: asset!("/assets/MailKeep-Banner.png"),
                        alt: "MailKeep",
                        class: "w-full mb-2",
                    }
                    p { class: "text-sm text-gray-500 dark:text-slate-400 mb-6 text-center",
                        { format!("Version: {}", clap::crate_version!()) }
                    }
                }
            }
        }
    }
}

#[component]
fn ThemeToggle() -> Element {
    rsx! {
        button {
            class: "flex items-center hover:text-indigo-200 cursor-pointer text-sm",
            title: "Change theme",
            onclick: move |_| {
                let next = THEME_MODE.peek().cycle();
                *THEME_MODE.write() = next;
                // localStorage write must be inside spawn — document::eval
                // doesn't execute from a synchronous event-handler body.
                spawn(async move {
                    document::eval(&format!(
                        "localStorage.setItem('mk_theme',{:?})",
                        next.as_str()
                    ));
                    let _ = set_theme_preference(next).await;
                });
            },
            { THEME_MODE.read().icon() }
        }
    }
}

// ── SearchBar
// ─────────────────────────────────────────────────────────────────────────────

/// The centred search input, isolated into its own component so subscriptions
/// to `use_route`/`SEARCH_QUERY` don't repaint the whole `NavBar` on every
/// navigation. Ported from BookBoss; MailKeep submits on Enter (server FTS)
/// rather than filtering live.
#[component]
fn SearchBar() -> Element {
    let route = use_route::<Route>();

    let mut focused = use_signal(|| false);
    let mut help_open = use_signal(|| false);
    let mut hint_seen = use_signal(|| false);
    let mut tip_index = use_signal(|| 0usize);
    let mut completion = use_signal(String::new); // ghost-text suffix (e.g. "der:")
    let mut cycle_prefix = use_signal(String::new); // token being cycled (e.g. "a")
    let mut cycle_idx = use_signal(|| 0usize);

    use_hook(move || {
        spawn(async move {
            if let Ok(val) = document::eval("return window.localStorage.getItem('mk_search_hint_seen')").await
                && !val.is_null()
            {
                hint_seen.set(true);
            }
        });
    });

    use_hook(move || {
        spawn(async move {
            loop {
                let mut timer = document::eval("setTimeout(() => dioxus.send(true), 3000)");
                let _ = timer.recv::<bool>().await;
                if *focused.peek() && SEARCH_QUERY.peek().is_empty() {
                    tip_index.with_mut(|i| *i = (*i + 1) % PLACEHOLDER_TIPS.len());
                }
            }
        });
    });

    let show_hint = use_memo(move || {
        let empty = SEARCH_QUERY().is_empty();
        (focused() && empty && !hint_seen()) || (help_open() && empty)
    });

    // The search bar (and its results panel) live only on HomePage.
    let search_active = matches!(route, Route::HomePage);

    let search_placeholder: &str = if focused() && SEARCH_QUERY().is_empty() {
        PLACEHOLDER_TIPS[tip_index() % PLACEHOLDER_TIPS.len()]
    } else {
        "Search mail…"
    };

    rsx! {
        div { class: "w-full px-4 pt-2 pb-3 sm:pt-0 sm:pb-0 sm:absolute sm:left-1/2 sm:-translate-x-1/2 sm:max-w-md",
            if search_active {
                div { class: "flex items-center gap-2",
                    div { class: "relative flex-1",
                        div { class: "relative w-full bg-white/90 dark:bg-slate-700/90 focus-within:bg-white dark:focus-within:bg-slate-700 rounded focus-within:ring-2 focus-within:ring-indigo-300",
                            // Search icon
                            svg {
                                class: "absolute left-2.5 top-1/2 -translate-y-1/2 w-4 h-4 text-gray-400 pointer-events-none",
                                xmlns: "http://www.w3.org/2000/svg",
                                fill: "none",
                                view_box: "0 0 24 24",
                                stroke_width: "2",
                                stroke: "currentColor",
                                path {
                                    stroke_linecap: "round",
                                    stroke_linejoin: "round",
                                    d: "m21 21-5.197-5.197m0 0A7.5 7.5 0 1 0 5.196 5.196a7.5 7.5 0 0 0 10.607 10.607Z",
                                }
                            }
                            // Ghost-text overlay: transparent typed text + gray completion suffix
                            if !completion().is_empty() {
                                span {
                                    class: "absolute inset-0 flex items-center pl-9 pr-8 text-sm pointer-events-none overflow-hidden select-none",
                                    "aria-hidden": "true",
                                    span { style: "color: transparent; white-space: pre;", "{SEARCH_QUERY()}" }
                                    span { class: "text-gray-400 dark:text-slate-500", style: "white-space: pre;", "{completion()}" }
                                }
                            }
                            input {
                                class: "relative w-full pl-9 pr-8 py-1.5 text-sm text-gray-900 dark:text-slate-100 bg-transparent placeholder-gray-400 dark:placeholder-slate-500 outline-none",
                                r#type: "text",
                                placeholder: "{search_placeholder}",
                                value: SEARCH_QUERY(),
                                onfocus: move |_| focused.set(true),
                                onblur: move |_| {
                                    focused.set(false);
                                    help_open.set(false);
                                    if !hint_seen() {
                                        hint_seen.set(true);
                                        spawn(async move {
                                            let _ = document::eval("window.localStorage.setItem('mk_search_hint_seen','1')").await;
                                        });
                                    }
                                },
                                oninput: move |e| {
                                    let val = e.value();
                                    let new_completion = compute_completion(&val, 0);
                                    *SEARCH_QUERY.write() = val;
                                    help_open.set(false);
                                    if !hint_seen() {
                                        hint_seen.set(true);
                                        spawn(async move {
                                            let _ = document::eval("window.localStorage.setItem('mk_search_hint_seen','1')").await;
                                        });
                                    }
                                    cycle_prefix.set(String::new());
                                    cycle_idx.set(0);
                                    completion.set(new_completion);
                                },
                                onkeydown: move |e: KeyboardEvent| {
                                    match e.key() {
                                        Key::Enter => {
                                            // Submit: one Tantivy query per Enter. Whitespace-only clears.
                                            let trimmed = SEARCH_QUERY().trim().to_string();
                                            *ACTIVE_SEARCH.write() = if trimmed.is_empty() { None } else { Some(trimmed) };
                                            *crate::components::OPEN_MESSAGE.write() = None;
                                            completion.set(String::new());
                                            cycle_prefix.set(String::new());
                                            cycle_idx.set(0);
                                        }
                                        Key::Tab => {
                                            let current_completion = completion();
                                            if !cycle_prefix().is_empty() && !current_completion.is_empty() {
                                                e.prevent_default();
                                                let applied = apply_completion(&SEARCH_QUERY(), &current_completion);
                                                (*SEARCH_QUERY.write()).clone_from(&applied);
                                                cycle_idx.set(cycle_idx() + 1);
                                                completion.set(String::new());
                                            } else if !current_completion.is_empty() {
                                                e.prevent_default();
                                                let current_input = SEARCH_QUERY();
                                                let prefix = current_input.split_whitespace().last().unwrap_or("").to_string();
                                                let applied = apply_completion(&current_input, &current_completion);
                                                (*SEARCH_QUERY.write()).clone_from(&applied);
                                                cycle_prefix.set(prefix);
                                                cycle_idx.set(cycle_idx() + 1);
                                                completion.set(String::new());
                                            } else if !cycle_prefix().is_empty() {
                                                e.prevent_default();
                                                let prefix = cycle_prefix();
                                                let current_idx = cycle_idx();
                                                cycle_idx.set(current_idx + 1);
                                                if let Some(cycled) = next_cycle_input(&SEARCH_QUERY(), &prefix, current_idx) {
                                                    *SEARCH_QUERY.write() = cycled;
                                                    completion.set(String::new());
                                                }
                                            }
                                        }
                                        Key::Escape => {
                                            if !completion().is_empty() || !cycle_prefix().is_empty() {
                                                completion.set(String::new());
                                                cycle_prefix.set(String::new());
                                                cycle_idx.set(0);
                                            } else {
                                                *SEARCH_QUERY.write() = String::new();
                                                *ACTIVE_SEARCH.write() = None;
                                            }
                                        }
                                        _ => {
                                            if !completion().is_empty() || !cycle_prefix().is_empty() {
                                                completion.set(String::new());
                                                cycle_prefix.set(String::new());
                                                cycle_idx.set(0);
                                            }
                                        }
                                    }
                                },
                            }
                            // Clear button
                            if !SEARCH_QUERY().is_empty() {
                                button {
                                    class: "absolute right-2 top-1/2 -translate-y-1/2 text-gray-400 dark:text-slate-500 hover:text-gray-600 dark:hover:text-slate-300 cursor-pointer",
                                    onclick: move |_| {
                                        *SEARCH_QUERY.write() = String::new();
                                        *ACTIVE_SEARCH.write() = None;
                                    },
                                    svg {
                                        class: "w-4 h-4",
                                        xmlns: "http://www.w3.org/2000/svg",
                                        fill: "none",
                                        view_box: "0 0 24 24",
                                        stroke_width: "2",
                                        stroke: "currentColor",
                                        path { stroke_linecap: "round", stroke_linejoin: "round", d: "M6 18 18 6M6 6l12 12" }
                                    }
                                }
                            }
                        }
                        // Hint strip
                        if show_hint() {
                            div {
                                class: "absolute top-full left-0 right-0 mt-1 bg-blue-50 dark:bg-slate-800 border border-blue-200 dark:border-slate-600 rounded-md px-3 py-2 text-xs text-blue-700 dark:text-blue-300 z-50 shadow-sm leading-relaxed",
                                span { class: "font-semibold", "field:value" }
                                " to narrow results — "
                                for field in ["from:", "to:", "subject:", "folder:", "account:", "has:", "attachments:", "before:", "after:", "date:"] {
                                    code { class: "inline-block bg-blue-100 dark:bg-slate-700 rounded px-1 mr-1 font-mono", "{field}" }
                                }
                                " · Negate with "
                                code { class: "bg-blue-100 dark:bg-slate-700 rounded px-1 font-mono", "!filters" }
                                " · Quote phrases: "
                                code { class: "bg-blue-100 dark:bg-slate-700 rounded px-1 font-mono", "subject:\"order confirmation\"" }
                                " · Press Enter to search"
                            }
                        }
                    }
                    // ? help toggle
                    button {
                        class: "shrink-0 text-xs px-2 py-0.5 rounded-full bg-indigo-500 hover:bg-indigo-400 text-white font-medium cursor-pointer leading-tight",
                        title: "Search help",
                        onclick: move |_| help_open.set(!help_open()),
                        "?"
                    }
                }
            }
        }
    }
}

// ── NavBar
// ─────────────────────────────────────────────────────────────────────────────

#[component]
pub(crate) fn NavBar() -> Element {
    let navigator = use_navigator();
    let mut user_menu_open = use_signal(|| false);
    let mut show_about = use_signal(|| false);
    let on_logout = move |_| {
        user_menu_open.set(false);
        spawn(async move {
            let _ = logout().await;
            navigator.push(Route::LandingPage { login_failed: None });
        });
    };

    rsx! {
        nav { class: "relative bg-indigo-700 text-white px-3 sm:px-6 py-3 flex flex-wrap items-center shadow-sm",
            div { class: "flex items-center gap-3 sm:gap-6 shrink-0",
                button {
                    class: "hidden sm:flex items-center cursor-pointer hover:opacity-80",
                    title: "About",
                    onclick: move |_| show_about.set(true),
                    img {
                        src: asset!("/assets/MailKeep-Title.png"),
                        alt: "MailKeep",
                        class: "h-8 w-auto",
                    }
                }
                button {
                    class: "flex items-center hover:text-indigo-200 cursor-pointer",
                    title: "Home",
                    onclick: move |_| {
                        *SELECTED_ACCOUNT.write() = None;
                        *ACTIVE_SEARCH.write() = None;
                        *SEARCH_QUERY.write() = String::new();
                        navigator.push(Route::HomePage {});
                    },
                    svg {
                        class: "w-5 h-5",
                        fill: "none",
                        view_box: "0 0 24 24",
                        stroke_width: "1.5",
                        stroke: "currentColor",
                        path {
                            stroke_linecap: "round",
                            stroke_linejoin: "round",
                            d: "m2.25 12 8.954-8.955c.44-.439 1.152-.439 1.591 0L21.75 12M4.5 9.75v10.125c0 .621.504 1.125 1.125 1.125H9.75v-4.875c0-.621.504-1.125 1.125-1.125h2.25c.621 0 1.125.504 1.125 1.125V21h4.125c.621 0 1.125-.504 1.125-1.125V9.75M8.25 21h8.25",
                        }
                    }
                }
            }
            SearchBar {}
            div { class: "flex items-center gap-4 shrink-0 ml-auto",
                ThemeToggle {}
                SuspenseBoundary {
                    fallback: |_| rsx! {},
                    AdminSettingsButton {}
                }
                div { class: "relative",
                    button {
                        class: "flex items-center hover:text-indigo-200",
                        title: "User",
                        onclick: move |_| user_menu_open.toggle(),
                        svg {
                            class: "w-5 h-5",
                            fill: "none",
                            view_box: "0 0 24 24",
                            stroke_width: "1.5",
                            stroke: "currentColor",
                            path {
                                stroke_linecap: "round",
                                stroke_linejoin: "round",
                                d: "M15.75 6a3.75 3.75 0 1 1-7.5 0 3.75 3.75 0 0 1 7.5 0ZM4.501 20.118a7.5 7.5 0 0 1 14.998 0A17.933 17.933 0 0 1 12 21.75c-2.676 0-5.216-.584-7.499-1.632Z",
                            }
                        }
                    }
                    if user_menu_open() {
                        div {
                            class: "fixed inset-0 z-40",
                            onclick: move |_| user_menu_open.set(false),
                        }
                        div { class: "absolute right-0 top-full mt-1 w-36 bg-white dark:bg-slate-800 rounded-lg shadow-lg py-1 z-50 border dark:border-slate-700",
                            button {
                                class: "w-full text-left px-4 py-2 text-sm text-gray-700 dark:text-slate-200 hover:bg-gray-100 dark:hover:bg-slate-700",
                                onclick: on_logout,
                                "Logout"
                            }
                        }
                    }
                }
            }
        }
        if show_about() {
            AboutModal { on_close: move |()| show_about.set(false) }
        }
    }
}
