use sea_orm_migration::{
    prelude::*,
    schema::{big_integer, integer, json_binary, small_integer, string, text, timestamp_with_time_zone},
};

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(Jobs::Table)
                    .if_not_exists()
                    .col(big_integer(Jobs::Id).primary_key().auto_increment())
                    .col(string(Jobs::JobType))
                    .col(json_binary(Jobs::Payload))
                    .col(string(Jobs::Status))
                    .col(small_integer(Jobs::Priority))
                    .col(small_integer(Jobs::Attempt))
                    .col(small_integer(Jobs::MaxAttempts))
                    .col(integer(Jobs::Version))
                    .col(timestamp_with_time_zone(Jobs::ScheduledAt))
                    .col(timestamp_with_time_zone(Jobs::StartedAt).null())
                    .col(timestamp_with_time_zone(Jobs::CompletedAt).null())
                    .col(text(Jobs::ErrorMessage).null())
                    .col(timestamp_with_time_zone(Jobs::CreatedAt))
                    .col(timestamp_with_time_zone(Jobs::UpdatedAt))
                    .to_owned(),
            )
            .await?;

        // Covering index for the claim query: status (equality filter), then
        // priority (consumed by ORDER BY DESC), then scheduled_at (range
        // predicate, ascending), then id for a stable tie-break.
        manager
            .create_index(
                Index::create()
                    .name("idx_jobs_claim")
                    .table(Jobs::Table)
                    .col((Jobs::Status, IndexOrder::Asc))
                    .col((Jobs::Priority, IndexOrder::Desc))
                    .col((Jobs::ScheduledAt, IndexOrder::Asc))
                    .col((Jobs::Id, IndexOrder::Asc))
                    .to_owned(),
            )
            .await?;

        // Index for GC queries that delete old completed/failed jobs.
        manager
            .create_index(
                Index::create()
                    .name("idx_jobs_gc")
                    .table(Jobs::Table)
                    .col((Jobs::Status, IndexOrder::Asc))
                    .col((Jobs::CompletedAt, IndexOrder::Asc))
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
enum Jobs {
    Table,
    Id,
    JobType,
    Payload,
    Status,
    Priority,
    Attempt,
    MaxAttempts,
    Version,
    ScheduledAt,
    StartedAt,
    CompletedAt,
    ErrorMessage,
    CreatedAt,
    UpdatedAt,
}
