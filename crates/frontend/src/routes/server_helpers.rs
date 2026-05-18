//! Shared helpers for server functions.

use dioxus::prelude::ServerFnError;
#[cfg(feature = "server")]
use {
    crate::server::BackendSessionPool,
    axum::http::Method,
    axum_session_auth::{Auth, Rights},
    bb_core::{types::Capability, user::UserId},
};

use crate::server::{AuthSession, AuthUser};

/// Extracts the authenticated `AuthUser` from the session.
///
/// Returns `Err("Not authenticated")` when the session carries no user or the
/// user is the anonymous default (empty username).
pub(crate) fn authenticated_user(auth_session: &AuthSession) -> Result<AuthUser, ServerFnError> {
    auth_session
        .current_user
        .as_ref()
        .filter(|u| !u.username.is_empty())
        .cloned()
        .ok_or_else(|| ServerFnError::new("Not authenticated"))
}

/// Converts any `Display` error into a `ServerFnError`.
///
/// Replaces the `.map_err(|e| ServerFnError::new(e.to_string()))` boilerplate
/// with a point-free `.map_err(to_server_err)`.
pub(crate) fn to_server_err<E: std::fmt::Display>(e: E) -> ServerFnError {
    ServerFnError::new(e.to_string())
}

/// Checks that the session user holds `capability` for `method`.
///
/// Returns `Err(ServerFnError::new("Forbidden"))` on failure so callers can use
/// `require_capability(...).await?` directly in server functions.
///
/// **Do not use** for checks that return a non-error fallback on failure
/// (e.g. `get_pending_count` which returns `Ok(None)` to hide the pending count
/// from unprivileged users).
#[cfg(feature = "server")]
pub(crate) async fn require_capability(auth_session: &AuthSession, capability: Capability, method: Method) -> Result<(), ServerFnError> {
    let current_user = auth_session.current_user.clone().unwrap_or_default();
    if !Auth::<AuthUser, UserId, BackendSessionPool>::build([method.clone()], true)
        .requires(Rights::any([Rights::permission(capability.as_str())]))
        .validate(&current_user, &method, None)
        .await
    {
        return Err(ServerFnError::new("Forbidden"));
    }
    Ok(())
}

/// Checks that the session user holds **at least one** of `capabilities`.
///
/// Useful when multiple distinct roles can perform the same action (e.g. both
/// `EditBook` and `ApproveImports` users need to set library memberships).
///
/// Returns `Err(ServerFnError::new("Forbidden"))` if none of the capabilities
/// match.
#[cfg(feature = "server")]
pub(crate) async fn require_any_capability(auth_session: &AuthSession, capabilities: &[Capability], method: Method) -> Result<(), ServerFnError> {
    let current_user = auth_session.current_user.clone().unwrap_or_default();
    for capability in capabilities {
        if Auth::<AuthUser, UserId, BackendSessionPool>::build([method.clone()], true)
            .requires(Rights::any([Rights::permission(capability.as_str())]))
            .validate(&current_user, &method, None)
            .await
        {
            return Ok(());
        }
    }
    Err(ServerFnError::new("Forbidden"))
}
