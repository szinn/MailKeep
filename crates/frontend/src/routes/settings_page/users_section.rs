use dioxus::prelude::*;
#[cfg(feature = "server")]
use mk_core::{CoreServices, types::Capability, user::NewUser};
#[cfg(feature = "server")]
use {
    crate::routes::server_helpers::{authenticated_user, to_server_err},
    crate::server::AuthSession,
    std::sync::Arc,
};

// ---------------------------------------------------------------------------
// DTOs
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub(crate) struct UserAdminRow {
    pub token: String,
    pub username: String,
    pub full_name: String,
    pub email: String,
    pub capabilities: Vec<String>,
}

impl UserAdminRow {
    pub fn role_label(&self) -> &'static str {
        if self.capabilities.iter().any(|c| c == "SuperAdmin") {
            "Super Admin"
        } else if self.capabilities.iter().any(|c| c == "Admin") {
            "Admin"
        } else {
            "User"
        }
    }

    pub fn role_sort_key(&self) -> u8 {
        if self.capabilities.iter().any(|c| c == "SuperAdmin") {
            0
        } else if self.capabilities.iter().any(|c| c == "Admin") {
            1
        } else {
            2
        }
    }
}

// ---------------------------------------------------------------------------
// Server functions
// ---------------------------------------------------------------------------

#[post(
    "/api/v1/admin/users/list",
    auth_session: axum::Extension<AuthSession>,
    core_services: axum::Extension<Arc<CoreServices>>
)]
pub(crate) async fn list_users_admin() -> Result<Vec<UserAdminRow>, ServerFnError> {
    let user = authenticated_user(&auth_session)?;

    if !user.permissions.contains("SuperAdmin") && !user.permissions.contains("Admin") {
        return Err(ServerFnError::new("Insufficient permissions"));
    }

    let mut users = core_services.user_service.list_users(None, None).await.map_err(to_server_err)?;

    users.sort_by(|a, b| {
        let a_key = role_sort_key_caps(&a.capabilities);
        let b_key = role_sort_key_caps(&b.capabilities);
        a_key.cmp(&b_key).then(a.username.cmp(&b.username))
    });

    Ok(users
        .into_iter()
        .map(|u| {
            let caps: Vec<String> = u.capabilities.iter().map(|c| c.as_str().to_string()).collect();
            UserAdminRow {
                token: u.token.to_string(),
                username: u.username,
                full_name: u.full_name,
                email: u.email_address.as_str().to_string(),
                capabilities: caps,
            }
        })
        .collect())
}

#[cfg(feature = "server")]
fn role_sort_key_caps(caps: &mk_core::types::Capabilities) -> u8 {
    if caps.contains(&Capability::SuperAdmin) {
        0
    } else if caps.contains(&Capability::Admin) {
        1
    } else {
        2
    }
}

#[put(
    "/api/v1/admin/users/create",
    auth_session: axum::Extension<AuthSession>,
    core_services: axum::Extension<Arc<CoreServices>>
)]
pub(crate) async fn admin_create_user(
    username: String,
    full_name: String,
    email: String,
    password: String,
    capabilities: Vec<String>,
) -> Result<String, ServerFnError> {
    use std::collections::HashSet;

    let actor = authenticated_user(&auth_session)?;

    if !actor.permissions.contains("SuperAdmin") && !actor.permissions.contains("Admin") {
        return Err(ServerFnError::new("Insufficient permissions"));
    }

    let caps: HashSet<Capability> = parse_capabilities(&capabilities)?;

    if caps.contains(&Capability::SuperAdmin) {
        return Err(ServerFnError::new("Cannot assign Super Admin role"));
    }
    if caps.contains(&Capability::Admin) && !actor.permissions.contains("SuperAdmin") {
        return Err(ServerFnError::new("Only Super Admin can create Admin users"));
    }

    let new_user = NewUser::new(username, password, email, caps, full_name, true).map_err(|e| {
        let msg = e.to_string();
        if msg.contains("Constraint") || msg.contains("unique") || msg.contains("duplicate") {
            ServerFnError::new("Username or email address is already in use")
        } else {
            ServerFnError::new(msg)
        }
    })?;

    let user = core_services.user_service.add_user(new_user).await.map_err(|e| {
        let msg = e.to_string();
        if msg.contains("Constraint") || msg.contains("unique") || msg.contains("duplicate") {
            ServerFnError::new("Username or email address is already in use")
        } else {
            ServerFnError::new(msg)
        }
    })?;

    Ok(user.token.to_string())
}

