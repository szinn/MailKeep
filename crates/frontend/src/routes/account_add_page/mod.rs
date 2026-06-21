use dioxus::prelude::*;

pub(crate) mod connection_form;
pub(crate) mod dtos;
pub(crate) mod folder_picker;
pub(crate) mod folder_tree;
use connection_form::ConnectionForm;
use dtos::{AccountSummaryDto, NewAccountDto, RemoteFolderDto, ServerConfigDto};
use folder_picker::FolderPicker;
use folder_tree::FolderTree;
#[cfg(feature = "server")]
use {
    crate::routes::server_helpers::{authenticated_user, to_server_err},
    crate::server::AuthSession,
    dtos::{account_to_summary, remote_folder_to_dto, server_config_from_dto, special_use_from_string},
    mk_core::{CoreServices, account::CreateAccountParams, folder::NewFolderRequest, types::EmailAddress},
    secrecy::SecretString,
    std::sync::Arc,
};

use crate::Route;

// ---------------------------------------------------------------------------
// Probe server functions
// ---------------------------------------------------------------------------

#[post(
    "/api/v1/accounts/probe/test-connection",
    auth_session: axum::Extension<AuthSession>,
    core_services: axum::Extension<Arc<CoreServices>>
)]
pub(crate) async fn probe_test_connection(server: ServerConfigDto, email: String, password: String) -> Result<(), ServerFnError> {
    authenticated_user(&auth_session)?;

    let cfg = server_config_from_dto(&server);
    let creds = mk_core::imap::ImapCredentials {
        username: email,
        password: SecretString::from(password),
    };

    core_services.imap_account_service.test_connection(cfg, creds).await.map_err(to_server_err)
}

#[post(
    "/api/v1/accounts/probe/list-folders",
    auth_session: axum::Extension<AuthSession>,
    core_services: axum::Extension<Arc<CoreServices>>
)]
pub(crate) async fn probe_list_folders(server: ServerConfigDto, email: String, password: String) -> Result<Vec<RemoteFolderDto>, ServerFnError> {
    authenticated_user(&auth_session)?;

    let cfg = server_config_from_dto(&server);
    let creds = mk_core::imap::ImapCredentials {
        username: email,
        password: SecretString::from(password),
    };

    let folders = core_services
        .imap_account_service
        .list_remote_folders(cfg, creds)
        .await
        .map_err(to_server_err)?;

    Ok(folders.into_iter().map(remote_folder_to_dto).collect())
}

// ---------------------------------------------------------------------------
// Create + list server functions
// ---------------------------------------------------------------------------

#[put(
    "/api/v1/accounts/create",
    auth_session: axum::Extension<AuthSession>,
    core_services: axum::Extension<Arc<CoreServices>>
)]
pub(crate) async fn create_account_and_start(payload: NewAccountDto) -> Result<AccountSummaryDto, ServerFnError> {
    let user = authenticated_user(&auth_session)?;

    let email = EmailAddress::new(payload.email.clone()).map_err(to_server_err)?;
    let server = server_config_from_dto(&payload.server);

    let account = core_services
        .account_service
        .create_account(CreateAccountParams {
            user_id: user.id(),
            display_name: payload.display_name.clone(),
            email_address: email,
            server,
            username: payload.email.clone(),
            password: SecretString::from(payload.password),
        })
        .await
        .map_err(to_server_err)?;

    // Authoritative \Noselect safety filter (spec §6): only selectable folders
    // become sync requests.
    let requests: Vec<NewFolderRequest> = payload
        .folders
        .into_iter()
        .filter(|f| !f.no_select)
        .map(|f| NewFolderRequest {
            path: f.path,
            display_name: None,
            special_use: special_use_from_string(&f.special_use),
            uidvalidity: None,
        })
        .collect();

    core_services
        .folder_service
        .create_folders_for_account(account.id, requests)
        .await
        .map_err(to_server_err)?;

    core_services.imap_account_service.start_account(account.id).await.map_err(to_server_err)?;

    Ok(account_to_summary(&account))
}

