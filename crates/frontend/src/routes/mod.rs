#[cfg(feature = "server")]
pub(crate) mod server_helpers;

pub(crate) mod profile_page;
pub(crate) mod settings_page;

pub(crate) use profile_page::ProfilePage;
pub(crate) use settings_page::SettingsPage;
