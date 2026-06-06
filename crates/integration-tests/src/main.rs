mod context;

#[cfg(feature = "sqlite")]
mod sqlite;

#[cfg(feature = "sqlite")]
mod jobs;

#[cfg(feature = "sqlite")]
mod folder_message;

pub(crate) use sqlite::setup;

#[tokio::test]
async fn test_setup() {
    let _ctx = setup().await;
}