#[post(
    "/api/v1/admin/users/update",
    auth_session: axum::Extension<AuthSession>,
    core_services: axum::Extension<Arc<CoreServices>>
)]
pub(crate) async fn admin_update_user(
    token: String,
    full_name: String,
    email: String,
    password: Option<String>,
    capabilities: Vec<String>,
) -> Result<(), ServerFnError> {
    use std::collections::HashSet;

    use mk_core::user::UserToken;

    let actor = authenticated_user(&auth_session)?;

    if !actor.permissions.contains("SuperAdmin") && !actor.permissions.contains("Admin") {
        return Err(ServerFnError::new("Insufficient permissions"));
    }

    let user_token: UserToken = token.parse().map_err(|_| ServerFnError::new("Invalid user token"))?;

    let mut user = core_services
        .user_service
        .find_by_token(user_token)
        .await
        .map_err(to_server_err)?
        .ok_or_else(|| ServerFnError::new("User not found"))?;

    if user.capabilities.contains(&Capability::SuperAdmin) && !actor.permissions.contains("SuperAdmin") {
        return Err(ServerFnError::new("Only Super Admin can edit a Super Admin user"));
    }

    let new_caps: HashSet<Capability> = parse_capabilities(&capabilities)?;

    if new_caps.contains(&Capability::SuperAdmin) && !user.capabilities.contains(&Capability::SuperAdmin) {
        return Err(ServerFnError::new("Cannot assign Super Admin role"));
    }
    if user.capabilities.contains(&Capability::SuperAdmin) && !new_caps.contains(&Capability::SuperAdmin) {
        return Err(ServerFnError::new("Cannot remove Super Admin role"));
    }
    if new_caps.contains(&Capability::Admin) && !actor.permissions.contains("SuperAdmin") {
        return Err(ServerFnError::new("Only Super Admin can assign Admin role"));
    }

    let full_name = full_name.trim().to_string();
    if full_name.is_empty() {
        return Err(ServerFnError::new("Full name is required"));
    }

    user.full_name = full_name;
    user.email_address = mk_core::types::EmailAddress::new(email).map_err(to_server_err)?;
    user.capabilities = new_caps;

    if let Some(pw) = password.filter(|p| !p.is_empty()) {
        user.password_hash = mk_core::user::User::encrypt_password(pw).map_err(to_server_err)?;
        let is_self = mk_core::user::UserToken::new(actor.id()).to_string() == token;
        if !is_self {
            user.change_password_on_login = true;
        }
    }

    core_services.user_service.update_user(user).await.map_err(to_server_err)?;

    Ok(())
}

#[post(
    "/api/v1/admin/users/delete",
    auth_session: axum::Extension<AuthSession>,
    core_services: axum::Extension<Arc<CoreServices>>
)]
pub(crate) async fn admin_delete_user(token: String) -> Result<(), ServerFnError> {
    use mk_core::user::UserToken;

    let actor = authenticated_user(&auth_session)?;

    if !actor.permissions.contains("SuperAdmin") && !actor.permissions.contains("Admin") {
        return Err(ServerFnError::new("Insufficient permissions"));
    }

    if mk_core::user::UserToken::new(actor.id()).to_string() == token {
        return Err(ServerFnError::new("You cannot delete your own account"));
    }

    let user_token: UserToken = token.parse().map_err(|_| ServerFnError::new("Invalid user token"))?;

    let user = core_services
        .user_service
        .find_by_token(user_token)
        .await
        .map_err(to_server_err)?
        .ok_or_else(|| ServerFnError::new("User not found"))?;

    if user.capabilities.contains(&Capability::SuperAdmin) {
        return Err(ServerFnError::new("Cannot delete the Super Admin user"));
    }

    core_services.user_service.delete_user(user.id).await.map_err(to_server_err)?;

    Ok(())
}

#[post(
    "/api/v1/admin/generate-password",
    auth_session: axum::Extension<AuthSession>
)]
pub(crate) async fn generate_password() -> Result<String, ServerFnError> {
    authenticated_user(&auth_session)?;

    Ok(make_password())
}

