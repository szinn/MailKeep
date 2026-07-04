use dioxus::prelude::*;
#[cfg(feature = "server")]
use {
    crate::components::message_to_row,
    crate::routes::server_helpers::{authenticated_user, to_server_err},
    crate::server::AuthSession,
    mk_core::{
        CoreServices,
        account::{AccountId, AccountToken},
    },
    std::sync::Arc,
};

use crate::components::MessageRowDto;

/// Default page size for the account message list.
pub(crate) const PAGE_SIZE: u32 = 50;

/// List one page of the selected account's messages (newest first), scoped to
/// the authenticated user. Mirrors `get_account_folders`: the ownership gate
/// (`get_account`) also 404s a foreign/unknown account before any data is read.
#[post(
    "/api/v1/home/messages",
    auth_session: axum::Extension<AuthSession>,
    core_services: axum::Extension<Arc<CoreServices>>
)]
pub(crate) async fn list_messages(account_token: String, limit: u32, offset: u32) -> Result<Vec<MessageRowDto>, ServerFnError> {
    let user = authenticated_user(&auth_session)?;
    let account_id: AccountId = account_token.parse::<AccountToken>().map_err(to_server_err)?.id();
    // Ownership gate (also 404s a foreign/unknown account).
    core_services.account_service.get_account(user.id(), account_id).await.map_err(to_server_err)?;
    let messages = core_services
        .message_service
        .list_messages_for_account(account_id, limit, offset)
        .await
        .map_err(to_server_err)?;
    Ok(messages.iter().map(message_to_row).collect())
}
