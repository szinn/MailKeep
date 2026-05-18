use sea_orm_migration::{
    prelude::*,
    schema::{big_integer, string, text, timestamp_with_time_zone},
};

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(UserSettings::Table)
                    .if_not_exists()
                    .col(big_integer(UserSettings::UserId).not_null())
                    .col(string(UserSettings::Key).not_null())
                    .col(text(UserSettings::Value).not_null())
                    .col(timestamp_with_time_zone(UserSettings::CreatedAt).not_null())
                    .col(timestamp_with_time_zone(UserSettings::UpdatedAt).not_null())
                    .primary_key(Index::create().col(UserSettings::UserId).col(UserSettings::Key))
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_user_settings_user_id")
                            .from(UserSettings::Table, UserSettings::UserId)
                            .to(Users::Table, Users::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_user_settings_key")
                    .table(UserSettings::Table)
                    .col(UserSettings::Key)
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Ok(())
    }
}

#[derive(DeriveIden)]
enum UserSettings {
    Table,
    UserId,
    Key,
    Value,
    CreatedAt,
    UpdatedAt,
}

#[derive(DeriveIden)]
enum Users {
    Table,
    Id,
}
