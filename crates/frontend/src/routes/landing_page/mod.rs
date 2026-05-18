use dioxus::prelude::*;
#[cfg(feature = "server")]
use {
    crate::OidcConfig,
    crate::routes::server_helpers::to_server_err,
    crate::server::AuthSession,
    mk_core::{
        CoreServices,
        types::{Capabilities, Capability},
        user::{NewUser, User, UserToken},
    },
    std::collections::HashSet,
    std::sync::Arc,
};

use crate::{
    Route,
    components::{LoginForm, RegisterAdminForm},
};

/// Returns the SSO sign-in button label, or `None` if SSO is not configured.
#[get(
    "/api/v1/sso/config",
    oidc_config: Option<axum::Extension<Arc<OidcConfig>>>,
)]
pub(crate) async fn get_sso_config() -> Result<Option<String>, ServerFnError> {
    let Some(axum::Extension(cfg)) = oidc_config else {
        return Ok(None);
    };
    if cfg.is_sso_available() {
        Ok(Some(cfg.button_label().to_owned()))
    } else {
        Ok(None)
    }
}

/// Returns `true` if at least one user exists, so the landing page can decide
/// whether to render the login form or the bootstrap-admin form.
#[get(
    "/api/v1/bootstrap/has_users",
    core_services: axum::Extension<Arc<CoreServices>>,
)]
async fn has_any_user() -> Result<bool, ServerFnError> {
    let users = core_services.user_service.list_users(None, Some(1)).await.map_err(to_server_err)?;
    Ok(!users.is_empty())
}

/// Authenticates a user by username/password.
///
/// Returns:
/// - `Ok(None)` — login succeeded, session is established.
/// - `Ok(Some(token))` — credentials are valid but the user must change their
///   password before the session is created. The caller redirects to a
///   password-change form using `token`.
/// - `Err` — invalid credentials or an internal error.
#[post(
    "/api/v1/auth/login",
    auth_session: axum::Extension<AuthSession>,
    core_services: axum::Extension<Arc<CoreServices>>,
)]
pub(crate) async fn perform_login(username: String, password: String) -> Result<Option<String>, ServerFnError> {
    let user = core_services
        .auth_service
        .is_valid_login(&username, &password)
        .await
        .map_err(to_server_err)?
        .ok_or_else(|| ServerFnError::new("Invalid username or password"))?;

    if user.change_password_on_login {
        return Ok(Some(UserToken::new(user.id).to_string()));
    }

    auth_session.login_user(user.id);
    Ok(None)
}

/// Creates the initial administrator. Fails if any user already exists, so this
/// can only run once during initial bootstrap.
#[post(
    "/api/v1/bootstrap/register_admin",
    auth_session: axum::Extension<AuthSession>,
    core_services: axum::Extension<Arc<CoreServices>>,
)]
pub(crate) async fn register_admin(username: String, full_name: String, password: String, email: String) -> Result<(), ServerFnError> {
    let existing = core_services.user_service.list_users(None, Some(1)).await.map_err(to_server_err)?;
    if !existing.is_empty() {
        return Err(ServerFnError::new("Admin already exists"));
    }

    let mut capabilities: Capabilities = HashSet::new();
    capabilities.insert(Capability::SuperAdmin);

    let new_user = NewUser::new(username, password, email, capabilities, full_name, false).map_err(to_server_err)?;
    let user = core_services.user_service.add_user(new_user).await.map_err(to_server_err)?;
    auth_session.login_user(user.id);
    Ok(())
}

/// Sets a new password for a user whose previous login required a password
/// change. The `token` identifies the user; no current-password check applies
/// because that credential was already validated by [`perform_login`].
#[post(
    "/api/v1/auth/change_initial_password",
    core_services: axum::Extension<Arc<CoreServices>>,
)]
async fn change_initial_password(token: String, new_password: String) -> Result<(), ServerFnError> {
    if new_password.trim().is_empty() {
        return Err(ServerFnError::new("Password must not be empty"));
    }
    let user_token: UserToken = token.parse().map_err(to_server_err)?;
    let user = core_services
        .user_service
        .find_by_token(user_token)
        .await
        .map_err(to_server_err)?
        .ok_or_else(|| ServerFnError::new("User not found"))?;

    let new_hash = User::encrypt_password(&new_password).map_err(to_server_err)?;
    core_services
        .user_service
        .update_user(User {
            password_hash: new_hash,
            change_password_on_login: false,
            ..user
        })
        .await
        .map_err(to_server_err)?;
    Ok(())
}

