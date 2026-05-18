// SeaORM uses i64 for all primary keys; domain types use u64. Auto-increment
// IDs are always positive and will not exceed i64::MAX in practice, so these
// casts are safe at this boundary. Page sizes/counts are similarly bounded.
#![allow(
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::cast_possible_truncation,
    reason = "SeaORM i64/u64 boundary — IDs and page values are always in range"
)]

use std::sync::Arc;

use mk_core::{
    Error,
    auth::SessionRepository,
    repository::{Repository, RepositoryService, RepositoryServiceBuilder},
    user::{UserRepository, UserSettingRepository},
};
use sea_orm::{ConnectOptions, Database, DatabaseConnection};
use sea_orm_migration::MigratorTrait;

pub mod error;

pub use error::*;

mod adapters;
mod entities;
mod migrations;
mod repository;
mod transaction;

use crate::{
    adapters::{session::SessionRepositoryAdapter, user::UserRepositoryAdapter, user_settings::UserSettingRepositoryAdapter},
    migrations::Migrator,
    repository::RepositoryImpl,
};

pub async fn open_database(database_path: &str) -> Result<DatabaseConnection, Error> {
    tracing::debug!("Connecting to database...");
    let mut opt = ConnectOptions::new(database_path);
    opt.max_connections(9)
        .min_connections(5)
        .sqlx_logging(true)
        .sqlx_logging_level(tracing::log::LevelFilter::Info);

    // For SQLite, apply PRAGMAs that sqlx does not set by default.
    // We use map_sqlx_sqlite_opts rather than URL query parameters because
    // sqlx-sqlite's URL parser only recognises mode/cache/immutable/vfs —
    // pragma names are not valid query parameters and will cause a parse error.
    if database_path.starts_with("sqlite:") {
        opt.map_sqlx_sqlite_opts(|o| {
            use std::time::Duration;

            use sqlx::sqlite::{SqliteJournalMode, SqliteSynchronous};
            o.journal_mode(SqliteJournalMode::Wal)
                .busy_timeout(Duration::from_secs(5))
                .synchronous(SqliteSynchronous::Normal)
                .foreign_keys(true)
        });
    }

    Ok(Database::connect(opt).await.map_err(handle_dberr)?)
}

pub async fn create_repository_service(database: DatabaseConnection) -> Result<Arc<RepositoryService>, Error> {
    let span = tracing::span!(tracing::Level::TRACE, "Migrations").entered();
    Migrator::up(&database, None).await.map_err(handle_dberr)?;
    span.exit();

    let repository_service = RepositoryServiceBuilder::default()
        .repository(Arc::new(RepositoryImpl::new(database)) as Arc<dyn Repository>)
        .session_repository(Arc::new(SessionRepositoryAdapter::new()) as Arc<dyn SessionRepository>)
        .user_repository(Arc::new(UserRepositoryAdapter::new()) as Arc<dyn UserRepository>)
        .user_setting_repository(Arc::new(UserSettingRepositoryAdapter::new()) as Arc<dyn UserSettingRepository>)
        .build()
        .map_err(|e| Error::Infrastructure(e.to_string()))?;

    Ok(Arc::new(repository_service))
}

#[cfg(test)]
mod tests {
    use sea_orm::{ConnectionTrait, Statement};

    use super::*;

    /// Verify that opening a SQLite database via `open_database` configures
    /// the four PRAGMAs we care about. Uses a tempfile-backed database because
    /// `sqlite::memory:` is single-connection and cannot exhibit the locking
    /// behaviour this configuration is designed to fix.
    #[tokio::test]
    async fn open_database_sets_sqlite_pragmas() {
        let dir = tempfile::tempdir().expect("create tempdir");
        let db_path = dir.path().join("bb-pragma-test.sqlite");
        let url = format!("sqlite://{}?mode=rwc", db_path.display());

        let db = open_database(&url).await.expect("open sqlite database");

        let backend = db.get_database_backend();

        let pragma = |name: &str| Statement::from_string(backend, format!("PRAGMA {name}"));

        let journal = db
            .query_one_raw(pragma("journal_mode"))
            .await
            .expect("query journal_mode")
            .expect("journal_mode row");
        let mode: String = journal.try_get("", "journal_mode").expect("journal_mode value");
        assert_eq!(mode.to_lowercase(), "wal", "journal_mode should be WAL");

        let busy = db
            .query_one_raw(pragma("busy_timeout"))
            .await
            .expect("query busy_timeout")
            .expect("busy_timeout row");
        let timeout: i32 = busy.try_get("", "timeout").expect("busy_timeout value");
        assert_eq!(timeout, 5000, "busy_timeout should be 5000ms");

        let sync = db
            .query_one_raw(pragma("synchronous"))
            .await
            .expect("query synchronous")
            .expect("synchronous row");
        let sync_val: i32 = sync.try_get("", "synchronous").expect("synchronous value");
        assert_eq!(sync_val, 1, "synchronous should be NORMAL (1)");

        let fk = db
            .query_one_raw(pragma("foreign_keys"))
            .await
            .expect("query foreign_keys")
            .expect("foreign_keys row");
        let fk_val: i32 = fk.try_get("", "foreign_keys").expect("foreign_keys value");
        assert_eq!(fk_val, 1, "foreign_keys should be ON (1)");
    }
}
