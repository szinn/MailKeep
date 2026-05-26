use sea_orm_migration::{
    prelude::*,
    schema::{big_integer, binary, boolean, string, text, timestamp_with_time_zone},
};

use super::m20260520_000001_create_users_table::Users;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(Accounts::Table)
                    .if_not_exists()
                    .col(big_integer(Accounts::Id).primary_key())
                    .col(big_integer(Accounts::Version))
                    .col(string(Accounts::Token).unique_key())
                    .col(big_integer(Accounts::UserId))
                    .col(string(Accounts::DisplayName))
                    .col(string(Accounts::EmailAddress))
                    .col(text(Accounts::Server))
                    .col(string(Accounts::Username))
                    .col(binary(Accounts::Credentials))
                    .col(boolean(Accounts::Enabled).default(true))
                    .col(string(Accounts::Status).default("PendingFirstSync"))
                    .col(text(Accounts::LastError).null())
                    .col(timestamp_with_time_zone(Accounts::LastSyncedAt).null())
                    .col(timestamp_with_time_zone(Accounts::CreatedAt))
                    .col(timestamp_with_time_zone(Accounts::UpdatedAt))
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_accounts_user")
                            .from(Accounts::Table, Accounts::UserId)
                            .to(Users::Table, Users::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_accounts_user_id")
                    .table(Accounts::Table)
                    .col(Accounts::UserId)
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_accounts_enabled")
                    .table(Accounts::Table)
                    .col(Accounts::Enabled)
                    .to_owned(),
            )
            .await?;

        Ok(())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Ok(())
    }
}

#[derive(DeriveIden)]
pub(crate) enum Accounts {
    Table,
    Id,
    Version,
    Token,
    UserId,
    DisplayName,
    EmailAddress,
    Server,
    Username,
    Credentials,
    Enabled,
    Status,
    LastError,
    LastSyncedAt,
    CreatedAt,
    UpdatedAt,
}
