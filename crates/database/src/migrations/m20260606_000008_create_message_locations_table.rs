use sea_orm_migration::{
    prelude::*,
    schema::{big_integer, json_binary, string, timestamp_with_time_zone},
};

use super::{m20260606_000006_create_folders_table::Folders, m20260606_000007_create_messages_table::Messages};

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(MessageLocations::Table)
                    .if_not_exists()
                    .col(big_integer(MessageLocations::Id).primary_key())
                    .col(big_integer(MessageLocations::Version))
                    .col(string(MessageLocations::Token).unique_key())
                    .col(big_integer(MessageLocations::MessageId))
                    .col(big_integer(MessageLocations::FolderId))
                    .col(big_integer(MessageLocations::Uid))
                    .col(big_integer(MessageLocations::Uidvalidity))
                    .col(json_binary(MessageLocations::Flags))
                    .col(timestamp_with_time_zone(MessageLocations::InternalDate))
                    .col(timestamp_with_time_zone(MessageLocations::FirstSeenAt))
                    .col(timestamp_with_time_zone(MessageLocations::UpdatedAt))
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_message_locations_message")
                            .from(MessageLocations::Table, MessageLocations::MessageId)
                            .to(Messages::Table, Messages::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_message_locations_folder")
                            .from(MessageLocations::Table, MessageLocations::FolderId)
                            .to(Folders::Table, Folders::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_message_locations_message_folder")
                    .table(MessageLocations::Table)
                    .col(MessageLocations::MessageId)
                    .col(MessageLocations::FolderId)
                    .unique()
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_message_locations_folder_uid")
                    .table(MessageLocations::Table)
                    .col(MessageLocations::FolderId)
                    .col(MessageLocations::Uid)
                    .col(MessageLocations::Uidvalidity)
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
pub(crate) enum MessageLocations {
    Table,
    Id,
    Version,
    Token,
    MessageId,
    FolderId,
    Uid,
    Uidvalidity,
    Flags,
    InternalDate,
    FirstSeenAt,
    UpdatedAt,
}
