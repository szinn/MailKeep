use sea_orm_migration::prelude::*;

/// Switch message identity from the RFC822 Message-ID to the raw-content hash.
///
/// A Message-ID is not a reliable identity key: distinct emails legitimately
/// share one (e.g. a Sent copy versus the list/Inbox copy that arrived with
/// extra `Received:`/list headers, resends, or clients that reuse IDs). The old
/// unique index on `(account_id, rfc822_message_id)` forced the archiver to
/// reject the second message as a conflict and drop it. Identity is now the
/// content hash: identical raw bytes dedup; different bytes are different
/// messages even when the Message-ID matches.
#[derive(DeriveMigrationName)]
pub struct Migration;

#[derive(DeriveIden)]
enum Messages {
    Table,
    AccountId,
    ContentHash,
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_index(Index::drop().name("idx_messages_account_message_id").table(Messages::Table).to_owned())
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_messages_account_content_hash")
                    .table(Messages::Table)
                    .col(Messages::AccountId)
                    .col(Messages::ContentHash)
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