#[cfg(feature = "server")]
fn make_password() -> String {
    use rand::RngExt;

    const UPPER: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ";
    const LOWER: &[u8] = b"abcdefghijklmnopqrstuvwxyz";
    const DIGITS: &[u8] = b"0123456789";
    const SPECIAL: &[u8] = b"!@#$%^&*()_+-=[]{}|;:,.<>?";
    const ALL: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789!@#$%^&*()_+-=[]{}|;:,.<>?";

    let mut rng = rand::rng();
    // Guarantee one of each required character class
    let mut pw: Vec<u8> = vec![
        UPPER[rng.random_range(0..UPPER.len())],
        LOWER[rng.random_range(0..LOWER.len())],
        DIGITS[rng.random_range(0..DIGITS.len())],
        SPECIAL[rng.random_range(0..SPECIAL.len())],
    ];
    for _ in 4..16 {
        pw.push(ALL[rng.random_range(0..ALL.len())]);
    }
    // Fisher-Yates shuffle
    for i in (1..pw.len()).rev() {
        let j = rng.random_range(0..=i);
        pw.swap(i, j);
    }
    String::from_utf8(pw).expect("all bytes are valid ASCII")
}

#[cfg(feature = "server")]
fn parse_capabilities(capabilities: &[String]) -> Result<mk_core::types::Capabilities, ServerFnError> {
    use std::collections::HashSet;
    let mut caps = HashSet::new();
    for s in capabilities {
        let cap = match s.as_str() {
            "Admin" => Capability::Admin,
            "SuperAdmin" => Capability::SuperAdmin,
            other => return Err(ServerFnError::new(format!("Unknown capability: {other}"))),
        };
        caps.insert(cap);
    }
    Ok(caps)
}

// ---------------------------------------------------------------------------
// UsersSection component
// ---------------------------------------------------------------------------

