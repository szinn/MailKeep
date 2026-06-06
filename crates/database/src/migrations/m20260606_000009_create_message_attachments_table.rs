use sea_orm_migration::{
    prelude::*,
    schema::{big_integer, boolean, string, text, timestamp_with_time_zone},
};

use super::{m20260525_000005_create_accounts_table::Accounts, m20260606_000007_create_messages_table::Messages};

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(MessageAttachments::Table)
                    .if_not_exists()
                    .col(big_integer(MessageAttachments::Id).primary_key())
                    .col(big_integer(MessageAttachments::Version))
                    .col(string(MessageAttachments::Token).unique_key())
                    .col(big_integer(MessageAttachments::MessageId))
                    .col(big_integer(MessageAttachments::AccountId))
                    .col(text(MessageAttachments::ContentHash))
                    .col(text(MessageAttachments::Filename).null())
                    .col(text(MessageAttachments::ContentType))
                    .col(big_integer(MessageAttachments::SizeBytes))
                    .col(boolean(MessageAttachments::IsInline))
                    .col(text(MessageAttachments::ContentId).null())
                    .col(timestamp_with_time_zone(MessageAttachments::CreatedAt))
                    .col(timestamp_with_time_zone(MessageAttachments::UpdatedAt))
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_message_attachments_message")
                            .from(MessageAttachments::Table, MessageAttachments::MessageId)
                            .to(Messages::Table, Messages::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_message_attachments_account")
                            .from(MessageAttachments::Table, MessageAttachments::AccountId)
                            .to(Accounts::Table, Accounts::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_message_attachments_message_id")
                    .table(MessageAttachments::Table)
                    .col(MessageAttachments::MessageId)
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_message_attachments_account_content_hash")
                    .table(MessageAttachments::Table)
                    .col(MessageAttachments::AccountId)
                    .col(MessageAttachments::ContentHash)
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
pub(crate) enum MessageAttachments {
    Table,
    Id,
    Version,
    Token,
    MessageId,
    AccountId,
    ContentHash,
    Filename,
    ContentType,
    SizeBytes,
    IsInline,
    ContentId,
    CreatedAt,
    UpdatedAt,
}
