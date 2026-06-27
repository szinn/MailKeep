use dioxus::prelude::*;
#[cfg(feature = "server")]
use {
    crate::routes::account_add_page::dtos::folder_to_account_folder,
    crate::routes::home_page::format::special_use_rank,
    crate::routes::server_helpers::{authenticated_user, to_server_err},
    crate::server::AuthSession,
    mk_core::{
        CoreServices,
        account::{AccountId, AccountToken},
        folder::FolderToken,
    },
    std::collections::HashMap,
    std::sync::Arc,
};

use crate::routes::account_add_page::dtos::{AccountFolderDto, FolderEnabledDto};

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

#[post(
    "/api/v1/accounts/folders",
    auth_session: axum::Extension<AuthSession>,
    core_services: axum::Extension<Arc<CoreServices>>
)]
pub(crate) async fn get_account_folders(account_token: String) -> Result<Vec<AccountFolderDto>, ServerFnError> {
    let user = authenticated_user(&auth_session)?;
    let account_id: AccountId = account_token.parse::<AccountToken>().map_err(to_server_err)?.id();
    core_services.account_service.get_account(user.id(), account_id).await.map_err(to_server_err)?; // ownership gate
    let folders = core_services.folder_service.list_folders(account_id).await.map_err(to_server_err)?;
    let mut out: Vec<AccountFolderDto> = folders.iter().map(folder_to_account_folder).collect();
    out.sort_by(|a, b| {
        special_use_rank(a.special_use.as_deref())
            .cmp(&special_use_rank(b.special_use.as_deref()))
            .then_with(|| a.path.cmp(&b.path))
    });
    Ok(out)
}

#[post(
    "/api/v1/accounts/folders/set-enabled",
    auth_session: axum::Extension<AuthSession>,
    core_services: axum::Extension<Arc<CoreServices>>
)]
pub(crate) async fn set_account_folders_enabled(account_token: String, folders: Vec<FolderEnabledDto>) -> Result<(), ServerFnError> {
    let user = authenticated_user(&auth_session)?;
    let account_id: AccountId = account_token.parse::<AccountToken>().map_err(to_server_err)?.id();
    let account = core_services.account_service.get_account(user.id(), account_id).await.map_err(to_server_err)?;

    if !folders.iter().any(|f| f.enabled) {
        return Err(ServerFnError::new("At least one folder must be enabled."));
    }

    // Only touch folders that actually belong to this account, and only when
    // changed.
    let current = core_services.folder_service.list_folders(account_id).await.map_err(to_server_err)?;
    let by_id: HashMap<u64, bool> = current.iter().map(|f| (f.id, f.enabled)).collect();
    let mut changed = false;
    for fe in &folders {
        let fid = fe.token.parse::<FolderToken>().map_err(to_server_err)?.id();
        if let Some(&cur) = by_id.get(&fid)
            && cur != fe.enabled
        {
            core_services.folder_service.set_enabled(fid, fe.enabled).await.map_err(to_server_err)?;
            changed = true;
        }
    }

    if changed && account.enabled {
        core_services.imap_account_service.stop_account(account_id).await.map_err(to_server_err)?;
        core_services.imap_account_service.start_account(account_id).await.map_err(to_server_err)?;
    }
    Ok(())
}
