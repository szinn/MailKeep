# Database

The project uses SeaORM. The build enables only the SQLite driver, and the app opens
`<metadata_path>/mailkeep.db` (`MAILKEEP__METADATA_PATH`). Write adapters to stay portable to
Postgres and MySQL.

## Environment Variables

The Postgres recipes need a running instance; `just create-database` drops and recreates the
database and role.

- `PGUSER`, `PGPASSWORD`, `PGDATABASE` — role and database made by `just create-database`
- `PGADMINUSER`, `PGADMINPASSWORD` — admin login for `just create-database` and `just database`

## SQLite Notes

When using the SQLite backend, MailKeep automatically configures these PRAGMAs
on every pool connection (see `mk_database::open_database`):

- `journal_mode=WAL` — readers and writers don't block each other
- `busy_timeout=5000` — wait up to 5s for a held lock before failing
- `synchronous=NORMAL` — durable across crashes (the standard pairing with WAL)
- `foreign_keys=true` — enforce FK constraints (off by default in SQLite)

**WAL sidecar files:** WAL mode produces two sidecar files alongside the main
database: `<db>-wal` (write-ahead log) and `<db>-shm` (shared memory index).
When backing up the SQLite database or bind-mounting it into a container,
include both sidecars — copying only the main `.sqlite` file mid-write can
produce a corrupt or out-of-date snapshot.

## SeaORM Adapter Patterns

**No raw SQL.** All queries MUST use SeaORM's portable query builder API (`Entity::find()`, `Entity::insert()`, `Entity::update_many()`, `Condition`, `Query::select()`, etc.). Raw SQL via `Statement::from_string` / `Statement::from_sql_and_values` is forbidden because it is not portable across the supported backends (Postgres, MySQL, SQLite) and silently diverges in behavior. If portable SeaORM doesn't express what you need (e.g. `FOR UPDATE SKIP LOCKED`), solve it at a higher layer — application-level synchronization, optimistic concurrency, or rethinking the design — rather than reaching for backend-specific SQL.

**Migration file naming:** `m<yyyymmdd>_<serial>_<description>.rs` where:

- `<yyyymmdd>` is the date the migration is added (e.g. `20260524`).
- `<serial>` is a **6-digit, monotonically increasing** counter across the entire migration history — NOT reset per day. If the last migration was `m20260520_000003_create_user_settings_table.rs`, the next is `m20260524_000004_*.rs` (continues 000004), not `m20260524_000001_*.rs`.
- `<description>` is a snake_case summary of what the migration does (e.g. `create_jobs_table`).

Register every new migration in `crates/database/src/migrations/mod.rs` in two places: the `mod` declarations and the `Migrator::migrations()` vector. Both must list migrations in serial order.

**Migrations:** Only `up()` migrations need to be implemented. The `down()` method can just
be empty.

**Enum storage:** All domain enums stored as plain `String` columns (no DB CHECK constraints).
Conversion functions are crate-private (e.g. `job_status_to_str` / `str_to_job_status` in `entities/jobs.rs`).
`From<Model> for DomainType` is infallible and panics on unknown values — acceptable since all
writes go through adapters.

**`ActiveModelBehavior` / `before_save`:** Versioned entities (e.g. `messages`, `accounts`) have a
`before_save` hook that auto-increments `version` and sets `updated_at`. When inserting, use
`version: Set(0)` — the hook bumps it to 1. Bulk `update_many` bypasses the hook.

**Optimistic locking pattern:**

```rust
let existing = Entity::find_by_id(id).one(db_tx).await?.ok_or(NotFound)?;
if existing.version != record.version { return Err(VersionConflict); }
// set all mutable fields, then .update()
```

**Subquery filter:** build a `Query::select()` and filter with `Column::in_subquery(..)` — see
the owned-accounts filter in `adapters/message.rs` (`messages::Column::AccountId.in_subquery`).

**Raw inserts in tests:** get the transaction with
`TransactionImpl::get_db_transaction(&*tx).unwrap()` and insert `ActiveModel`s directly.

**Adding a new repository to `RepositoryService`:**

1. Add field + accessor to `core/src/repository.rs` `RepositoryService`
2. Create `database/src/adapters/<name>.rs` with adapter impl + tests
3. Register in `database/src/adapters/mod.rs`
4. Import + wire into builder in `database/src/lib.rs`
5. Add the new `Mock<Name>Repository` to every core service test helper that builds a
   `RepositoryService` (find them with a search for `RepositoryService` under `crates/core/src`)
