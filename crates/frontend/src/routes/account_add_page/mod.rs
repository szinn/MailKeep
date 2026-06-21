use dioxus::prelude::*;

pub(crate) mod dtos;
use dtos::{RemoteFolderDto, ServerConfigDto};
#[cfg(feature = "server")]
use {
    crate::routes::server_helpers::{authenticated_user, to_server_err},
    crate::server::AuthSession,
    dtos::{remote_folder_to_dto, server_config_from_dto},
    mk_core::CoreServices,
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
