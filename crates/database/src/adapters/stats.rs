use chrono::Utc;
use mk_core::{
    Error,
    repository::Transaction,
    stats::{ArchiveStats, StatsRepository},
    user::UserId,
};
use sea_orm::{
    ColumnTrait, ConnectionTrait, DbBackend, EntityTrait, FromQueryResult, PaginatorTrait, QueryFilter, QuerySelect, QueryTrait,
    prelude::DateTimeWithTimeZone,
    sea_query::{Alias, Expr, ExprTrait, Query},
};

use crate::{
    entities::{accounts, folders, message_attachments, messages, prelude},
    error::handle_dberr,
    transaction::TransactionImpl,
};

pub(crate) struct StatsRepositoryAdapter;

impl StatsRepositoryAdapter {
    pub(crate) fn new() -> Self {
        Self
    }
}

#[derive(FromQueryResult)]
struct MsgAgg {
    message_count: i64,
    storage_bytes: Option<i64>,
    attachment_count: Option<i64>,
}

#[derive(FromQueryResult)]
struct LastSyncAgg {
    last_synced_at: Option<DateTimeWithTimeZone>,
}

#[async_trait::async_trait]
impl StatsRepository for StatsRepositoryAdapter {
    async fn archive_stats(&self, transaction: &dyn Transaction, user_id: UserId) -> Result<ArchiveStats, Error> {
        let db = TransactionImpl::get_db_transaction(transaction)?;
        // SUM widens to NUMERIC/DECIMAL on Postgres/MySQL, so cast it back to a
        // 64-bit integer. Only the type name differs per backend.
        let int_ty = match db.get_database_backend() {
            DbBackend::Postgres => "BIGINT",
            DbBackend::MySql => "SIGNED",
            _ => "INTEGER",
        };
        let owned_accounts = || {
            prelude::Accounts::find()
                .select_only()
                .column(accounts::Column::Id)
                .filter(accounts::Column::UserId.eq(user_id as i64))
                .into_query()
        };

        // Messages: count + raw-blob byte sum + attachment-count sum, scoped to
        // the user's accounts. One message row per (account, content) already,
        // so no DISTINCT needed here.
        let msg = prelude::Messages::find()
            .select_only()
            .column_as(messages::Column::Id.count(), "message_count")
            .column_as(messages::Column::SizeBytes.sum().cast_as(Alias::new(int_ty)), "storage_bytes")
            .column_as(messages::Column::AttachmentCount.sum().cast_as(Alias::new(int_ty)), "attachment_count")
            .filter(messages::Column::AccountId.in_subquery(owned_accounts()))
            .into_model::<MsgAgg>()
            .one(db)
            .await
            .map_err(handle_dberr)?
            .unwrap_or(MsgAgg {
                message_count: 0,
                storage_bytes: None,
                attachment_count: None,
            });

        // Attachment blobs: physical on-disk size dedups by content within an
        // account (the same attachment shared across messages is stored once),
        // so sum over DISTINCT (account_id, content_hash, size_bytes).
        let distinct_attachments = prelude::MessageAttachments::find()
            .select_only()
            .column(message_attachments::Column::AccountId)
            .column(message_attachments::Column::ContentHash)
            .column(message_attachments::Column::SizeBytes)
            .filter(message_attachments::Column::AccountId.in_subquery(owned_accounts()))
            .distinct()
            .into_query();
        let att_query = Query::select()
            .expr_as(
                Expr::col(message_attachments::Column::SizeBytes).sum().cast_as(Alias::new(int_ty)),
                Alias::new("storage_bytes"),
            )
            .from_subquery(distinct_attachments, Alias::new("distinct_attachments"))
            .to_owned();
        let att_bytes: Option<i64> = match db.query_one(&att_query).await.map_err(handle_dberr)? {
            Some(row) => row.try_get("", "storage_bytes").map_err(handle_dberr)?,
            None => None,
        };

        let account_count = prelude::Accounts::find()
            .filter(accounts::Column::UserId.eq(user_id as i64))
            .count(db)
            .await
            .map_err(handle_dberr)?;

        let last = prelude::Folders::find()
            .select_only()
            .column_as(folders::Column::LastSyncedAt.max(), "last_synced_at")
            .filter(folders::Column::AccountId.in_subquery(owned_accounts()))
            .into_model::<LastSyncAgg>()
            .one(db)
            .await
            .map_err(handle_dberr)?
            .unwrap_or(LastSyncAgg { last_synced_at: None });

        Ok(ArchiveStats {
            message_count: msg.message_count as u64,
            attachment_count: msg.attachment_count.unwrap_or(0) as u64,
            storage_bytes: (msg.storage_bytes.unwrap_or(0) + att_bytes.unwrap_or(0)) as u64,
            account_count,
            last_synced_at: last.last_synced_at.map(|d| d.with_timezone(&Utc)),
        })
    }
}

