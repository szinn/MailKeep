use dioxus::prelude::*;

#[cfg(feature = "server")]
use crate::server::AuthSession;
use crate::{
    Route,
    components::{NavBar, THEME_MODE, theme::get_theme_preference},
};

#[get("/api/v1/check_auth", auth_session: axum::Extension<AuthSession>)]
async fn check_auth() -> Result<bool, ServerFnError> {
    Ok(auth_session.current_user.as_ref().is_some_and(|u| !u.username.is_empty()))
}

#[component]
pub(crate) fn AppLayout() -> Element {
    // Load persisted theme preference once; write to global signal.
    let theme_pref = use_server_future(get_theme_preference);
    use_effect(move || {
        if let Ok(res) = theme_pref {
            if let Some(Ok(Some(mode))) = res() {
                *THEME_MODE.write() = mode;
            }
        }
    });

    // Fallback: read localStorage so the icon and class are correct even when
    // the server future is unavailable (e.g. first hydration before SSR data
    // arrives). Runs once after mount, only on the WASM client.
    use_hook(move || {
        spawn(async move {
            if let Ok(val) = document::eval("return localStorage.getItem('mk_theme') || ''").await {
                if let Some(s) = val.as_str() {
                    if !s.is_empty() {
                        use crate::components::theme::ThemeMode;
                        if *THEME_MODE.peek() == ThemeMode::System {
                            *THEME_MODE.write() = ThemeMode::from_str(s);
                        }
                    }
                }
            }
        });
    });

    // Apply dark class to <html> whenever THEME_MODE changes.
    // localStorage is written only in ThemeToggle (on explicit user action) to
    // avoid corrupting the value before the saved preference has been loaded.
    // spawn is required — document::eval doesn't execute from a synchronous
    // use_effect body. No .await so the eval queue never blocks.
    use_effect(move || {
        let _ = *THEME_MODE.read();
        spawn(async move {
            let mode = *THEME_MODE.peek();
            document::eval(&format!(
                "(function(){{var m={:?};var \
                 dark=m==='dark'||(m==='system'&&window.matchMedia('(prefers-color-scheme:dark)').matches);document.documentElement.classList.toggle('dark',\
                 dark);}})()",
                mode.as_str()
            ));
        });
    });

    // Exit selection mode and clear search whenever the route changes.
    let route = use_route::<Route>();
    use_effect(move || {
        // Subscribe to route changes so the effect re-runs on navigation.
        let _ = &route;
    });

    rsx! {
        document::Stylesheet { href: asset!("/assets/tailwind.css") }
        document::Link { rel: "icon", href: asset!("/assets/favicon.ico") }
        document::Link { rel: "apple-touch-icon", sizes: "180x180", href: asset!("/assets/apple-touch-icon.png") }
        document::Link { rel: "apple-touch-icon", sizes: "32x32", href: asset!("/assets/favicon-32x32.png") }
        document::Link { rel: "apple-touch-icon", sizes: "16x16", href: asset!("/assets/favicon-16x16.png") }
        div { class: "h-screen flex flex-col bg-gray-50 dark:bg-slate-900 text-gray-900 dark:text-slate-100",
            NavBar {}
            main { class: "flex-1 flex overflow-hidden",
                SuspenseBoundary {
                    fallback: |_| rsx! {},
                    AuthGate {}
                }
            }
        }
    }
}

/// Wraps the Outlet so that only the page content area suspends during the auth
/// check, leaving the `NavBar` visible immediately.
#[component]
fn AuthGate() -> Element {
    let navigator = use_navigator();
    let auth = use_server_future(check_auth)?;

    use_effect(move || {
        if let Some(Ok(false)) = auth() {
            navigator.replace(Route::LandingPage { login_failed: None });
        }
    });

    rsx! { Outlet::<Route> {} }
}
