use sea_orm_migration::{
    prelude::*,
    schema::{big_integer, boolean, string, text, timestamp_with_time_zone},
};

use super::m20260525_000005_create_accounts_table::Accounts;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(Folders::Table)
                    .if_not_exists()
                    .col(big_integer(Folders::Id).primary_key())
                    .col(big_integer(Folders::Version))
                    .col(string(Folders::Token).unique_key())
                    .col(big_integer(Folders::AccountId))
                    .col(text(Folders::Path))
                    .col(text(Folders::DisplayName).null())
                    .col(text(Folders::SpecialUse).null())
                    .col(boolean(Folders::Enabled).default(true))
                    .col(boolean(Folders::IdleEnabled).default(false))
                    .col(big_integer(Folders::Uidvalidity).null())
                    .col(big_integer(Folders::LastUid).default(0))
                    .col(timestamp_with_time_zone(Folders::LastSyncedAt).null())
                    .col(text(Folders::LastError).null())
                    .col(timestamp_with_time_zone(Folders::CreatedAt))
                    .col(timestamp_with_time_zone(Folders::UpdatedAt))
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_folders_account")
                            .from(Folders::Table, Folders::AccountId)
                            .to(Accounts::Table, Accounts::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_folders_account_path")
                    .table(Folders::Table)
                    .col(Folders::AccountId)
                    .col(Folders::Path)
                    .unique()
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
pub(crate) enum Folders {
    Table,
    Id,
    Version,
    Token,
    AccountId,
    Path,
    DisplayName,
    SpecialUse,
    Enabled,
    IdleEnabled,
    Uidvalidity,
    LastUid,
    LastSyncedAt,
    LastError,
    CreatedAt,
    UpdatedAt,
}