#[cfg(test)]
mod tests {
    use chrono::{TimeZone, Utc};
    use mk_core::{repository::Repository, stats::StatsRepository};
    use sea_orm::{ActiveModelTrait, ActiveValue::Set, Database, DatabaseConnection};
    use sea_orm_migration::MigratorTrait;

    use super::StatsRepositoryAdapter;
    use crate::{
        entities::{accounts, folders, message_attachments, messages, users},
        migrations::Migrator,
        repository::RepositoryImpl,
    };

    async fn setup() -> DatabaseConnection {
        let db = Database::connect("sqlite::memory:").await.unwrap();
        Migrator::up(&db, None).await.unwrap();
        db
    }

    fn ts(secs: i64) -> chrono::DateTime<Utc> {
        Utc.timestamp_opt(secs, 0).unwrap()
    }

    async fn seed_user(db: &DatabaseConnection, id: i64) {
        users::ActiveModel {
            id: Set(id),
            version: Set(1),
            token: Set(format!("USR_{id}")),
            username: Set(format!("user{id}")),
            full_name: Set(format!("User {id}")),
            password_hash: Set("hash".into()),
            email_address: Set(format!("user{id}@example.com")),
            capabilities: Set("[]".into()),
            change_password_on_login: Set(false),
            created_at: Set(ts(1).into()),
            updated_at: Set(ts(1).into()),
        }
        .insert(db)
        .await
        .unwrap();
    }

