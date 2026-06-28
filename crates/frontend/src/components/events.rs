use dioxus::prelude::*;

/// Bumped whenever the server signals an account change (MK-19). Views that
/// display account data subscribe by reading this inside a reactive closure
/// (e.g. a `use_resource`), so a bump triggers a re-fetch.
pub(crate) static ACCOUNTS_REVISION: GlobalSignal<u32> = Signal::global(|| 0);
