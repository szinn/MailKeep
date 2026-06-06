pub use sea_orm_migration::prelude::*;

mod m20260520_000001_create_users_table;
mod m20260520_000002_create_sessions_table;
mod m20260520_000003_create_user_settings_table;
mod m20260524_000004_create_jobs_table;
mod m20260525_000005_create_accounts_table;
mod m20260606_000006_create_folders_table;

pub struct Migrator;

#[async_trait::async_trait]
impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![
            Box::new(m20260520_000001_create_users_table::Migration),
            Box::new(m20260520_000002_create_sessions_table::Migration),
            Box::new(m20260520_000003_create_user_settings_table::Migration),
            Box::new(m20260524_000004_create_jobs_table::Migration),
            Box::new(m20260525_000005_create_accounts_table::Migration),
            Box::new(m20260606_000006_create_folders_table::Migration),
        ]
    }
}
