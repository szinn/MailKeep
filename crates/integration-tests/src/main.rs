mod context;

#[cfg(feature = "sqlite")]
mod sqlite;

#[cfg(feature = "sqlite")]
mod jobs;

pub(crate) use sqlite::setup;

#[tokio::test]
async fn test_setup() {
    let _ctx = setup().await;
}
