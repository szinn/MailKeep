use dioxus::prelude::*;
#[cfg(feature = "server")]
use {
    crate::routes::server_helpers::{authenticated_user, to_server_err},
    crate::server::AuthSession,
    mk_core::{CoreServices, types::EmailAddress, user::User},
    std::sync::Arc,
};

use crate::Route;

// ---------------------------------------------------------------------------
// DTOs
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
struct ProfileInfo {
    full_name: String,
    email: String,
}

// ---------------------------------------------------------------------------
// Server functions
// ---------------------------------------------------------------------------

#[get(
    "/api/v1/profile/context",
    auth_session: axum::Extension<AuthSession>
)]
async fn get_profile_context() -> Result<(), ServerFnError> {
    authenticated_user(&auth_session)?;
    Ok(())
}

#[get(
    "/api/v1/profile/info",
    auth_session: axum::Extension<AuthSession>,
    core_services: axum::Extension<Arc<CoreServices>>
)]
async fn get_profile_info() -> Result<ProfileInfo, ServerFnError> {
    let user_id = authenticated_user(&auth_session)?.id();

    let user = core_services
        .user_service
        .find_by_id(user_id)
        .await
        .map_err(to_server_err)?
        .ok_or_else(|| ServerFnError::new("User not found"))?;

    Ok(ProfileInfo {
        full_name: user.full_name.clone(),
        email: user.email_address.to_string(),
    })
}

#[post(
    "/api/v1/profile/update",
    auth_session: axum::Extension<AuthSession>,
    core_services: axum::Extension<Arc<CoreServices>>
)]
async fn update_profile(full_name: String, email: String) -> Result<(), ServerFnError> {
    let full_name = full_name.trim().to_string();
    if full_name.is_empty() {
        return Err(ServerFnError::new("Full name must not be empty"));
    }
    let email_address = EmailAddress::new(&email).map_err(to_server_err)?;

    let user_id = authenticated_user(&auth_session)?.id();

    let existing = core_services
        .user_service
        .find_by_id(user_id)
        .await
        .map_err(to_server_err)?
        .ok_or_else(|| ServerFnError::new("User not found"))?;

    core_services
        .user_service
        .update_user(User {
            full_name,
            email_address,
            ..existing
        })
        .await
        .map_err(to_server_err)?;

    Ok(())
}

#[post(
    "/api/v1/profile/change-password",
    auth_session: axum::Extension<AuthSession>,
    core_services: axum::Extension<Arc<CoreServices>>
)]
async fn change_password(current: String, new_password: String) -> Result<(), ServerFnError> {
    if new_password.trim().is_empty() {
        return Err(ServerFnError::new("New password must not be empty"));
    }

    let user_id = authenticated_user(&auth_session)?.id();

    let existing = core_services
        .user_service
        .find_by_id(user_id)
        .await
        .map_err(to_server_err)?
        .ok_or_else(|| ServerFnError::new("User not found"))?;

    if !existing.check_password(&current) {
        return Err(ServerFnError::new("Current password is incorrect"));
    }

    let new_hash = User::encrypt_password(&new_password).map_err(to_server_err)?;

    core_services
        .user_service
        .update_user(User {
            password_hash: new_hash,
            ..existing
        })
        .await
        .map_err(to_server_err)?;

    Ok(())
}

