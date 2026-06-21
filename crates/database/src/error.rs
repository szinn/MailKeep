use mk_core::RepositoryError;
use sea_orm::{DbErr, RuntimeErr};

/// `PostgreSQL` error codes.
/// See: <https://www.postgresql.org/docs/current/errcodes-appendix.html>
mod pg_error_codes {
    /// 25006: `read_only_sql_transaction`
    /// Raised when a write operation is attempted on a read-only transaction.
    pub const READ_ONLY_SQL_TRANSACTION: &str = "25006";
    /// 23505: `unique_violation`
    /// Raised when a unique constraint is violated.
    pub const UNIQUE_VIOLATION: &str = "23505";
    /// 23503: `foreign_key_violation`
    /// Raised when a foreign key constraint is violated.
    pub const FOREIGN_KEY_VIOLATION: &str = "23503";
    /// 40001: `serialization_failure`
    /// Raised when a transaction cannot be serialized.
    pub const SERIALIZATION_FAILURE: &str = "40001";
    /// 57014: `query_canceled`
    /// Raised when a query is canceled.
    pub const QUERY_CANCELED: &str = "57014";
}

/// SQLite primary result codes (surfaced by sqlx as the bare integer string).
/// See: <https://www.sqlite.org/rescode.html>
mod sqlite_busy {
    /// 5: `SQLITE_BUSY` — the database file is locked by another connection.
    pub const BUSY: &str = "5";
    /// 6: `SQLITE_LOCKED` — a table in the database is locked.
    pub const LOCKED: &str = "6";
}

#[allow(clippy::needless_pass_by_value, reason = "Required for map_err")]
pub fn handle_dberr(error: DbErr) -> RepositoryError {
    // Connectivity errors: network/DNS failure, pool exhaustion, closed pool.
    // Checked before sql_err() because these need special transient handling.
    if let DbErr::Conn(RuntimeErr::SqlxError(ref sqlx_err)) = error
        && matches!(**sqlx_err, sqlx::Error::Io(_) | sqlx::Error::PoolTimedOut | sqlx::Error::PoolClosed)
    {
        return RepositoryError::Connection(error.to_string());
    }
    // Pool acquire failure (e.g. pool exhausted before timeout).
    if let DbErr::ConnectionAcquire(_) = &error {
        return RepositoryError::Connection(error.to_string());
    }

    // Check sql_err first — it is database-agnostic and handles common constraint
    // violations uniformly across Postgres, MySQL, and SQLite.
    if let Some(sql_err) = error.sql_err() {
        return match sql_err {
            sea_orm::SqlErr::UniqueConstraintViolation(msg) => RepositoryError::Constraint(msg),
            sea_orm::SqlErr::ForeignKeyConstraintViolation(msg) => RepositoryError::Constraint(format!("Foreign key violation: {msg}")),
            _ => {
                tracing::error!(error = ?error, "Unhandled sql_err");
                RepositoryError::Database(error.to_string())
            }
        };
    }

    // Fall back to database-specific error codes for errors not covered by sql_err
    // (read-only transactions, serialization failures, query cancellation, etc.).
    if let DbErr::Query(RuntimeErr::SqlxError(sqlx_err)) | DbErr::Exec(RuntimeErr::SqlxError(sqlx_err)) = &error
        && let Some(db_err) = sqlx_err.as_database_error()
        && let Some(code) = db_err.code()
    {
        return match code.as_ref() {
            pg_error_codes::READ_ONLY_SQL_TRANSACTION => RepositoryError::ReadOnly,
            pg_error_codes::UNIQUE_VIOLATION => RepositoryError::Constraint(db_err.message().to_string()),
            pg_error_codes::FOREIGN_KEY_VIOLATION => RepositoryError::Constraint(format!("Foreign key violation: {}", db_err.message())),
            pg_error_codes::SERIALIZATION_FAILURE => RepositoryError::Conflict,
            pg_error_codes::QUERY_CANCELED => {
                tracing::warn!(error = %error, "Query canceled");
                RepositoryError::QueryCanceled
            }
            // SQLite contention: SQLITE_BUSY (5) and SQLITE_LOCKED (6). The lock
            // clears once the holding transaction commits, so these are transient
            // and must be retried — not treated as a permanent failure. busy_timeout
            // covers simple write waits, but read→write snapshot conflicts
            // (BUSY_SNAPSHOT) still surface here. These bare numeric codes are
            // SQLite-only; Postgres/MySQL use SQLSTATE strings, so there is no
            // collision with the codes matched above.
            sqlite_busy::BUSY | sqlite_busy::LOCKED => {
                tracing::warn!(error_code = %code, error = %error, "Database busy/locked — transient, will retry");
                RepositoryError::Busy(error.to_string())
            }
            _ => {
                tracing::error!(error_code = %code, error = %error, "Unhandled database error code");
                RepositoryError::Database(error.to_string())
            }
        };
    }

    tracing::error!(error = ?error, "Unhandled database error");
    RepositoryError::Database(error.to_string())
}