#[component]
pub(crate) fn LandingPage(login_failed: Option<u8>) -> Element {
    let navigator = use_navigator();
    let has_users = use_server_future(has_any_user)?;
    let mut must_change_token: Signal<Option<String>> = use_signal(|| None);

    let initial_error = login_failed.map(|_| "Sign-in failed. Please try again.".to_string());

    let show_bootstrap = matches!(has_users(), Some(Ok(false)));

    rsx! {
        div { class: "min-h-screen flex items-center justify-center bg-gray-100 dark:bg-slate-900 p-4",
            if let Some(token) = must_change_token() {
                ChangeInitialPasswordForm {
                    token,
                    on_done: move |()| {
                        must_change_token.set(None);
                        navigator.replace(Route::HomePage {});
                    },
                }
            } else if show_bootstrap {
                RegisterAdminForm {}
            } else {
                LoginForm {
                    initial_error,
                    on_must_change: move |token: String| must_change_token.set(Some(token)),
                }
            }
        }
    }
}

#[component]
fn ChangeInitialPasswordForm(token: String, on_done: EventHandler<()>) -> Element {
    let mut new_password = use_signal(String::new);
    let mut confirm = use_signal(String::new);
    let mut error_msg: Signal<Option<String>> = use_signal(|| None);
    let mut saving = use_signal(|| false);

    rsx! {
        div { class: "bg-white dark:bg-slate-800 rounded-2xl shadow-lg w-full max-w-md p-8",
            h2 { class: "text-xl font-semibold text-gray-900 dark:text-slate-100 mb-1 text-center",
                "Set a new password"
            }
            p { class: "text-sm text-gray-500 dark:text-slate-400 text-center mb-6",
                "Your account requires a new password before continuing."
            }
            if let Some(msg) = error_msg() {
                div { class: "mb-4 p-3 bg-red-50 dark:bg-red-950 border border-red-200 dark:border-red-800 text-red-700 dark:text-red-300 rounded-lg text-sm",
                    "{msg}"
                }
            }
            div { class: "mb-4",
                label { class: "block text-sm font-medium text-gray-700 dark:text-slate-300 mb-1", "New password" }
                input {
                    r#type: "password",
                    class: "w-full px-3 py-2 border border-gray-300 dark:border-slate-600 rounded-lg bg-white dark:bg-slate-700 text-gray-900 dark:text-slate-100",
                    value: new_password,
                    oninput: move |e| new_password.set(e.value()),
                    disabled: saving,
                }
            }
            div { class: "mb-4",
                label { class: "block text-sm font-medium text-gray-700 dark:text-slate-300 mb-1", "Confirm new password" }
                input {
                    r#type: "password",
                    class: "w-full px-3 py-2 border border-gray-300 dark:border-slate-600 rounded-lg bg-white dark:bg-slate-700 text-gray-900 dark:text-slate-100",
                    value: confirm,
                    oninput: move |e| confirm.set(e.value()),
                    disabled: saving,
                }
            }
            button {
                class: "w-full py-2 px-4 bg-indigo-600 hover:bg-indigo-700 disabled:bg-indigo-400 text-white font-semibold rounded-lg",
                disabled: saving,
                onclick: {
                    let token = token.clone();
                    move |_| {
                        let np = new_password();
                        let cp = confirm();
                        if np != cp {
                            error_msg.set(Some("Passwords do not match.".to_string()));
                            return;
                        }
                        error_msg.set(None);
                        saving.set(true);
                        let token = token.clone();
                        spawn(async move {
                            match change_initial_password(token, np).await {
                                Ok(()) => on_done.call(()),
                                Err(e) => {
                                    error_msg.set(Some(e.to_string()));
                                    saving.set(false);
                                }
                            }
                        });
                    }
                },
                if saving() { "Saving…" } else { "Set password" }
            }
        }
    }
}
