use sea_orm_migration::{
    prelude::*,
    schema::{big_integer, boolean, integer, json_binary, string, text, timestamp_with_time_zone},
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
                    .table(Messages::Table)
                    .if_not_exists()
                    .col(big_integer(Messages::Id).primary_key())
                    .col(big_integer(Messages::Version))
                    .col(string(Messages::Token).unique_key())
                    .col(big_integer(Messages::AccountId))
                    .col(text(Messages::Rfc822MessageId))
                    .col(text(Messages::ContentHash))
                    .col(text(Messages::Subject).null())
                    .col(text(Messages::FromAddress))
                    .col(text(Messages::FromName).null())
                    .col(json_binary(Messages::ToAddresses))
                    .col(json_binary(Messages::CcAddresses))
                    .col(json_binary(Messages::BccAddresses))
                    .col(json_binary(Messages::ReplyToAddresses))
                    .col(timestamp_with_time_zone(Messages::SentDate).null())
                    .col(text(Messages::InReplyTo).null())
                    .col(json_binary(Messages::References))
                    .col(text(Messages::Snippet))
                    .col(big_integer(Messages::SizeBytes))
                    .col(boolean(Messages::HasAttachments))
                    .col(integer(Messages::AttachmentCount))
                    .col(timestamp_with_time_zone(Messages::CreatedAt))
                    .col(timestamp_with_time_zone(Messages::UpdatedAt))
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_messages_account")
                            .from(Messages::Table, Messages::AccountId)
                            .to(Accounts::Table, Accounts::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_messages_account_message_id")
                    .table(Messages::Table)
                    .col(Messages::AccountId)
                    .col(Messages::Rfc822MessageId)
                    .unique()
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_messages_account_sent_date_desc")
                    .table(Messages::Table)
                    .col(Messages::AccountId)
                    .col((Messages::SentDate, IndexOrder::Desc))
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_messages_account_from_address")
                    .table(Messages::Table)
                    .col(Messages::AccountId)
                    .col(Messages::FromAddress)
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
pub(crate) enum Messages {
    Table,
    Id,
    Version,
    Token,
    AccountId,
    Rfc822MessageId,
    ContentHash,
    Subject,
    FromAddress,
    FromName,
    ToAddresses,
    CcAddresses,
    BccAddresses,
    ReplyToAddresses,
    SentDate,
    InReplyTo,
    References,
    Snippet,
    SizeBytes,
    HasAttachments,
    AttachmentCount,
    CreatedAt,
    UpdatedAt,
}
