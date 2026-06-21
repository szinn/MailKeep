use dioxus::prelude::*;

#[cfg(feature = "server")]
use crate::server::AuthSession;
use crate::{
    Route,
    components::{THEME_MODE, set_theme_preference},
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
                    onclick: move |_| { navigator.push(Route::HomePage {}); },
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
