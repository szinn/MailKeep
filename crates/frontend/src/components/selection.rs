use dioxus::prelude::*;

/// The account token currently selected in the HomePage list, or `None` for the
/// empty-selection state (global statistics). A global signal so both the row
/// toggle and the nav-bar Home button can drive the right panel — mirrors the
/// `ACCOUNTS_REVISION` / `THEME_MODE` idiom.
pub(crate) static SELECTED_ACCOUNT: GlobalSignal<Option<String>> = Signal::global(|| None);
