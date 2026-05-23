use dioxus::prelude::*;
#[cfg(feature = "server")]
use {crate::routes::server_helpers::authenticated_user, crate::server::AuthSession};

use crate::Route;

#[get(
    "/api/v1/home/context",
    auth_session: axum::Extension<AuthSession>,
)]
async fn get_home_context() -> Result<(), ServerFnError> {
    authenticated_user(&auth_session)?;
    Ok(())
}

#[component]
pub(crate) fn HomePage() -> Element {
    let navigator = use_navigator();
    let auth = use_server_future(get_home_context)?;

    use_effect(move || {
        if let Some(Err(_)) = auth() {
            navigator.replace(Route::LandingPage { login_failed: None });
        }
    });

    rsx! {
        div { class: "flex-1 overflow-auto p-8",
            div { class: "max-w-2xl mx-auto",
                h1 { class: "text-2xl font-semibold text-gray-900 dark:text-slate-100 mb-4",
                    "MailKeep"
                }
                p { class: "text-gray-600 dark:text-slate-400",
                    "Welcome. This is a placeholder home page."
                }
            }
        }
    }
}