    async fn seed_account(db: &DatabaseConnection, id: i64, user_id: i64) {
        accounts::ActiveModel {
            id: Set(id),
            version: Set(1),
            token: Set(format!("ACC_{id}")),
            user_id: Set(user_id),
            display_name: Set(format!("Account {id}")),
            email_address: Set(format!("acct{id}@example.com")),
            server: Set(r#"{"host":"h","port":993,"tls":"Tls"}"#.into()),
            username: Set(format!("acct{id}")),
            credentials: Set(vec![0u8; 28]),
            enabled: Set(true),
            status: Set("Idle".into()),
            last_error: Set(None),
            last_synced_at: Set(None),
            created_at: Set(ts(1).into()),
            updated_at: Set(ts(1).into()),
        }
        .insert(db)
        .await
        .unwrap();
    }

    #[allow(clippy::too_many_arguments)]
    async fn seed_message(db: &DatabaseConnection, id: i64, account_id: i64, content_hash: &str, size_bytes: i64, attachment_count: i32) {
        messages::ActiveModel {
            id: Set(id),
            version: Set(1),
            token: Set(format!("MSG_{id}")),
            account_id: Set(account_id),
            rfc822_message_id: Set(format!("<{id}@x>")),
            content_hash: Set(content_hash.into()),
            subject: Set(Some("s".into())),
            from_address: Set("from@x.com".into()),
            from_name: Set(None),
            to_addresses: Set(serde_json::json!([])),
            cc_addresses: Set(serde_json::json!([])),
            bcc_addresses: Set(serde_json::json!([])),
            reply_to_addresses: Set(serde_json::json!([])),
            sent_date: Set(None),
            in_reply_to: Set(None),
            references: Set(serde_json::json!([])),
            snippet: Set(String::new()),
            size_bytes: Set(size_bytes),
            has_attachments: Set(attachment_count > 0),
            attachment_count: Set(attachment_count),
            indexed: Set(false),
            created_at: Set(ts(1).into()),
            updated_at: Set(ts(1).into()),
        }
        .insert(db)
        .await
        .unwrap();
    }

    async fn seed_attachment(db: &DatabaseConnection, id: i64, message_id: i64, account_id: i64, content_hash: &str, size_bytes: i64) {
        message_attachments::ActiveModel {
            id: Set(id),
            version: Set(1),
            token: Set(format!("ATT_{id}")),
            message_id: Set(message_id),
            account_id: Set(account_id),
            content_hash: Set(content_hash.into()),
            filename: Set(Some("a.bin".into())),
            content_type: Set("application/octet-stream".into()),
            size_bytes: Set(size_bytes),
            is_inline: Set(false),
            content_id: Set(None),
            created_at: Set(ts(1).into()),
            updated_at: Set(ts(1).into()),
        }
        .insert(db)
        .await
        .unwrap();
    }

    async fn seed_folder(db: &DatabaseConnection, id: i64, account_id: i64, last_synced: Option<i64>) {
        folders::ActiveModel {
            id: Set(id),
            version: Set(1),
            token: Set(format!("FLD_{id}")),
            account_id: Set(account_id),
            path: Set("INBOX".into()),
            display_name: Set(None),
            special_use: Set(None),
            enabled: Set(true),
            idle_enabled: Set(false),
            uidvalidity: Set(Some(1)),
            last_uid: Set(0),
            last_synced_at: Set(last_synced.map(|s| ts(s).into())),
            last_error: Set(None),
            created_at: Set(ts(1).into()),
            updated_at: Set(ts(1).into()),
        }
        .insert(db)
        .await
        .unwrap();
    }

    /// Drive the adapter through a read-only transaction over `db`.
    async fn run_stats(db: &DatabaseConnection, user_id: u64) -> mk_core::stats::ArchiveStats {
        let repo = RepositoryImpl::new(db.clone());
        let tx = repo.begin_read_only().await.unwrap();
        let out = StatsRepositoryAdapter::new().archive_stats(&*tx, user_id).await.unwrap();
        tx.rollback().await.unwrap();
        out
    }

    #[tokio::test]
    async fn aggregates_are_scoped_per_user() {
        let db = setup().await;
        // User 10: two accounts, three messages, folders synced.
        seed_user(&db, 10).await;
        seed_account(&db, 1, 10).await;
        seed_account(&db, 2, 10).await;
        seed_message(&db, 1001, 1, "m1", 1000, 1).await;
        seed_message(&db, 1002, 1, "m2", 2000, 0).await;
        seed_message(&db, 1003, 2, "m3", 500, 2).await;
        seed_attachment(&db, 2001, 1001, 1, "att_x", 300).await;
        seed_attachment(&db, 2002, 1003, 2, "att_y", 400).await;
        seed_attachment(&db, 2003, 1003, 2, "att_z", 100).await;
        seed_folder(&db, 3001, 1, Some(1_700_000_000)).await;
        seed_folder(&db, 3002, 2, Some(1_700_000_500)).await;

        // User 20: separate data that must NOT leak into user 10's totals.
        seed_user(&db, 20).await;
        seed_account(&db, 9, 20).await;
        seed_message(&db, 9001, 9, "z1", 9999, 5).await;
        seed_attachment(&db, 9101, 9001, 9, "zatt", 8888).await;
        seed_folder(&db, 9201, 9, Some(1_800_000_000)).await;

        let s = run_stats(&db, 10).await;
        assert_eq!(s.account_count, 2);
        assert_eq!(s.message_count, 3);
        assert_eq!(s.attachment_count, 3, "sum of messages.attachment_count (1 + 0 + 2)");
        // storage = messages(1000+2000+500) + attachments(300+400+100) = 4300
        assert_eq!(s.storage_bytes, 4300);
        assert_eq!(s.last_synced_at, Some(ts(1_700_000_500)), "MAX across folders");
    }

    #[tokio::test]
    async fn attachment_dedup_not_double_counted() {
        let db = setup().await;
        seed_user(&db, 10).await;
        seed_account(&db, 1, 10).await;
        seed_message(&db, 1001, 1, "m1", 100, 1).await;
        seed_message(&db, 1002, 1, "m2", 100, 1).await;
        // Same attachment content shared by both messages (same account +
        // content_hash + size): physically one blob → counted ONCE.
        seed_attachment(&db, 2001, 1001, 1, "shared", 500).await;
        seed_attachment(&db, 2002, 1002, 1, "shared", 500).await;

        let s = run_stats(&db, 10).await;
        // messages 100+100 = 200; attachments DISTINCT = 500 (not 1000).
        assert_eq!(s.storage_bytes, 700);
    }

    #[tokio::test]
    async fn empty_archive_is_all_zero() {
        let db = setup().await;
        seed_user(&db, 10).await; // user exists but has no accounts/messages
        let s = run_stats(&db, 10).await;
        assert_eq!(s.account_count, 0);
        assert_eq!(s.message_count, 0);
        assert_eq!(s.attachment_count, 0);
        assert_eq!(s.storage_bytes, 0);
        assert_eq!(s.last_synced_at, None);
    }

    #[tokio::test]
    async fn last_synced_none_when_never_synced() {
        let db = setup().await;
        seed_user(&db, 10).await;
        seed_account(&db, 1, 10).await;
        seed_folder(&db, 3001, 1, None).await; // folder exists, never synced
        let s = run_stats(&db, 10).await;
        assert_eq!(s.last_synced_at, None);
    }
}