#[component]
pub(crate) fn ProfilePage() -> Element {
    let navigator = use_navigator();
    let auth = use_server_future(get_profile_context)?;

    use_effect(move || {
        if let Some(Err(_)) = auth() {
            navigator.replace(Route::LandingPage { login_failed: None });
        }
    });

    rsx! {
        div { class: "flex-1 overflow-auto p-8",
            div { class: "max-w-lg mx-auto flex flex-col gap-10",

                // ── Profile ──────────────────────────────────────────────
                section {
                    h2 { class: "text-lg font-semibold text-gray-900 dark:text-slate-100 mb-4", "Profile" }
                    ProfileSectionContent {}
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// ---------------------------------------------------------------------------
// Profile section
// ---------------------------------------------------------------------------

#[component]
fn ProfileSectionContent() -> Element {
    let info = use_server_future(get_profile_info)?;

    // Profile info signals
    let mut full_name = use_signal(String::new);
    let mut email = use_signal(String::new);
    let mut profile_saving = use_signal(|| false);
    let mut profile_saved = use_signal(|| false);
    let mut profile_error: Signal<Option<String>> = use_signal(|| None);

    // Change-password modal signals
    let mut pw_modal_open = use_signal(|| false);
    let mut current_pw = use_signal(String::new);
    let mut new_pw = use_signal(String::new);
    let mut confirm_pw = use_signal(String::new);
    let mut pw_saving = use_signal(|| false);
    let mut pw_error: Signal<Option<String>> = use_signal(|| None);

    use_effect(move || {
        if let Some(Ok(i)) = info() {
            full_name.set(i.full_name.clone());
            email.set(i.email.clone());
        }
    });

    let passwords_match = use_memo(move || confirm_pw().is_empty() || new_pw() == confirm_pw());

    let mut close_pw_modal = move || {
        pw_modal_open.set(false);
        current_pw.set(String::new());
        new_pw.set(String::new());
        confirm_pw.set(String::new());
        pw_error.set(None);
    };

    let input_class = "w-full rounded-md border border-gray-300 dark:border-slate-600 px-3 py-1.5 text-sm focus:outline-none focus:ring-1 \
                       focus:ring-indigo-500 bg-white dark:bg-slate-700 text-gray-900 dark:text-slate-100";
    let label_class = "block text-sm font-medium text-gray-700 dark:text-slate-300 mb-1";

    rsx! {
        // ── Profile info card ─────────────────────────────────────────────
        div { class: "rounded-lg border border-gray-200 dark:border-slate-700 bg-white dark:bg-slate-800 px-4 py-4 flex flex-col gap-3",
            div {
                label { class: label_class, "Full Name" }
                input {
                    r#type: "text",
                    class: input_class,
                    value: full_name,
                    oninput: move |e| {
                        profile_saved.set(false);
                        full_name.set(e.value());
                    },
                }
            }
            div {
                label { class: label_class, "Email" }
                input {
                    r#type: "email",
                    class: input_class,
                    value: email,
                    oninput: move |e| {
                        profile_saved.set(false);
                        email.set(e.value());
                    },
                }
            }
            if let Some(err) = profile_error() {
                p { class: "text-xs text-red-600", "{err}" }
            }
            div { class: "flex items-center justify-between",
                // Left — Change Password trigger
                button {
                    class: "px-3 py-1.5 text-sm font-medium rounded border border-gray-300 dark:border-slate-600 text-gray-700 dark:text-slate-300 hover:bg-gray-50 dark:hover:bg-slate-700",
                    onclick: move |_| pw_modal_open.set(true),
                    "Change Password"
                }
                // Right — Save
                div { class: "flex items-center gap-3",
                    if profile_saved() {
                        span { class: "text-xs text-green-600", "Saved!" }
                    }
                    button {
                        class: "px-3 py-1.5 text-sm font-medium rounded bg-indigo-600 text-white hover:bg-indigo-700 disabled:opacity-50",
                        disabled: profile_saving(),
                        onclick: move |_| {
                            let name = full_name();
                            let em = email();
                            profile_saving.set(true);
                            profile_saved.set(false);
                            profile_error.set(None);
                            spawn(async move {
                                match update_profile(name, em).await {
                                    Ok(()) => profile_saved.set(true),
                                    Err(e) => profile_error.set(Some(e.to_string())),
                                }
                                profile_saving.set(false);
                            });
                        },
                        if profile_saving() { "Saving…" } else { "Save" }
                    }
                }
            }
        }

        // ── Change Password modal ─────────────────────────────────────────
        if pw_modal_open() {
            div {
                class: "fixed inset-0 z-50 flex items-center justify-center bg-black/40",
                tabindex: -1,
                onmounted: move |e| async move { let _ = e.set_focus(true).await; },
                onkeydown: move |e| { if e.key() == Key::Escape { close_pw_modal(); } },
                div { class: "bg-white dark:bg-slate-800 rounded-2xl shadow-xl w-full max-w-sm p-6",
                    h3 { class: "text-base font-semibold text-gray-900 dark:text-slate-100 mb-4", "Change Password" }

                    div { class: "flex flex-col gap-3",
                        div {
                            label { class: label_class, "Current password" }
                            input {
                                r#type: "password",
                                class: input_class,
                                value: current_pw,
                                oninput: move |e| current_pw.set(e.value()),
                            }
                        }
                        div {
                            label { class: label_class, "New password" }
                            input {
                                r#type: "password",
                                class: input_class,
                                value: new_pw,
                                oninput: move |e| new_pw.set(e.value()),
                            }
                        }
                        div {
                            label { class: label_class, "Confirm new password" }
                            input {
                                r#type: "password",
                                class: input_class,
                                value: confirm_pw,
                                oninput: move |e| confirm_pw.set(e.value()),
                            }
                            if !passwords_match() {
                                p { class: "text-xs text-red-600 mt-1", "Passwords do not match" }
                            }
                        }
                        if let Some(err) = pw_error() {
                            p { class: "text-xs text-red-600", "{err}" }
                        }
                    }

                    div { class: "flex justify-end gap-3 mt-5",
                        button {
                            class: "px-3 py-1.5 text-sm font-medium rounded border border-gray-300 dark:border-slate-600 text-gray-700 dark:text-slate-300 hover:bg-gray-50 dark:hover:bg-slate-700",
                            disabled: pw_saving(),
                            onclick: move |_| close_pw_modal(),
                            "Cancel"
                        }
                        button {
                            class: "px-3 py-1.5 text-sm font-medium rounded bg-indigo-600 text-white hover:bg-indigo-700 disabled:opacity-50",
                            disabled: pw_saving() || !passwords_match() || new_pw().is_empty(),
                            onclick: move |_| {
                                let cur = current_pw();
                                let new = new_pw();
                                pw_saving.set(true);
                                pw_error.set(None);
                                spawn(async move {
                                    match change_password(cur, new).await {
                                        Ok(()) => close_pw_modal(),
                                        Err(e) => pw_error.set(Some(e.to_string())),
                                    }
                                    pw_saving.set(false);
                                });
                            },
                            if pw_saving() { "Saving…" } else { "Change Password" }
                        }
                    }
                }
            }
        }
    }
}
