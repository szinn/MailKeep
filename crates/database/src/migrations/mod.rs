pub use sea_orm_migration::prelude::*;

mod m20260520_000001_create_users_table;
mod m20260520_000002_create_sessions_table;
mod m20260520_000003_create_user_settings_table;
mod m20260524_000004_create_jobs_table;
mod m20260525_000005_create_accounts_table;
mod m20260606_000006_create_folders_table;
mod m20260606_000007_create_messages_table;
mod m20260606_000008_create_message_locations_table;
mod m20260606_000009_create_message_attachments_table;
mod m20260621_000010_message_identity_by_content_hash;

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
            Box::new(m20260606_000007_create_messages_table::Migration),
            Box::new(m20260606_000008_create_message_locations_table::Migration),
            Box::new(m20260606_000009_create_message_attachments_table::Migration),
            Box::new(m20260621_000010_message_identity_by_content_hash::Migration),
        ]
    }
}
