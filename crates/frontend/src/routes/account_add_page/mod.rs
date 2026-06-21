use dioxus::prelude::*;

pub(crate) mod connection_form;
pub(crate) mod dtos;
pub(crate) mod folder_tree;
use dtos::{AccountSummaryDto, NewAccountDto, RemoteFolderDto, ServerConfigDto};
#[cfg(feature = "server")]
use {
    crate::routes::server_helpers::{authenticated_user, to_server_err},
    crate::server::AuthSession,
    dtos::{account_to_summary, remote_folder_to_dto, server_config_from_dto, special_use_from_string},
    mk_core::{CoreServices, account::CreateAccountParams, folder::NewFolderRequest, types::EmailAddress},
    secrecy::SecretString,
    std::sync::Arc,
};

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
