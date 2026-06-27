use dioxus::prelude::*;
#[cfg(feature = "server")]
use {
    crate::routes::server_helpers::{authenticated_user, to_server_err},
    crate::server::AuthSession,
    mk_core::{
        CoreServices,
        account::{AccountId, AccountToken},
    },
    std::sync::Arc,
};

#[post(
    "/api/v1/accounts/set-enabled",
    auth_session: axum::Extension<AuthSession>,
    core_services: axum::Extension<Arc<CoreServices>>
)]
pub(crate) async fn set_account_enabled(account_token: String, enabled: bool) -> Result<(), ServerFnError> {
    let user = authenticated_user(&auth_session)?;
    let account_id: AccountId = account_token.parse::<AccountToken>().map_err(to_server_err)?.id();
    if enabled {
        core_services.account_service.enable(user.id(), account_id).await.map_err(to_server_err)?;
        core_services.imap_account_service.start_account(account_id).await.map_err(to_server_err)?;
    } else {
        core_services.account_service.disable(user.id(), account_id).await.map_err(to_server_err)?;
        core_services.imap_account_service.stop_account(account_id).await.map_err(to_server_err)?;
    }
    Ok(())
}

#[post(
    "/api/v1/accounts/delete",
    auth_session: axum::Extension<AuthSession>,
    core_services: axum::Extension<Arc<CoreServices>>
)]
pub(crate) async fn delete_account(account_token: String) -> Result<(), ServerFnError> {
    let user = authenticated_user(&auth_session)?;
    let account_id: AccountId = account_token.parse::<AccountToken>().map_err(to_server_err)?.id();
    // Ownership gate (also 404s a foreign/unknown account).
    core_services.account_service.get_account(user.id(), account_id).await.map_err(to_server_err)?;
    // Stop the sync task before deleting; best-effort (already-stopped is fine).
    let _ = core_services.imap_account_service.stop_account(account_id).await;
    core_services
        .account_service
        .delete_account(user.id(), account_id)
        .await
        .map_err(to_server_err)?;
    Ok(())
}
