use sea_orm_migration::{prelude::*, schema::boolean};

/// Add the `indexed` watermark column to `messages`.
///
/// The FTS indexer drains rows where `indexed = false` in ascending id order,
/// builds the search index for them, then flips the flag. Ascending id is a
/// stable, deterministic drain order with no starvation — NOT temporal, since
/// message ids are random token-derived values. Existing rows default to
/// `false` so they get picked up on the next drain.
#[derive(DeriveMigrationName)]
pub struct Migration;

#[derive(DeriveIden)]
enum Messages {
    Table,
    Indexed,
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(Messages::Table)
                    .add_column(boolean(Messages::Indexed).default(false))
                    .to_owned(),
            )
            .await?;

        // Supports the `WHERE indexed = false` drain query. A partial index
        // (`WHERE indexed = false`) would be tighter on Postgres/SQLite, but
        // SeaQuery's index builder has no portable WHERE-clause support, so we
        // use a plain index that works uniformly across Postgres/MySQL/SQLite.
        manager
            .create_index(
                Index::create()
                    .name("idx_messages_indexed")
                    .table(Messages::Table)
                    .col(Messages::Indexed)
                    .to_owned(),
            )
            .await?;

        Ok(())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Ok(())
    }
}