#[get(
    "/api/v1/accounts/list",
    auth_session: axum::Extension<AuthSession>,
    core_services: axum::Extension<Arc<CoreServices>>
)]
pub(crate) async fn list_accounts() -> Result<Vec<AccountSummaryDto>, ServerFnError> {
    let user = authenticated_user(&auth_session)?;
    let mut accounts = core_services.account_service.list_accounts(user.id()).await.map_err(to_server_err)?;
    accounts.sort_by(|a, b| a.display_name.to_lowercase().cmp(&b.display_name.to_lowercase()));
    Ok(accounts.iter().map(account_to_summary).collect())
}

#[get(
    "/api/v1/accounts/add-context",
    auth_session: axum::Extension<AuthSession>,
)]
async fn get_account_add_context() -> Result<(), ServerFnError> {
    authenticated_user(&auth_session)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// AccountAddPage wizard
// ---------------------------------------------------------------------------

/// Maps a `ServerFnError` to a user-facing message. Mirrors the variant match
/// in `login_form.rs` — `ServerError` carries the friendly message produced by
/// `to_server_err`; anything else falls back to the `Display` impl.
fn friendly(e: &ServerFnError) -> String {
    match e {
        ServerFnError::ServerError { message, .. } => message.clone(),
        other => other.to_string(),
    }
}

fn login_friendly() -> String {
    "Login failed — check your email and password.".to_string()
}

#[component]
pub(crate) fn AccountAddPage() -> Element {
    let navigator = use_navigator();
    let auth = use_server_future(get_account_add_context)?;
    use_effect(move || {
        if let Some(Err(_)) = auth() {
            navigator.replace(Route::LandingPage { login_failed: None });
        }
    });

    let display_name = use_signal(String::new);
    let host = use_signal(String::new);
    let port = use_signal(|| "993".to_string());
    let tls = use_signal(|| "Tls".to_string());
    let email = use_signal(String::new);
    let password = use_signal(String::new);
    let port_touched = use_signal(|| false);

    let mut tree = use_signal(FolderTree::default);
    let mut picker_shown = use_signal(|| false);
    let mut picker_stale = use_signal(|| false);
    let mut connected_email: Signal<Option<String>> = use_signal(|| None);

    let mut probing = use_signal(|| false);
    let mut conn_error: Signal<Option<String>> = use_signal(|| None);
    let mut folder_error: Signal<Option<String>> = use_signal(|| None);
    let mut submit_error: Signal<Option<String>> = use_signal(|| None);
    let mut submitting = use_signal(|| false);

    // Failure state 3: mark the picker stale whenever a connection field changes
    // after a successful probe, so the user is prompted to re-test.
    use_effect(move || {
        let _ = (host(), port(), tls(), email(), password());
        if picker_shown() {
            picker_stale.set(true);
        }
    });

    let server_dto = move || ServerConfigDto {
        host: host().trim().to_string(),
        port: port().trim().parse::<u16>().unwrap_or(0),
        tls: tls(),
    };

    // Failure state 2: re-run only `probe_list_folders` (connection already good).
    let do_list_folders = move || {
        spawn(async move {
            folder_error.set(None);
            probing.set(true);
            match probe_list_folders(server_dto(), email().trim().to_string(), password()).await {
                Ok(folders) => {
                    let mut t = FolderTree::build(folders);
                    t.default_select_inbox();
                    tree.set(t);
                    picker_shown.set(true);
                    picker_stale.set(false);
                }
                Err(e) => folder_error.set(Some(friendly(&e))),
            }
            probing.set(false);
        });
    };

    let on_probe = move |_| {
        conn_error.set(None);
        folder_error.set(None);
        if display_name().trim().is_empty() {
            conn_error.set(Some("Display name is required.".into()));
            return;
        }
        if host().trim().is_empty() {
            conn_error.set(Some("IMAP server is required.".into()));
            return;
        }
        if server_dto().port == 0 {
            conn_error.set(Some("Enter a valid port.".into()));
            return;
        }
        if !email().contains('@') {
            conn_error.set(Some("Enter a valid email address.".into()));
            return;
        }
        if password().is_empty() {
            conn_error.set(Some("Password is required.".into()));
            return;
        }

        spawn(async move {
            probing.set(true);
            match probe_test_connection(server_dto(), email().trim().to_string(), password()).await {
                Ok(()) => {
                    connected_email.set(Some(email().trim().to_string()));
                    match probe_list_folders(server_dto(), email().trim().to_string(), password()).await {
                        // Failure state 2 handled in the inner Err arm: connection
                        // stayed good (banner kept), only folders failed.
                        Ok(folders) => {
                            let mut t = FolderTree::build(folders);
                            t.default_select_inbox();
                            tree.set(t);
                            picker_shown.set(true);
                            picker_stale.set(false);
                        }
                        Err(e) => {
                            picker_shown.set(false);
                            folder_error.set(Some(friendly(&e)));
                        }
                    }
                }
                // Failure state 1: connection failed — inline conn error, picker hidden.
                Err(_e) => {
                    picker_shown.set(false);
                    conn_error.set(Some(login_friendly()));
                }
            }
            probing.set(false);
        });
    };

    let on_submit = move |_| {
        submit_error.set(None);
        let payload = NewAccountDto {
            display_name: display_name().trim().to_string(),
            email: email().trim().to_string(),
            server: server_dto(),
            password: password(),
            folders: tree().selected_new_folders(),
        };
        if payload.folders.is_empty() {
            submit_error.set(Some("Select at least one folder.".into()));
            return;
        }
        spawn(async move {
            submitting.set(true);
            // Failure state 4: on Err keep the form (submitting back to false);
            // on Ok navigate home.
            match create_account_and_start(payload).await {
                Ok(_) => {
                    navigator.push(Route::HomePage {});
                }
                Err(e) => {
                    submit_error.set(Some(friendly(&e)));
                    submitting.set(false);
                }
            }
        });
    };

    rsx! {
        div { class: "flex-1 overflow-auto p-8",
            div { class: "max-w-2xl mx-auto",
                h1 { class: "text-2xl font-semibold text-gray-900 dark:text-slate-100 mb-6", "Add account" }

                if let Some(em) = connected_email() {
                    div { class: "mb-4 text-sm text-green-700 dark:text-green-400", "✓ Connected as {em}" }
                }
                if let Some(msg) = conn_error() {
                    div { class: "mb-4 p-3 bg-red-50 dark:bg-red-950 border border-red-200 dark:border-red-800 text-red-700 dark:text-red-300 rounded-lg text-sm",
                        "{msg}"
                    }
                }

                ConnectionForm {
                    display_name,
                    host,
                    port,
                    tls,
                    email,
                    password,
                    port_touched,
                    probing: probing(),
                    on_probe,
                }

                if let Some(msg) = folder_error() {
                    div { class: "mt-4 p-3 bg-red-50 dark:bg-red-950 border border-red-200 dark:border-red-800 text-red-700 dark:text-red-300 rounded-lg text-sm",
                        "{msg}"
                        button {
                            class: "ml-2 underline",
                            r#type: "button",
                            disabled: probing(),
                            onclick: move |_| do_list_folders(),
                            "Retry loading folders"
                        }
                    }
                }

                if picker_shown() {
                    if picker_stale() {
                        div { class: "mt-4 text-xs text-amber-600 dark:text-amber-400",
                            "Connection details changed — re-test to refresh the folder list."
                        }
                    }
                    FolderPicker {
                        tree,
                        dimmed: picker_stale(),
                        on_toggle: move |idx: usize| {
                            let mut t = tree();
                            let cur = t.nodes[idx].selected;
                            t.set_subtree(idx, !cur);
                            tree.set(t);
                        },
                        on_select_all: move |v: bool| {
                            let mut t = tree();
                            t.select_all(v);
                            tree.set(t);
                        },
                    }

                    if let Some(msg) = submit_error() {
                        div { class: "mt-4 p-3 bg-red-50 dark:bg-red-950 border border-red-200 dark:border-red-800 text-red-700 dark:text-red-300 rounded-lg text-sm",
                            "{msg}"
                        }
                    }

                    button {
                        class: "mt-6 w-full py-2 px-4 bg-indigo-600 hover:bg-indigo-700 disabled:bg-indigo-400 text-white font-semibold rounded-lg",
                        r#type: "button",
                        disabled: submitting() || picker_stale() || tree().selected_count() == 0,
                        onclick: on_submit,
                        if submitting() {
                            "Adding…"
                        } else {
                            "Add account · sync {tree().selected_count()} folders"
                        }
                    }
                }
            }
        }
    }
}