#[component]
pub(crate) fn UsersSection(is_super_admin: bool, current_user_token: String) -> Element {
    // Increment to trigger a reload of the user list.
    let mut refresh = use_signal(|| 0u32);

    let users_resource = use_resource(move || async move {
        let _ = refresh(); // subscribe
        list_users_admin().await
    });

    let mut modal_target: Signal<Option<Option<UserAdminRow>>> = use_signal(|| None);
    let mut delete_target: Signal<Option<UserAdminRow>> = use_signal(|| None);
    let mut delete_error: Signal<Option<String>> = use_signal(|| None);
    let mut deleting = use_signal(|| false);

    rsx! {
        div { class: "w-full max-w-3xl",
            // Header
            div { class: "flex items-center justify-between mb-6",
                h2 { class: "text-lg font-semibold text-gray-900 dark:text-slate-100", "Users" }
                button {
                    class: "px-3 py-1.5 text-sm font-medium rounded bg-indigo-600 text-white hover:bg-indigo-700",
                    onclick: move |_| modal_target.set(Some(None)),
                    "+ Add User"
                }
            }

            // Error from delete
            if let Some(msg) = delete_error() {
                div { class: "mb-4 p-3 bg-red-50 border border-red-200 text-red-700 rounded-lg text-sm dark:bg-red-900/30 dark:border-red-800 dark:text-red-400",
                    "{msg}"
                }
            }

            // User table
            match users_resource() {
                None => rsx! {
                    div { class: "text-gray-400 text-sm dark:text-slate-500", "Loading…" }
                },
                Some(Err(e)) => rsx! {
                    div { class: "p-3 bg-red-50 border border-red-200 text-red-700 rounded-lg text-sm dark:bg-red-900/30 dark:border-red-800 dark:text-red-400",
                        "{e}"
                    }
                },
                Some(Ok(rows)) => rsx! {
                    div { class: "rounded-lg border border-gray-200 bg-white overflow-hidden dark:border-slate-700 dark:bg-slate-800",
                        table { class: "w-full text-sm",
                            thead {
                                tr { class: "bg-gray-50 border-b border-gray-200 dark:bg-slate-800 dark:border-slate-700",
                                    th { class: "px-4 py-3 text-left text-xs font-medium text-gray-500 uppercase tracking-wide dark:text-slate-400", "Username" }
                                    th { class: "px-4 py-3 text-left text-xs font-medium text-gray-500 uppercase tracking-wide dark:text-slate-400", "Full Name" }
                                    th { class: "px-4 py-3 text-left text-xs font-medium text-gray-500 uppercase tracking-wide dark:text-slate-400", "Role" }
                                    th { class: "px-4 py-3 text-right text-xs font-medium text-gray-500 uppercase tracking-wide dark:text-slate-400", "Actions" }
                                }
                            }
                            tbody { class: "divide-y divide-gray-100 dark:divide-slate-700",
                                for row in rows {
                                    {
                                        let is_self = row.token == current_user_token;
                                        let is_super = row.role_sort_key() == 0;
                                        let can_edit = is_super_admin || !is_super;
                                        let row_edit = row.clone();
                                        let row_del = row.clone();
                                        rsx! {
                                            tr { class: "hover:bg-gray-50 dark:hover:bg-slate-700",
                                                td { class: "px-4 py-3 font-medium text-gray-900 dark:text-slate-100", "{row.username}" }
                                                td { class: "px-4 py-3 text-gray-600 dark:text-slate-400", "{row.full_name}" }
                                                td { class: "px-4 py-3",
                                                    span {
                                                        class: match row.role_sort_key() {
                                                            0 => "inline-flex items-center px-2 py-0.5 rounded text-xs font-medium bg-purple-100 text-purple-800 dark:bg-purple-900 dark:text-purple-300",
                                                            1 => "inline-flex items-center px-2 py-0.5 rounded text-xs font-medium bg-indigo-100 text-indigo-800 dark:bg-indigo-900 dark:text-indigo-300",
                                                            _ => "inline-flex items-center px-2 py-0.5 rounded text-xs font-medium bg-gray-100 text-gray-700 dark:bg-slate-700 dark:text-slate-300",
                                                        },
                                                        { row.role_label() }
                                                    }
                                                }
                                                td { class: "px-4 py-3",
                                                    div { class: "flex items-center justify-end gap-2",
                                                        button {
                                                            class: if can_edit {
                                                                "p-1.5 text-gray-500 hover:text-indigo-600 hover:bg-indigo-50 dark:hover:bg-indigo-900/30 rounded"
                                                            } else {
                                                                "p-1.5 text-gray-300 cursor-not-allowed rounded dark:text-slate-600"
                                                            },
                                                            disabled: !can_edit,
                                                            title: "Edit user",
                                                            onclick: move |_| {
                                                                if can_edit {
                                                                    modal_target.set(Some(Some(row_edit.clone())));
                                                                }
                                                            },
                                                            "✎"
                                                        }
                                                        if !is_super && !is_self {
                                                            button {
                                                                class: "p-1.5 text-gray-500 hover:text-red-600 hover:bg-red-50 dark:hover:bg-red-900/30 rounded",
                                                                title: "Delete user",
                                                                onclick: move |_| {
                                                                    delete_error.set(None);
                                                                    delete_target.set(Some(row_del.clone()));
                                                                },
                                                                "✕"
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        if let Some(Ok(ref rows)) = users_resource() {
                            if rows.is_empty() {
                                div { class: "px-4 py-8 text-center text-gray-400 text-sm dark:text-slate-500", "No users found." }
                            }
                        }
                    }
                },
            }
        }

        // ── User create/edit modal ──────────────────────────────────────────
        if let Some(target) = modal_target() {
            UserModal {
                is_self: target.as_ref().is_some_and(|r| r.token == current_user_token),
                editing: target,
                is_super_admin,
                on_close: move || modal_target.set(None),
                on_saved: move || {
                    modal_target.set(None);
                    *refresh.write() += 1;
                },
            }
        }

        // ── Delete confirmation dialog ──────────────────────────────────────
        if let Some(target) = delete_target() {
            div {
                class: "fixed inset-0 z-50 flex items-center justify-center bg-black/40",
                tabindex: -1,
                onmounted: move |e| async move { let _ = e.set_focus(true).await; },
                onkeydown: move |e| { if e.key() == Key::Escape { delete_target.set(None); } },
                div { class: "bg-white rounded-2xl shadow-xl w-full max-w-sm p-6 dark:bg-slate-800",
                    h3 { class: "text-base font-semibold text-gray-900 mb-2 dark:text-slate-100", "Delete User" }
                    p { class: "text-sm text-gray-600 mb-6 dark:text-slate-400",
                        "Are you sure you want to delete "
                        span { class: "font-medium text-gray-900 dark:text-slate-100", "{target.username}" }
                        "? This cannot be undone."
                    }
                    div { class: "flex justify-end gap-3",
                        button {
                            class: "px-4 py-2 text-sm font-medium rounded-lg border border-gray-300 text-gray-700 hover:bg-gray-50 dark:border-slate-600 dark:text-slate-300 dark:hover:bg-slate-700",
                            disabled: deleting(),
                            onclick: move |_| delete_target.set(None),
                            "Cancel"
                        }
                        button {
                            class: "px-4 py-2 text-sm font-medium rounded-lg bg-red-600 text-white hover:bg-red-700 disabled:opacity-50",
                            disabled: deleting(),
                            onclick: move |_| {
                                let tok = target.token.clone();
                                deleting.set(true);
                                spawn(async move {
                                    match admin_delete_user(tok).await {
                                        Ok(()) => {
                                            delete_target.set(None);
                                            *refresh.write() += 1;
                                        }
                                        Err(e) => {
                                            delete_error.set(Some(e.to_string()));
                                            delete_target.set(None);
                                        }
                                    }
                                    deleting.set(false);
                                });
                            },
                            if deleting() { "Deleting…" } else { "Delete" }
                        }
                    }
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// UserModal component
// ---------------------------------------------------------------------------

#[derive(Clone, PartialEq)]
enum RoleChoice {
    SuperAdmin,
    Admin,
    User,
}

impl RoleChoice {
    fn from_caps(caps: &[String]) -> Self {
        if caps.iter().any(|c| c == "SuperAdmin") {
            Self::SuperAdmin
        } else if caps.iter().any(|c| c == "Admin") {
            Self::Admin
        } else {
            Self::User
        }
    }
}

#[component]
fn UserModal(editing: Option<UserAdminRow>, is_self: bool, is_super_admin: bool, on_close: EventHandler<()>, on_saved: EventHandler<()>) -> Element {
    let is_edit = editing.is_some();

    let initial_role = editing.as_ref().map_or(RoleChoice::User, |r| RoleChoice::from_caps(&r.capabilities));
    let editing_is_super = initial_role == RoleChoice::SuperAdmin;

    let mut username = use_signal(|| editing.as_ref().map(|r| r.username.clone()).unwrap_or_default());
    let mut full_name = use_signal(|| editing.as_ref().map(|r| r.full_name.clone()).unwrap_or_default());
    let mut email = use_signal(|| editing.as_ref().map(|r| r.email.clone()).unwrap_or_default());
    let mut password = use_signal(String::new);
    let mut role = use_signal(|| initial_role);
    let mut error_msg: Signal<Option<String>> = use_signal(|| None);
    let mut saving = use_signal(|| false);
    let mut generating = use_signal(|| false);

    rsx! {
        div {
            class: "fixed inset-0 z-50 flex items-center justify-center bg-black/40",
            tabindex: -1,
            onmounted: move |e| async move { let _ = e.set_focus(true).await; },
            onkeydown: move |e| {
                if e.key() == Key::Escape {
                    on_close.call(());
                }
            },
            div { class: "bg-white rounded-2xl shadow-xl w-full max-w-lg max-h-[90vh] overflow-y-auto dark:bg-slate-800",
                div { class: "p-6",
                    h3 { class: "text-base font-semibold text-gray-900 mb-5 dark:text-slate-100",
                        if is_edit { "Edit User" } else { "Add User" }
                    }

                    if let Some(msg) = error_msg() {
                        div { class: "mb-4 p-3 bg-red-50 border border-red-200 text-red-700 rounded-lg text-sm dark:bg-red-900/30 dark:border-red-800 dark:text-red-400",
                            "{msg}"
                        }
                    }

                    // Username
                    div { class: "mb-4",
                        label { class: "block text-sm font-medium text-gray-700 mb-1 dark:text-slate-300", "Username" }
                        input {
                            r#type: "text",
                            class: if is_edit {
                                "w-full px-3 py-2 border border-gray-200 rounded-lg bg-gray-50 text-gray-500 cursor-not-allowed text-sm dark:border-slate-700 dark:bg-slate-700/50 dark:text-slate-400"
                            } else {
                                "w-full px-3 py-2 border border-gray-300 rounded-lg focus:outline-hidden focus:ring-2 focus:ring-indigo-500 text-sm dark:bg-slate-700 dark:border-slate-600 dark:text-slate-100"
                            },
                            placeholder: "username",
                            value: username,
                            readonly: is_edit,
                            oninput: move |e| {
                                if !is_edit { username.set(e.value()); }
                            },
                            disabled: saving,
                        }
                    }

                    // Full Name
                    div { class: "mb-4",
                        label { class: "block text-sm font-medium text-gray-700 mb-1 dark:text-slate-300", "Full Name" }
                        input {
                            r#type: "text",
                            class: "w-full px-3 py-2 border border-gray-300 rounded-lg focus:outline-hidden focus:ring-2 focus:ring-indigo-500 text-sm dark:bg-slate-700 dark:border-slate-600 dark:text-slate-100",
                            placeholder: "Full name",
                            value: full_name,
                            oninput: move |e| full_name.set(e.value()),
                            disabled: saving,
                        }
                    }

                    // Email
                    div { class: "mb-4",
                        label { class: "block text-sm font-medium text-gray-700 mb-1 dark:text-slate-300", "Email Address" }
                        input {
                            r#type: "email",
                            class: "w-full px-3 py-2 border border-gray-300 rounded-lg focus:outline-hidden focus:ring-2 focus:ring-indigo-500 text-sm dark:bg-slate-700 dark:border-slate-600 dark:text-slate-100",
                            placeholder: "user@example.com",
                            value: email,
                            oninput: move |e| email.set(e.value()),
                            disabled: saving,
                        }
                    }

                    // Password
                    div { class: "mb-4",
                        label { class: "block text-sm font-medium text-gray-700 mb-1 dark:text-slate-300", "Password" }
                        if is_edit {
                            p { class: "text-xs text-gray-400 mb-1 dark:text-slate-500",
                                if is_self {
                                    "Leave blank to keep your current password."
                                } else {
                                    "Leave blank to keep current password. Setting a new password will require the user to change it on next login."
                                }
                            }
                        }
                        div { class: "flex gap-2",
                            input {
                                r#type: "text",
                                class: "flex-1 min-w-0 px-3 py-2 border border-gray-300 rounded-lg focus:outline-hidden focus:ring-2 focus:ring-indigo-500 text-sm font-mono dark:bg-slate-700 dark:border-slate-600 dark:text-slate-100",
                                placeholder: if is_edit { "New password (optional)" } else { "Password" },
                                value: password,
                                oninput: move |e| password.set(e.value()),
                                disabled: saving,
                            }
                            button {
                                class: "px-3 py-2 text-xs font-medium rounded-lg border border-gray-300 text-gray-700 hover:bg-gray-50 disabled:opacity-50 whitespace-nowrap dark:border-slate-600 dark:text-slate-300 dark:hover:bg-slate-700",
                                disabled: saving() || generating(),
                                title: "Generate password",
                                onclick: move |_| {
                                    generating.set(true);
                                    spawn(async move {
                                        match generate_password().await {
                                            Ok(pw) => password.set(pw),
                                            Err(e) => error_msg.set(Some(e.to_string())),
                                        }
                                        generating.set(false);
                                    });
                                },
                                if generating() { "…" } else { "Generate" }
                            }
                            button {
                                class: "px-3 py-2 text-xs font-medium rounded-lg border border-gray-300 text-gray-700 hover:bg-gray-50 disabled:opacity-50 dark:border-slate-600 dark:text-slate-300 dark:hover:bg-slate-700",
                                disabled: password().is_empty(),
                                title: "Copy to clipboard",
                                onclick: move |_| {
                                    let pw = password();
                                    if !pw.is_empty() {
                                        // Escape backticks for JS template literal safety.
                                        let escaped = pw.replace('`', "\\`").replace('$', "\\$");
                                        spawn(async move {
                                            let _ = document::eval(&format!("navigator.clipboard.writeText(`{escaped}`)")).await;
                                        });
                                    }
                                },
                                "Copy"
                            }
                        }
                    }

                    // Role
                    div { class: "mb-4",
                        label { class: "block text-sm font-medium text-gray-700 mb-1 dark:text-slate-300", "Role" }
                        select {
                            class: if editing_is_super {
                                "w-full px-3 py-2 border border-gray-200 rounded-lg text-sm bg-gray-50 text-gray-500 cursor-not-allowed dark:border-slate-700 dark:bg-slate-700/50 dark:text-slate-400"
                            } else {
                                "w-full px-3 py-2 border border-gray-300 rounded-lg focus:outline-hidden focus:ring-2 focus:ring-indigo-500 text-sm bg-white dark:bg-slate-700 dark:border-slate-600 dark:text-slate-100"
                            },
                            disabled: saving() || editing_is_super,
                            onchange: move |e| {
                                let new_role = match e.value().as_str() {
                                    "SuperAdmin" => RoleChoice::SuperAdmin,
                                    "Admin" => RoleChoice::Admin,
                                    _ => RoleChoice::User,
                                };
                                role.set(new_role);
                            },
                            option { value: "User", selected: role() == RoleChoice::User, "User" }
                            option {
                                value: "Admin",
                                selected: role() == RoleChoice::Admin,
                                disabled: !is_super_admin,
                                "Admin"
                            }
                            option {
                                value: "SuperAdmin",
                                selected: role() == RoleChoice::SuperAdmin,
                                disabled: true,
                                "Super Admin"
                            }
                        }
                    }

                    // Actions
                    div { class: "flex justify-end gap-3 pt-2",
                        button {
                            class: "px-4 py-2 text-sm font-medium rounded-lg border border-gray-300 text-gray-700 hover:bg-gray-50 dark:border-slate-600 dark:text-slate-300 dark:hover:bg-slate-700",
                            disabled: saving(),
                            onclick: move |_| on_close.call(()),
                            "Cancel"
                        }
                        button {
                            class: "px-4 py-2 text-sm font-medium rounded-lg bg-indigo-600 text-white hover:bg-indigo-700 disabled:opacity-50",
                            disabled: saving(),
                            onclick: move |_| {
                                let un = username().trim().to_string();
                                let fn_ = full_name().trim().to_string();
                                let em = email().trim().to_string();
                                let pw = password();
                                let chosen_role = role();

                                if !is_edit && un.is_empty() {
                                    error_msg.set(Some("Username is required.".to_string()));
                                    return;
                                }
                                if fn_.is_empty() {
                                    error_msg.set(Some("Full name is required.".to_string()));
                                    return;
                                }
                                if em.is_empty() {
                                    error_msg.set(Some("Email address is required.".to_string()));
                                    return;
                                }
                                if !is_edit && pw.is_empty() {
                                    error_msg.set(Some("Password is required for new users.".to_string()));
                                    return;
                                }
                                let capabilities: Vec<String> = match chosen_role {
                                    RoleChoice::SuperAdmin => vec!["SuperAdmin".to_string()],
                                    RoleChoice::Admin => vec!["Admin".to_string()],
                                    RoleChoice::User => vec![],
                                };

                                let edit_token = editing.as_ref().map(|r| r.token.clone());
                                error_msg.set(None);
                                saving.set(true);

                                spawn(async move {
                                    // Step 1: create or update the user
                                    // For create, admin_create_user returns the new user's token
                                    // directly so we avoid a list_users round-trip.
                                    let _target_token: Option<String> = if let Some(tok) = edit_token.clone() {
                                        let pw_opt = if pw.is_empty() { None } else { Some(pw) };
                                        match admin_update_user(tok.clone(), fn_, em, pw_opt, capabilities).await {
                                            Err(ServerFnError::ServerError { message, .. }) => {
                                                error_msg.set(Some(message));
                                                saving.set(false);
                                                return;
                                            }
                                            Err(e) => {
                                                error_msg.set(Some(e.to_string()));
                                                saving.set(false);
                                                return;
                                            }
                                            Ok(()) => Some(tok),
                                        }
                                    } else {
                                        match admin_create_user(un.clone(), fn_, em, pw, capabilities).await {
                                            Err(ServerFnError::ServerError { message, .. }) => {
                                                error_msg.set(Some(message));
                                                saving.set(false);
                                                return;
                                            }
                                            Err(e) => {
                                                error_msg.set(Some(e.to_string()));
                                                saving.set(false);
                                                return;
                                            }
                                            Ok(new_token) => Some(new_token),
                                        }
                                    };

                                    on_saved.call(());
                                });
                            },
                            if saving() { "Saving…" } else { "Save" }
                        }
                    }
                }
            }
        }
    }
}
