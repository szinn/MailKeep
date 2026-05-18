use dioxus::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[allow(clippy::unsafe_derive_deserialize)]
pub(crate) enum ThemeMode {
    #[default]
    System,
    Light,
    Dark,
}

impl ThemeMode {
    pub(crate) fn cycle(self) -> Self {
        match self {
            Self::System => Self::Light,
            Self::Light => Self::Dark,
            Self::Dark => Self::System,
        }
    }

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::Light => "light",
            Self::Dark => "dark",
        }
    }

    pub(crate) fn from_str(s: &str) -> Self {
        match s {
            "light" => Self::Light,
            "dark" => Self::Dark,
            _ => Self::System,
        }
    }

    pub(crate) fn icon(self) -> Element {
        match self {
            Self::System => rsx! {
                svg {
                    class: "w-5 h-5",
                    xmlns: "http://www.w3.org/2000/svg",
                    fill: "none",
                    view_box: "0 0 24 24",
                    stroke_width: "1.5",
                    stroke: "currentColor",
                    path {
                        stroke_linecap: "round",
                        stroke_linejoin: "round",
                        d: "M9 17.25v1.007a3 3 0 0 1-.879 2.122L7.5 21h9l-.621-.621A3 3 0 0 1 15 18.257V17.25m6-12V15a2.25 2.25 0 0 1-2.25 2.25H5.25A2.25 2.25 0 0 1 3 15V5.25m18 0A2.25 2.25 0 0 0 18.75 3H5.25A2.25 2.25 0 0 0 3 5.25m18 0H3",
                    }
                }
            },
            Self::Light => rsx! {
                svg {
                    class: "w-5 h-5",
                    xmlns: "http://www.w3.org/2000/svg",
                    fill: "none",
                    view_box: "0 0 24 24",
                    stroke_width: "1.5",
                    stroke: "currentColor",
                    path {
                        stroke_linecap: "round",
                        stroke_linejoin: "round",
                        d: "M12 3v2.25m6.364.386-1.591 1.591M21 12h-2.25m-.386 6.364-1.591-1.591M12 18.75V21m-4.773-4.227-1.591 1.591M5.25 12H3m4.227-4.773L5.636 5.636M15.75 12a3.75 3.75 0 1 1-7.5 0 3.75 3.75 0 0 1 7.5 0Z",
                    }
                }
            },
            Self::Dark => rsx! {
                svg {
                    class: "w-5 h-5",
                    xmlns: "http://www.w3.org/2000/svg",
                    fill: "none",
                    view_box: "0 0 24 24",
                    stroke_width: "1.5",
                    stroke: "currentColor",
                    path {
                        stroke_linecap: "round",
                        stroke_linejoin: "round",
                        d: "M21.752 15.002A9.72 9.72 0 0 1 18 15.75c-5.385 0-9.75-4.365-9.75-9.75 0-1.33.266-2.597.748-3.752A9.753 9.753 0 0 0 3 11.25C3 16.635 7.365 21 12.75 21a9.753 9.753 0 0 0 9.002-5.998Z",
                    }
                }
            },
        }
    }
}

pub(crate) static THEME_MODE: GlobalSignal<ThemeMode> = Signal::global(ThemeMode::default);

// ── Server functions
// ──────────────────────────────────────────────────────────

#[cfg(feature = "server")]
use {
    crate::routes::server_helpers::{authenticated_user, to_server_err},
    crate::server::AuthSession,
    mk_core::CoreServices,
    std::sync::Arc,
};

const THEME_SETTING_KEY: &str = "ui_theme";

#[get(
    "/api/v1/settings/theme",
    auth_session: axum::Extension<AuthSession>,
    core_services: axum::Extension<Arc<CoreServices>>
)]
pub(crate) async fn get_theme_preference() -> Result<Option<ThemeMode>, ServerFnError> {
    let user_id = authenticated_user(&auth_session)?.id();
    let setting = core_services
        .user_setting_service
        .get(user_id, THEME_SETTING_KEY)
        .await
        .map_err(to_server_err)?;
    Ok(setting.map(|s| ThemeMode::from_str(&s.value)))
}

#[post(
    "/api/v1/settings/theme",
    auth_session: axum::Extension<AuthSession>,
    core_services: axum::Extension<Arc<CoreServices>>
)]
pub(crate) async fn set_theme_preference(mode: ThemeMode) -> Result<(), ServerFnError> {
    let user_id = authenticated_user(&auth_session)?.id();
    core_services
        .user_setting_service
        .set(user_id, THEME_SETTING_KEY, mode.as_str())
        .await
        .map_err(to_server_err)?;
    Ok(())
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cycle_rotates_system_light_dark_system() {
        assert_eq!(ThemeMode::System.cycle(), ThemeMode::Light);
        assert_eq!(ThemeMode::Light.cycle(), ThemeMode::Dark);
        assert_eq!(ThemeMode::Dark.cycle(), ThemeMode::System);
    }

    #[test]
    fn as_str_from_str_round_trip() {
        for mode in [ThemeMode::System, ThemeMode::Light, ThemeMode::Dark] {
            assert_eq!(ThemeMode::from_str(mode.as_str()), mode);
        }
    }

    #[test]
    fn from_str_unknown_defaults_to_system() {
        assert_eq!(ThemeMode::from_str("bogus"), ThemeMode::System);
        assert_eq!(ThemeMode::from_str(""), ThemeMode::System);
    }

    #[test]
    fn icon_returns_element_for_each_mode() {
        // Just verify each variant produces an Element without panicking.
        let _ = ThemeMode::System.icon();
        let _ = ThemeMode::Light.icon();
        let _ = ThemeMode::Dark.icon();
    }
}
