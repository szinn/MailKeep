#[cfg(feature = "sqlite")]
mod context;

#[cfg(feature = "sqlite")]
mod sqlite;

#[cfg(feature = "sqlite")]
mod jobs;

#[cfg(feature = "sqlite")]
mod folder_message;

#[cfg(feature = "sqlite")]
mod ingest;

#[cfg(feature = "greenmail")]
mod greenmail;

#[cfg(feature = "sqlite")]
pub(crate) use sqlite::setup;

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn test_setup() {
    let _ctx = setup().await;
}
