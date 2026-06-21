#[cfg(feature = "server")]
pub(crate) mod server_helpers;

pub(crate) mod account_add_page;
pub(crate) mod home_page;
pub(crate) mod landing_page;
pub(crate) mod profile_page;
pub(crate) mod settings_page;

pub(crate) use home_page::HomePage;
pub(crate) use landing_page::LandingPage;
pub(crate) use profile_page::ProfilePage;
pub(crate) use settings_page::SettingsPage;
