use chrono::Utc;
use mk_core::{
    Error,
    repository::Transaction,
    stats::{ArchiveStats, StatsRepository},
    user::UserId,
};
use sea_orm::{ConnectionTrait, DbBackend, FromQueryResult, Statement, Value, prelude::DateTimeWithTimeZone};

use crate::{error::handle_dberr, transaction::TransactionImpl};

pub(crate) struct StatsRepositoryAdapter;

impl StatsRepositoryAdapter {
    pub(crate) fn new() -> Self {
        Self
    }
}

#[derive(FromQueryResult)]
struct MsgAgg {
    message_count: i64,
    storage_bytes: i64,
    attachment_count: i64,
}

#[derive(FromQueryResult)]
struct SumAgg {
    storage_bytes: i64,
}

#[derive(FromQueryResult)]
struct CountAgg {
    account_count: i64,
}

#[derive(FromQueryResult)]
struct LastSyncAgg {
    last_synced_at: Option<DateTimeWithTimeZone>,
}

#[async_trait::async_trait]
impl StatsRepository for StatsRepositoryAdapter {
    async fn archive_stats(&self, transaction: &dyn Transaction, user_id: UserId) -> Result<ArchiveStats, Error> {
        let db = TransactionImpl::get_db_transaction(transaction)?;
        let backend = db.get_database_backend();
        // Per-backend placeholder token and integer CAST type.
        let (ph, int_ty) = match backend {
            DbBackend::Postgres => ("$1", "BIGINT"),
            DbBackend::MySql => ("?", "SIGNED"),
            // Sqlite and any future backend: `?` placeholder, INTEGER cast.
            _ => ("?", "INTEGER"),
        };
        let uid = Value::BigInt(Some(user_id as i64));

        // Messages: count + raw-blob byte sum + attachment-count sum, scoped to
        // the user's accounts. One message row per (account, content) already,
        // so no DISTINCT needed here.
        let msg_sql = format!(
            "SELECT COUNT(*) AS message_count, CAST(COALESCE(SUM(size_bytes), 0) AS {int_ty}) AS storage_bytes, CAST(COALESCE(SUM(attachment_count), 0) AS \
             {int_ty}) AS attachment_count FROM messages WHERE account_id IN (SELECT id FROM accounts WHERE user_id = {ph})"
        );
        let msg = MsgAgg::find_by_statement(Statement::from_sql_and_values(backend, msg_sql, [uid.clone()]))
            .one(db)
            .await
            .map_err(handle_dberr)?
            .unwrap_or(MsgAgg {
                message_count: 0,
                storage_bytes: 0,
                attachment_count: 0,
            });

        // Attachment blobs: physical on-disk size dedups by content within an
        // account (the same attachment shared across messages is stored once),
        // so sum over DISTINCT (account_id, content_hash, size_bytes).
        let att_sql = format!(
            "SELECT CAST(COALESCE(SUM(size_bytes), 0) AS {int_ty}) AS storage_bytes FROM (SELECT DISTINCT account_id, content_hash, size_bytes FROM \
             message_attachments WHERE account_id IN (SELECT id FROM accounts WHERE user_id = {ph})) AS distinct_attachments"
        );
        let att = SumAgg::find_by_statement(Statement::from_sql_and_values(backend, att_sql, [uid.clone()]))
            .one(db)
            .await
            .map_err(handle_dberr)?
            .unwrap_or(SumAgg { storage_bytes: 0 });

        let acc_sql = format!("SELECT COUNT(*) AS account_count FROM accounts WHERE user_id = {ph}");
        let acc = CountAgg::find_by_statement(Statement::from_sql_and_values(backend, acc_sql, [uid.clone()]))
            .one(db)
            .await
            .map_err(handle_dberr)?
            .unwrap_or(CountAgg { account_count: 0 });

        let ls_sql = format!("SELECT MAX(last_synced_at) AS last_synced_at FROM folders WHERE account_id IN (SELECT id FROM accounts WHERE user_id = {ph})");
        let last = LastSyncAgg::find_by_statement(Statement::from_sql_and_values(backend, ls_sql, [uid.clone()]))
            .one(db)
            .await
            .map_err(handle_dberr)?
            .unwrap_or(LastSyncAgg { last_synced_at: None });

        Ok(ArchiveStats {
            message_count: msg.message_count as u64,
            attachment_count: msg.attachment_count as u64,
            storage_bytes: (msg.storage_bytes + att.storage_bytes) as u64,
            account_count: acc.account_count as u64,
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
